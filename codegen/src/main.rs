//! Writes `src/generated.rs` from `spec/enablebanking-api.yaml`.
//!
//! Run `cargo run -p codegen` from the workspace root after updating the spec, and `cargo run -p codegen -- --check` to fail when the checked-in client is stale.

use std::path::Path;
use std::process::ExitCode;

use progenitor_impl::{
    GenerationSettings, Generator, HooksMode, InterfaceStyle, TagStyle, TypeImpl,
};
use quote::quote;
use schemars::schema::{InstanceType, SchemaObject};
use serde_json::{Map, Value};

/// Method names for every operation, keyed by `METHOD /path`. An operation missing here stops generation, so each new endpoint gets a name chosen by a person.
const OPERATIONS: &[(&str, &str)] = &[
    ("POST /auth", "start_authorization"),
    ("POST /sessions", "authorize_session"),
    ("GET /sessions/{session_id}", "get_session"),
    ("DELETE /sessions/{session_id}", "delete_session"),
    ("GET /accounts/{account_id}/details", "get_account_details"),
    (
        "GET /accounts/{account_id}/balances",
        "get_account_balances",
    ),
    (
        "GET /accounts/{account_id}/transactions",
        "get_account_transactions",
    ),
    (
        "GET /accounts/{account_id}/transactions/{transaction_id}",
        "get_transaction",
    ),
    ("POST /payments", "create_payment"),
    ("GET /payments/{payment_id}", "get_payment"),
    ("DELETE /payments/{payment_id}", "delete_payment"),
    ("POST /payments/{payment_id}/submit", "submit_payment"),
    (
        "GET /payments/{payment_id}/transactions/{transaction_id}",
        "get_payment_transaction",
    ),
    ("GET /aspsps", "get_aspsps"),
    ("GET /application", "get_application"),
];

/// JSON Schema 2020-12 keywords that OpenAPI 3.0 cannot express and this lowering does not translate.
const UNSUPPORTED: &[&str] = &[
    "$defs",
    "$dynamicRef",
    "$dynamicAnchor",
    "prefixItems",
    "unevaluatedItems",
    "unevaluatedProperties",
    "dependentSchemas",
    "dependentRequired",
    "if",
    "then",
    "else",
    "contentSchema",
];

