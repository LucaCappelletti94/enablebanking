//! Writes `src/generated.rs` from `spec/enablebanking-api.yaml`.
//!
//! Run `cargo run -p codegen` from the workspace root after updating the spec, and `cargo run -p codegen -- --check` to fail when the checked-in client is stale.

use std::path::Path;
use std::process::ExitCode;

use heck::ToUpperCamelCase;
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

/// The keys of a Path Item Object that hold operations.
const HTTP_METHODS: &[&str] = &[
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// What one run of the generator does.
#[derive(Clone, Copy)]
enum Mode {
    /// Writes the client.
    Write,
    /// Fails when the checked-in client differs from a fresh one.
    Check,
    /// Prints the version of the vendored description.
    SpecVersion,
}

fn main() -> ExitCode {
    let mode = match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [] => Mode::Write,
        [flag] if flag == "--check" => Mode::Check,
        [flag] if flag == "--spec-version" => Mode::SpecVersion,
        _ => {
            eprintln!("usage: codegen [--check | --spec-version]");
            return ExitCode::from(2);
        }
    };
    match run(Path::new("."), mode) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("codegen: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Runs the generator in `mode` on the workspace at `root`.
fn run(root: &Path, mode: Mode) -> Result<(), String> {
    let spec_path = root.join("spec/enablebanking-api.yaml");
    let output_path = root.join("src/generated.rs");
    let text = std::fs::read_to_string(&spec_path)
        .map_err(|error| format!("reading {}: {error}", spec_path.display()))?;
    match mode {
        Mode::SpecVersion => {
            println!("{}", spec_version(&text)?);
            Ok(())
        }
        Mode::Check => {
            let current = std::fs::read_to_string(&output_path).unwrap_or_default();
            if current == generate(&text)? {
                Ok(())
            } else {
                Err(format!(
                    "{} is stale, run `cargo run -p codegen`",
                    output_path.display()
                ))
            }
        }
        Mode::Write => std::fs::write(&output_path, generate(&text)?)
            .map_err(|error| format!("writing {}: {error}", output_path.display())),
    }
}

/// The `info.version` of the description in `yaml`.
fn spec_version(yaml: &str) -> Result<String, String> {
    let spec: Value =
        serde_norway::from_str(yaml).map_err(|error| format!("parsing the spec: {error}"))?;
    spec.pointer("/info/version")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "the spec has no info.version".to_owned())
}

/// The Rust source of the client for the OpenAPI 3.1 description in `yaml`.
fn generate(yaml: &str) -> Result<String, String> {
    let version = spec_version(yaml)?;
    let mut spec: Value =
        serde_norway::from_str(yaml).map_err(|error| format!("parsing the spec: {error}"))?;
    lower(&mut spec)?;
    name_operations(&mut spec)?;
    let enums = string_enums(&spec)?;
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
    for string_enum in &enums {
        settings.with_replacement(
            &string_enum.name,
            format!("crate::types::{}", string_enum.name),
            [TypeImpl::Display, TypeImpl::FromStr].into_iter(),
        );
    }
    let tokens = Generator::new(&settings)
        .generate_tokens(&openapi)
        .map_err(|error| format!("generating the client: {error}"))?;
    let mut file: syn::File =
        syn::parse2(tokens).map_err(|error| format!("parsing the generated tokens: {error}"))?;
    let types = file
        .items
        .iter_mut()
        .find_map(|item| match item {
            syn::Item::Mod(module) if module.ident == "types" => module.content.as_mut(),
            _ => None,
        })
        .ok_or("progenitor generated no `types` module")?;
    for string_enum in &enums {
        types.1.extend(string_enum.items()?);
    }
    // progenitor's usage sketches name undeclared variables, so they are shown as text instead of counted as ignored doctests.
    let source = prettyplease::unparse(&file).replace("```ignore", "```text");
    Ok(format!(
        "// Generated by `cargo run -p codegen` from spec/enablebanking-api.yaml, API version {version}. Do not edit.\n\n{source}"
    ))
}

/// A component schema that is a string enumeration, generated as an enum that keeps values the description does not list.
struct StringEnum {
    /// Name of the Rust type, as typify names the schema and keys its replacements.
    name: String,
    /// Documentation of the type.
    doc: String,
    /// The listed values, each with its variant name.
    variants: Vec<(String, String)>,
}

/// Every component schema that is a string enumeration.
fn string_enums(spec: &Value) -> Result<Vec<StringEnum>, String> {
    let schemas = spec
        .pointer("/components/schemas")
        .and_then(Value::as_object)
        .ok_or("the spec has no component schemas")?;
    let mut enums = Vec::new();
    for (schema, body) in schemas {
        let Some(values) = body.get("enum").and_then(Value::as_array) else {
            continue;
        };
        let mut variants = Vec::with_capacity(values.len());
        for value in values {
            let value = value
                .as_str()
                .ok_or_else(|| format!("schema {schema} lists the non-string value {value}"))?;
            let variant = value.to_upper_camel_case();
            if variant.is_empty()
                || variant.starts_with(|c: char| c.is_ascii_digit())
                || variant == "Unknown"
                || variants.iter().any(|(_, taken)| *taken == variant)
            {
                return Err(format!(
                    "value {value:?} of schema {schema} makes no distinct variant name"
                ));
            }
            variants.push((value.to_owned(), variant));
        }
        let doc = body
            .get("description")
            .and_then(Value::as_str)
            .filter(|description| !description.trim().is_empty())
            .map_or_else(|| format!("`{schema}`"), str::to_owned);
        enums.push(StringEnum {
            name: schema.to_upper_camel_case(),
            doc,
            variants,
        });
    }
    Ok(enums)
}

impl StringEnum {
    /// The enum and its impls, as items of the `types` module.
    fn items(&self) -> Result<Vec<syn::Item>, String> {
        let name = syn::Ident::new(&self.name, proc_macro2::Span::call_site());
        let doc = &self.doc;
        let values: Vec<&String> = self.variants.iter().map(|(value, _)| value).collect();
        let idents: Vec<syn::Ident> = self
            .variants
            .iter()
            .map(|(_, variant)| syn::Ident::new(variant, proc_macro2::Span::call_site()))
            .collect();
        let docs: Vec<String> = values.iter().map(|value| format!("`{value}`")).collect();
        let tokens = quote! {
            #[doc = #doc]
            ///
            /// Values the description does not list arrive as [`Self::Unknown`].
            #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
            #[non_exhaustive]
            pub enum #name {
                #(
                    #[doc = #docs]
                    #idents,
                )*
                /// A value this version of the description does not list, kept as sent.
                Unknown(::std::string::String),
            }
            impl #name {
                /// The value as the API spells it.
                pub fn as_str(&self) -> &str {
                    match self {
                        #( Self::#idents => #values, )*
                        Self::Unknown(value) => value,
                    }
                }
            }
            impl ::std::fmt::Display for #name {
                fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                    f.write_str(self.as_str())
                }
            }
            impl ::std::str::FromStr for #name {
                type Err = ::std::convert::Infallible;
                fn from_str(value: &str) -> ::std::result::Result<Self, Self::Err> {
                    Ok(match value {
                        #( #values => Self::#idents, )*
                        other => Self::Unknown(other.to_owned()),
                    })
                }
            }
            impl ::std::convert::From<&str> for #name {
                fn from(value: &str) -> Self {
                    match value.parse() {
                        Ok(parsed) => parsed,
                        Err(never) => match never {},
                    }
                }
            }
            impl ::serde::Serialize for #name {
                fn serialize<S: ::serde::Serializer>(&self, serializer: S) -> ::std::result::Result<S::Ok, S::Error> {
                    serializer.serialize_str(self.as_str())
                }
            }
            impl<'de> ::serde::Deserialize<'de> for #name {
                fn deserialize<D: ::serde::Deserializer<'de>>(deserializer: D) -> ::std::result::Result<Self, D::Error> {
                    let value = <::std::string::String as ::serde::Deserialize>::deserialize(deserializer)?;
                    Ok(Self::from(value.as_str()))
                }
            }
        };
        let file: syn::File =
            syn::parse2(tokens).map_err(|error| format!("parsing enum {}: {error}", self.name))?;
        Ok(file.items)
    }
}

