//! The strict check the weekly sandbox job applies to every raw response.

use serde_json::json;

include!("support/conformance.rs");

const ACCOUNT: &str = "/accounts/07896a3e-45d1-4bd0-9f8e-1cb32c8a6e01";

fn recorded(name: &str) -> serde_json::Value {
    let path = format!(
        "{}/tests/fixtures/sandbox/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn bytes(body: &serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(body).unwrap()
}

#[test]
fn recorded_sandbox_responses_conform() {
    let conformance = Conformance::new();
    for (path, file) in [
        (
            "/sessions/4ab6c0a8-0b5c-4bf6-8c8a-5d1b1c0f2e33".to_owned(),
            "session.json",
        ),
        (format!("{ACCOUNT}/transactions"), "transactions.json"),
        (format!("{ACCOUNT}/details"), "details.json"),
        (format!("{ACCOUNT}/balances"), "balances.json"),
        ("/application".to_owned(), "application.json"),
    ] {
        let violations =
            conformance.response_violations("GET", &path, 200, &bytes(&recorded(file)));
        assert_eq!(violations, Vec::<String>::new(), "{file}");
    }
}

#[test]
fn undeclared_properties_nulls_and_enum_values_are_reported() {
    let conformance = Conformance::new();
    let check = |body: &serde_json::Value| {
        conformance
            .response_violations("GET", "/application", 200, &bytes(body))
            .len()
    };
    let mut extra = recorded("application.json");
    extra["undocumented"] = json!(1);
    assert_eq!(check(&extra), 1);
    let mut null_required = recorded("application.json");
    null_required["kid"] = json!(null);
    assert_eq!(check(&null_required), 1);
    let mut wrong_enum = recorded("application.json");
    wrong_enum["environment"] = json!("STAGING");
    assert_eq!(check(&wrong_enum), 1);
}

#[test]
fn error_bodies_are_checked_against_the_documented_error_schema() {
    let conformance = Conformance::new();
    let error = json!({"code": 422, "message": "No transaction found matching provided id", "error": "TRANSACTION_DOES_NOT_EXIST", "detail": null});
    let path = format!("{ACCOUNT}/transactions/missing");
    assert_eq!(
        conformance.response_violations("GET", &path, 422, &bytes(&error)),
        Vec::<String>::new()
    );
    let unknown_code = json!({"message": "x", "error": "SOMETHING_NEW"});
    assert_eq!(
        conformance
            .response_violations("GET", &path, 422, &bytes(&unknown_code))
            .len(),
        1
    );
}

#[test]
fn undescribed_operations_and_undocumented_statuses_are_reported() {
    let conformance = Conformance::new();
    assert_eq!(
        conformance.response_violations("GET", "/unknown", 200, b"{}"),
        ["GET /unknown is not described"]
    );
    assert_eq!(
        conformance.response_violations("PATCH", "/application", 200, b"{}"),
        ["PATCH /application is not described"]
    );
    assert_eq!(
        conformance.response_violations("GET", "/application", 418, b"{}"),
        ["status 418 is not documented"]
    );
    assert_eq!(
        conformance
            .response_violations("GET", "/application", 200, b"not json")
            .len(),
        1
    );
}

#[test]
fn a_templated_path_matches_its_operation() {
    let conformance = Conformance::new();
    let deleted = json!({"message": "OK"});
    let path = "/payments/77915a83-762d-422c-a8e3-f5ed89ff2f36";
    assert_eq!(
        conformance.response_violations("DELETE", path, 200, &bytes(&deleted)),
        Vec::<String>::new()
    );
    let transactions = recorded("transactions.json");
    assert_eq!(
        conformance.response_violations(
            "GET",
            &format!("{ACCOUNT}/transactions"),
            200,
            &bytes(&transactions)
        ),
        Vec::<String>::new()
    );
}
