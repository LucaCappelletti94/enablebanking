// Strict validation of raw API responses against the vendored description, pulled in by `include!`.

/// Checks raw responses against the vendored description.
#[expect(
    clippy::expect_used,
    reason = "test support, where an unreadable description should stop the run"
)]
mod conformance {
    use serde_json::{Value, json};

    /// The description made strict: no undeclared properties, and `null` only where the description allows it or the property is optional, as the API sends them.
    pub(crate) struct Conformance {
        /// Component schemas, made strict.
        components: Value,
        /// Path items, with their response schemas made strict.
        paths: Value,
    }

    impl Conformance {
        /// The vendored description, made strict.
        pub(crate) fn new() -> Self {
            let mut spec: Value =
                serde_norway::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/spec/enablebanking-api.yaml")))
                    .expect("the vendored description is YAML");
            strict(&mut spec, false);
            Self {
                components: spec["components"].take(),
                paths: spec["paths"].take(),
            }
        }

        /// Every way a `status` response to `method path` departs from what the description documents for that operation.
        pub(crate) fn response_violations(&self, method: &str, path: &str, status: u16, body: &[u8]) -> Vec<String> {
            let Some(operation) = self.operation(method, path) else {
                return vec![format!("{method} {path} is not described")];
            };
            let responses = &operation["responses"];
            let Some(response) = responses.get(status.to_string()).or_else(|| responses.get("default")) else {
                return vec![format!("status {status} is not documented")];
            };
            let Some(schema) = response.pointer("/content/application~1json/schema") else {
                return if body.is_empty() { Vec::new() } else { vec!["a body where none is documented".to_owned()] };
            };
            match serde_json::from_slice(body) {
                Ok(body) => self.validate(schema, &body),
                Err(error) => vec![format!("the body is not JSON: {error}")],
            }
        }

        /// The operation whose path template matches `path`, preferring templates with fewer parameters.
        fn operation(&self, method: &str, path: &str) -> Option<&Value> {
            let segments: Vec<&str> = path.split('/').collect();
            let method = method.to_lowercase();
            self.paths
                .as_object()?
                .iter()
                .filter(|(template, _)| {
                    let parts: Vec<&str> = template.split('/').collect();
                    parts.len() == segments.len()
                        && parts.iter().zip(&segments).all(|(part, segment)| part.starts_with('{') || part == segment)
                })
                .min_by_key(|(template, _)| template.matches('{').count())
                .and_then(|(_, item)| item.get(&method))
        }

        /// Every way `body` departs from `schema`, whose references point into the components.
        fn validate(&self, schema: &Value, body: &Value) -> Vec<String> {
            let mut root = schema.clone();
            root["components"] = self.components.clone();
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

    /// Forbids undeclared properties and lets optional properties be `null`. In a `properties` map the keys are property names.
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