/// The schema of a string with `format`, as typify matches conversions.
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
    lower_value(spec, "", false)
}

/// Lowers `value` and everything below it. In a `properties` map the keys are property names, so only the schemas they hold are lowered.
fn lower_value(value: &mut Value, path: &str, properties_map: bool) -> Result<(), String> {
    match value {
        Value::Object(object) if properties_map => {
            for (name, schema) in object.iter_mut() {
                lower_value(schema, &format!("{path}/{name}"), false)?;
            }
            Ok(())
        }
        Value::Object(object) => {
            lower_object(object, path)?;
            for (key, child) in object.iter_mut() {
                lower_value(child, &format!("{path}/{key}"), key == "properties")?;
            }
            Ok(())
        }
        Value::Array(items) => {
            for (index, child) in items.iter_mut().enumerate() {
                lower_value(child, &format!("{path}/{index}"), false)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Marks every property outside `required` as nullable. The API sends `null` for unset optional properties, while the description only declares them optional.
fn nullable_optionals(object: &mut Map<String, Value>) {
    let required: Vec<String> = object
        .get("required")
        .and_then(Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let Some(Value::Object(properties)) = object.get_mut("properties") else {
        return;
    };
    for (name, schema) in properties.iter_mut() {
        if required.contains(name) {
            continue;
        }
        let Value::Object(schema) = schema else {
            continue;
        };
        if let Some(reference) = schema.remove("$ref") {
            // A `$ref` ignores its siblings in OpenAPI 3.0, so the reference moves into an `allOf` that can carry `nullable`.
            schema.insert(
                "allOf".to_owned(),
                Value::Array(vec![serde_json::json!({ "$ref": reference })]),
            );
        }
        schema.insert("nullable".to_owned(), Value::Bool(true));
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
    nullable_optionals(object);
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
#[expect(clippy::unwrap_used, reason = "a test panics on any unexpected value")]
mod tests {
    use serde_json::json;

    use super::{OPERATIONS, lower, name_operations, string_enums};

    #[test]
    fn nullable_type_lists_constants_and_schema_examples_become_openapi_3_0() {
        let mut spec = json!({
            "openapi": "3.1.0",
            "components": {"schemas": {"Note": {"type": "object", "required": ["text", "kind", "parent", "either", "tags"], "properties": {
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

    #[test]
    fn optional_properties_become_nullable_and_required_ones_stay() {
        let mut spec = json!({
            "openapi": "3.1.0",
            "components": {"schemas": {"Party": {
                "type": "object",
                "required": ["name"],
                "properties": {
                    "name": {"type": "string"},
                    "aliases": {"type": "array", "items": {"type": "string"}},
                    "account": {"$ref": "#/components/schemas/Account", "description": "Where it pays from"},
                    "examples": {"type": "string"},
                    "if": {"type": "integer"},
                },
            }}},
        });
        lower(&mut spec).unwrap();
        let properties = &spec["components"]["schemas"]["Party"]["properties"];
        assert_eq!(properties["name"], json!({"type": "string"}));
        assert_eq!(
            properties["aliases"],
            json!({"type": "array", "items": {"type": "string"}, "nullable": true})
        );
        assert_eq!(
            properties["account"],
            json!({"allOf": [{"$ref": "#/components/schemas/Account"}], "description": "Where it pays from", "nullable": true})
        );
        assert_eq!(
            properties["examples"],
            json!({"type": "string", "nullable": true})
        );
        assert_eq!(
            properties["if"],
            json!({"type": "integer", "nullable": true})
        );
    }

    #[test]
    fn string_enums_are_named_like_typify_and_refuse_ambiguous_values() {
        let spec = json!({"components": {"schemas": {
            "PSUType": {"type": "string", "enum": ["personal", "business"]},
            "Amount": {"type": "object"},
        }}});
        let enums = string_enums(&spec).unwrap();
        assert_eq!(enums.len(), 1);
        assert_eq!(enums[0].name, "PsuType");
        assert_eq!(
            enums[0].variants,
            [
                ("personal".to_owned(), "Personal".to_owned()),
                ("business".to_owned(), "Business".to_owned())
            ]
        );
        assert!(enums[0].items().is_ok());
        for values in [
            json!(["UNKNOWN"]),
            json!(["1ST"]),
            json!(["A_B", "a_b"]),
            json!([1]),
        ] {
            let spec =
                json!({"components": {"schemas": {"Odd": {"type": "string", "enum": values}}}});
            assert!(string_enums(&spec).is_err(), "{values}");
        }
    }

    #[test]
    fn an_enum_is_documented_by_its_description_or_else_by_its_name() {
        let spec = json!({"components": {"schemas": {
            "Usage": {"type": "string", "enum": ["PRIV"], "description": "Account usage"},
            "Blank": {"type": "string", "enum": ["X"], "description": "  "},
        }}});
        let docs: Vec<(String, String)> = string_enums(&spec)
            .unwrap()
            .into_iter()
            .map(|string_enum| (string_enum.name, string_enum.doc))
            .collect();
        assert_eq!(
            docs,
            [
                ("Blank".to_owned(), "`Blank`".to_owned()),
                ("Usage".to_owned(), "Account usage".to_owned())
            ]
        );
    }
}
