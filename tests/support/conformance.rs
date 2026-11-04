// Strict validation of raw API responses against the vendored description, pulled in by `include!`.

mod conformance {
    use serde_json::{Value, json};

    /// The description's component schemas, made strict: no undeclared properties, and `null` only where the description allows it or the property is optional, as the API sends them.
    pub struct Conformance {
        components: Value,
    }

    impl Conformance {
        pub fn new() -> Self {
            let spec: Value =
                serde_norway::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/spec/enablebanking-api.yaml")))
                    .expect("the vendored description is YAML");
            let mut components = spec["components"].clone();
            strict(&mut components, false);
            Self { components }
        }

        /// Every way `body` departs from component schema `schema`.
        pub fn violations(&self, schema: &str, body: &Value) -> Vec<String> {
            let root = json!({"$ref": format!("#/components/schemas/{schema}"), "components": self.components});
            let validator = jsonschema::options()
                .with_draft(jsonschema::Draft::Draft202012)
                .build(&root)
                .expect("the vendored description compiles as JSON Schema");
            validator
                .iter_errors(body)
                .map(|error| format!("{error} at {}", error.instance_path()))
                .collect()
        }
    }

    fn strict(value: &mut Value, properties_map: bool) {
        match value {
            Value::Object(object) if properties_map => {
                object.values_mut().for_each(|schema| strict(schema, false));
            }
            Value::Object(object) => {
                if object.contains_key("properties") {
                    object
                        .entry("unevaluatedProperties")
                        .or_insert(Value::Bool(false));
                    let required: Vec<String> = object
                        .get("required")
                        .and_then(Value::as_array)
                        .map(|names| names.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                        .unwrap_or_default();
                    if let Some(Value::Object(properties)) = object.get_mut("properties") {
                        for (name, schema) in properties.iter_mut() {
                            if !required.contains(name) {
                                *schema = json!({"anyOf": [schema.take(), {"type": "null"}]});
                            }
                        }
                    }
                }
                for (key, child) in object.iter_mut() {
                    strict(child, key == "properties");
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|item| strict(item, false)),
            _ => {}
        }
    }
}

use conformance::Conformance;
