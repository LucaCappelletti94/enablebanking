// A local server answering the README example with responses shaped like Enable Banking's, pulled in by `include!`.

const APPLICATION_ID: &str = "cf589be3-3755-465b-a8df-a90a16a31403";
const MOCK_ACCOUNT: &str = "07896a3e-45d1-4bd0-9f8e-1cb32c8a6e01";

async fn mock_bank() -> wiremock::MockServer {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let access = serde_json::json!({"valid_until": "2027-02-02T00:00:00Z"});
    let aspsp = serde_json::json!({"name": "CrediFriuli", "country": "IT"});
    Mock::given(method("POST"))
        .and(path("/auth"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "url": "https://auth.enablebanking.com/ais/start?sessionid=73100c65-c54d-46a1-87d1-aa3effde435a",
            "authorization_id": "73100c65-c54d-46a1-87d1-aa3effde435a",
            "psu_id_hash": "12427b547618d01d46cabfdd6fd7e832bb42076fb210d40e56783e0d1f4798fd",
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/sessions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "session_id": "4ab6c0a8-0b5c-4bf6-8c8a-5d1b1c0f2e33",
            "accounts": [{
                "uid": MOCK_ACCOUNT,
                "account_id": {"iban": "IT60X0542811101000000123456"},
                "cash_account_type": "CACC",
                "currency": "EUR",
                "identification_hash": "WwpbCiAgImFjY291bnQiLAogICJpYmFuIgpdCl0=",
                "identification_hashes": ["WwpbCiAgImFjY291bnQiLAogICJpYmFuIgpdCl0="],
            }],
            "aspsp": aspsp,
            "psu_type": "personal",
            "access": access,
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/accounts/{MOCK_ACCOUNT}/transactions")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "transactions": [{
                "entry_reference": "5561990681",
                "transaction_amount": {"currency": "EUR", "amount": "7123.45"},
                "debtor": {"name": "Etat de Fribourg"},
                "credit_debit_indicator": "CRDT",
                "status": "BOOK",
                "booking_date": "2026-10-28",
                "remittance_information": ["SALAIRE OCTOBRE 2026"],
            }],
            "continuation_key": null,
        })))
        .expect(1)
        .mount(&server)
        .await;
    server
}