const HTTP_METHODS: &[&str] = &[
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

fn main() -> ExitCode {
    let check = match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [] => false,
        [flag] if flag == "--check" => true,
        _ => {
            eprintln!("usage: codegen [--check]");
            return ExitCode::from(2);
        }
    };
    match run(Path::new("."), check) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("codegen: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(root: &Path, check: bool) -> Result<(), String> {
    let spec_path = root.join("spec/enablebanking-api.yaml");
    let output_path = root.join("src/generated.rs");
    let text = std::fs::read_to_string(&spec_path)
        .map_err(|error| format!("reading {}: {error}", spec_path.display()))?;
    let generated = generate(&text)?;
    if check {
        let current = std::fs::read_to_string(&output_path).unwrap_or_default();
        if current != generated {
            return Err(format!(
                "{} is stale, run `cargo run -p codegen`",
                output_path.display()
            ));
        }
        return Ok(());
    }
    std::fs::write(&output_path, generated)
        .map_err(|error| format!("writing {}: {error}", output_path.display()))
}

/// The Rust source of the client for the OpenAPI 3.1 description in `yaml`.
fn generate(yaml: &str) -> Result<String, String> {
    let mut spec: Value =
        serde_norway::from_str(yaml).map_err(|error| format!("parsing the spec: {error}"))?;
    lower(&mut spec)?;
    name_operations(&mut spec)?;
    let version = spec
        .pointer("/info/version")
        .and_then(Value::as_str)
        .ok_or("the spec has no info.version")?
        .to_owned();
    let openapi: openapiv3::OpenAPI = serde_json::from_value(spec)
        .map_err(|error| format!("reading the lowered spec: {error}"))?;

    let mut settings = GenerationSettings::new();
    settings
        .with_interface(InterfaceStyle::Builder)
        .with_tag(TagStyle::Merged)
        .with_inner_type(quote!(crate::auth::Signer))
        .with_hooks(HooksMode::Expected)
        .with_derive("PartialEq")
        .with_conversion(
            string_format("date"),
            "::jiff::civil::Date",
            [TypeImpl::Display, TypeImpl::FromStr].into_iter(),
        )
        .with_conversion(
            string_format("date-time"),
            "::jiff::Timestamp",
            [TypeImpl::Display, TypeImpl::FromStr].into_iter(),
        )
        .with_replacement(
            "AmountType",
            "crate::Amount",
            std::iter::once(TypeImpl::Display),
        );
    let tokens = Generator::new(&settings)
        .generate_tokens(&openapi)
        .map_err(|error| format!("generating the client: {error}"))?;
    let file: syn::File =
        syn::parse2(tokens).map_err(|error| format!("parsing the generated tokens: {error}"))?;
    // progenitor's usage sketches name undeclared variables, so they are shown as text instead of counted as ignored doctests.
    let source = prettyplease::unparse(&file).replace("```ignore", "```text");
    Ok(format!(
        "// Generated by `cargo run -p codegen` from spec/enablebanking-api.yaml, API version {version}. Do not edit.\n\n{source}"
    ))
}

fn string_format(format: &str) -> SchemaObject {
    SchemaObject {
        instance_type: Some(InstanceType::String.into()),
        format: Some(format.to_owned()),
        ..SchemaObject::default()
    }
}

/// Rewrites the OpenAPI 3.1 description into the 3.0 dialect that progenitor reads.
fn lower(spec: &mut Value) -> Result<(), String> {
    let root = spec.as_object_mut().ok_or("the spec is not an object")?;
    match root.get("openapi").and_then(Value::as_str) {
        Some(version) if version.starts_with("3.1.") || version.starts_with("3.0.") => {}
        other => return Err(format!("unsupported OpenAPI version {other:?}")),
    }
    for key in ["webhooks", "jsonSchemaDialect"] {
        if root.contains_key(key) {
            return Err(format!("the spec uses `{key}`, which OpenAPI 3.0 lacks"));
        }
    }
    root.insert("openapi".to_owned(), Value::from("3.0.3"));
    lower_value(spec, "")
}

fn lower_value(value: &mut Value, path: &str) -> Result<(), String> {
    match value {
        Value::Object(object) => {
            lower_object(object, path)?;
            for (key, child) in object.iter_mut() {
                lower_value(child, &format!("{path}/{key}"))?;
            }
            Ok(())
        }
        Value::Array(items) => {
            for (index, child) in items.iter_mut().enumerate() {
                lower_value(child, &format!("{path}/{index}"))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Lowers the 3.1 keywords of one object. Schema `examples` are arrays, while parameter and media type `examples` are maps of Example Objects and stay.
fn lower_object(object: &mut Map<String, Value>, path: &str) -> Result<(), String> {
    if let Some(keyword) = UNSUPPORTED
        .iter()
        .find(|keyword| object.contains_key(**keyword))
    {
        return Err(format!("{path} uses `{keyword}`, which OpenAPI 3.0 lacks"));
    }
    if let Some(Value::Array(examples)) = object.get("examples") {
        let first = examples.first().cloned();
        object.remove("examples");
        if let Some(first) = first {
            object.entry("example").or_insert(first);
        }
    }
    if let Some(constant) = object.remove("const") {
        object.insert("enum".to_owned(), Value::Array(vec![constant]));
    }
    if let Some(Value::Array(types)) = object.get("type") {
        let (nulls, others): (Vec<&Value>, Vec<&Value>) =
            types.iter().partition(|kind| kind.as_str() == Some("null"));
        let [single] = others.as_slice() else {
            return Err(format!("{path} has the type list {types:?}"));
        };
        let single = (*single).clone();
        let nullable = !nulls.is_empty();
        object.insert("type".to_owned(), single);
        if nullable {
            object.insert("nullable".to_owned(), Value::Bool(true));
        }
    }
    for keyword in ["anyOf", "oneOf"] {
        if let Some(Value::Array(variants)) = object.get_mut(keyword) {
            let before = variants.len();
            variants.retain(|variant| variant.get("type").and_then(Value::as_str) != Some("null"));
            if variants.len() < before {
                object.insert("nullable".to_owned(), Value::Bool(true));
            }
        }
    }
    Ok(())
}

/// Replaces every machine-made `operationId` with the name in [`OPERATIONS`].
fn name_operations(spec: &mut Value) -> Result<(), String> {
    let paths = spec
        .get_mut("paths")
        .and_then(Value::as_object_mut)
        .ok_or("the spec has no paths")?;
    let mut named = 0;
    for (path, item) in paths.iter_mut() {
        let Some(item) = item.as_object_mut() else {
            continue;
        };
        for (method, operation) in item.iter_mut() {
            if !HTTP_METHODS.contains(&method.as_str()) {
                continue;
            }
            let key = format!("{} {path}", method.to_uppercase());
            let name = OPERATIONS
                .iter()
                .find(|(operation_key, _)| *operation_key == key)
                .map(|(_, name)| *name)
                .ok_or_else(|| format!("`{key}` has no method name in codegen OPERATIONS"))?;
            operation
                .as_object_mut()
                .ok_or_else(|| format!("`{key}` is not an object"))?
                .insert("operationId".to_owned(), Value::from(name));
            named += 1;
        }
    }
    if named != OPERATIONS.len() {
        return Err(format!(
            "the spec has {named} operations but codegen names {}; remove the names of deleted operations",
            OPERATIONS.len()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{OPERATIONS, lower, name_operations};

    #[test]
    fn nullable_type_lists_constants_and_schema_examples_become_openapi_3_0() {
        let mut spec = json!({
            "openapi": "3.1.0",
            "components": {"schemas": {"Note": {"type": "object", "properties": {
                "text": {"type": ["string", "null"], "examples": ["hello"]},
                "kind": {"const": "note"},
                "parent": {"anyOf": [{"$ref": "#/components/schemas/Note"}, {"type": "null"}]},
                "either": {"anyOf": [{"type": "string"}, {"type": "integer"}]},
                "tags": {"allOf": [{"type": ["array", "null"], "items": {"type": "string"}}]},
            }}}},
            "paths": {"/notes": {"get": {"parameters": [{"name": "q", "in": "query", "examples": {"one": {"value": "a"}}}]}}},
        });
        lower(&mut spec).unwrap();
        assert_eq!(spec["openapi"], "3.0.3");
        let properties = &spec["components"]["schemas"]["Note"]["properties"];
        assert_eq!(
            properties["text"],
            json!({"type": "string", "nullable": true, "example": "hello"})
        );
        assert_eq!(properties["kind"], json!({"enum": ["note"]}));
        assert_eq!(
            properties["parent"],
            json!({"anyOf": [{"$ref": "#/components/schemas/Note"}], "nullable": true})
        );
        assert_eq!(
            properties["either"],
            json!({"anyOf": [{"type": "string"}, {"type": "integer"}]})
        );
        assert_eq!(
            properties["tags"]["allOf"][0],
            json!({"type": "array", "nullable": true, "items": {"type": "string"}})
        );
        assert_eq!(
            spec["paths"]["/notes"]["get"]["parameters"][0]["examples"],
            json!({"one": {"value": "a"}})
        );
    }

    #[test]
    fn keywords_without_an_openapi_3_0_form_stop_generation() {
        let mut spec =
            json!({"openapi": "3.1.0", "components": {"schemas": {"Pair": {"prefixItems": []}}}});
        let error = lower(&mut spec).unwrap_err();
        assert!(
            error.contains("/components/schemas/Pair") && error.contains("prefixItems"),
            "{error}"
        );
        let mut multi = json!({"openapi": "3.1.0", "components": {"schemas": {"Id": {"type": ["string", "integer"]}}}});
        assert!(lower(&mut multi).is_err());
    }

    #[test]
    fn an_operation_without_a_chosen_name_stops_generation() {
        let mut spec = json!({"paths": {"/new": {"get": {"operationId": "new_new_get"}}}});
        let error = name_operations(&mut spec).unwrap_err();
        assert!(error.contains("GET /new"), "{error}");
    }

    #[test]
    fn a_name_left_for_a_removed_operation_stops_generation() {
        let mut spec = json!({"paths": {"/application": {"get": {"operationId": "x"}}}});
        let error = name_operations(&mut spec).unwrap_err();
        assert!(error.contains("remove the names"), "{error}");
    }

    #[test]
    fn every_listed_operation_gets_its_name() {
        let mut paths = serde_json::Map::new();
        for (key, _) in OPERATIONS {
            let (method, path) = key.split_once(' ').unwrap();
            let item = paths.entry(path.to_owned()).or_insert_with(|| json!({}));
            item[method.to_lowercase()] = json!({"operationId": "machine_made"});
        }
        let mut spec = json!({"paths": paths});
        name_operations(&mut spec).unwrap();
        assert_eq!(spec["paths"]["/aspsps"]["get"]["operationId"], "get_aspsps");
        assert_eq!(
            spec["paths"]["/accounts/{account_id}/transactions"]["get"]["operationId"],
            "get_account_transactions"
        );
    }

    #[test]
    fn descriptions_other_than_openapi_3_are_refused() {
        for version in [json!("2.0"), json!("4.0.0"), json!(null)] {
            let mut spec = json!({"openapi": version, "paths": {}});
            let error = lower(&mut spec).unwrap_err();
            assert!(error.contains("unsupported OpenAPI version"), "{error}");
        }
    }
}
