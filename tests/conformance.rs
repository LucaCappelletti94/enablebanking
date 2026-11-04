//! The strict check the weekly sandbox job applies to raw responses.

use serde_json::json;

include!("support/conformance.rs");

fn recorded(name: &str) -> serde_json::Value {
    let path = format!(
        "{}/tests/fixtures/sandbox/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn recorded_sandbox_responses_conform() {
    let conformance = Conformance::new();
    for (file, schema) in [
        ("session.json", "GetSessionResponse"),
        ("transactions.json", "HalTransactions"),
        ("details.json", "AccountResource"),
        ("balances.json", "HalBalances"),
        ("application.json", "GetApplicationResponse"),
    ] {
        assert_eq!(
            conformance.violations(schema, &recorded(file)),
            Vec::<String>::new(),
            "{file}"
        );
    }
}

#[test]
fn undeclared_properties_and_nulls_in_required_ones_are_reported() {
    let conformance = Conformance::new();
    let mut extra = recorded("application.json");
    extra["undocumented"] = json!(1);
    let violations = conformance.violations("GetApplicationResponse", &extra);
    assert_eq!(violations.len(), 1, "{violations:?}");

    let mut null_required = recorded("application.json");
    null_required["kid"] = json!(null);
    assert_eq!(
        conformance
            .violations("GetApplicationResponse", &null_required)
            .len(),
        1
    );

    let mut wrong_enum = recorded("application.json");
    wrong_enum["environment"] = json!("STAGING");
    assert_eq!(
        conformance
            .violations("GetApplicationResponse", &wrong_enum)
            .len(),
        1
    );
}
