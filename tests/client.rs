//! The client against a local server speaking the Enable Banking API.

use enablebanking::types::{CreditDebitIndicator, ErrorCode};
use enablebanking::{Amount, Client, Error, SignError, Signer};
use jiff::civil::date;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;
use wiremock::matchers::{method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

const APP: &str = "cf589be3-3755-465b-a8df-a90a16a31403";
const KEY: &[u8] = include_bytes!("fixtures/test-key.pem");
const PUBLIC_KEY: &[u8] = include_bytes!("fixtures/test-key.pub.pem");
const ACCOUNT: Uuid = Uuid::from_u128(0x0789_6a3e_45d1_4bd0_9f8e_1cb3_2c8a_6e01);

#[derive(Debug, Deserialize)]
struct Claims {
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
}

fn client(server: &MockServer) -> Client {
    Client::new(&server.uri(), Signer::new(APP, KEY).unwrap())
}

fn application() -> serde_json::Value {
    json!({
        "name": "clerk",
        "kid": APP,
        "environment": "PRODUCTION",
        "redirect_urls": ["https://example.com/"],
        "active": true,
        "countries": ["IT"],
        "services": ["AIS"],
    })
}

fn bearer(request: &wiremock::Request) -> String {
    request.headers["authorization"]
        .to_str()
        .unwrap()
        .strip_prefix("Bearer ")
        .unwrap()
        .to_owned()
}

#[tokio::test(flavor = "current_thread")]
async fn requests_carry_a_token_the_api_accepts() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/application"))
        .respond_with(ResponseTemplate::new(200).set_body_json(application()))
        .mount(&server)
        .await;

    let application = client(&server).get_application().send().await.unwrap();
    assert_eq!(application.name, "clerk");

    let requests = server.received_requests().await.unwrap();
    let token = bearer(&requests[0]);
    let header = jsonwebtoken::decode_header(&token).unwrap();
    assert_eq!(header.alg, Algorithm::RS256);
    assert_eq!(header.kid.as_deref(), Some(APP));
    assert_eq!(header.typ.as_deref(), Some("JWT"));
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&["api.enablebanking.com"]);
    validation.set_issuer(&["enablebanking.com"]);
    let claims = jsonwebtoken::decode::<Claims>(
        &token,
        &DecodingKey::from_rsa_pem(PUBLIC_KEY).unwrap(),
        &validation,
    )
    .unwrap()
    .claims;
    assert_eq!(
        (claims.iss.as_str(), claims.aud.as_str()),
        ("enablebanking.com", "api.enablebanking.com")
    );
    assert!(
        claims.exp - claims.iat <= 86_400,
        "the API refuses tokens living over a day"
    );
    assert!(claims.exp - claims.iat >= 600);
}

#[tokio::test(flavor = "current_thread")]
async fn one_token_serves_consecutive_requests() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/application"))
        .respond_with(ResponseTemplate::new(200).set_body_json(application()))
        .mount(&server)
        .await;

    let client = client(&server);
    client.get_application().send().await.unwrap();
    client.get_application().send().await.unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(bearer(&requests[0]), bearer(&requests[1]));
}

fn transaction(reference: &str, amount: &str, indicator: &str) -> serde_json::Value {
    json!({
        "entry_reference": reference,
        "transaction_amount": {"currency": "EUR", "amount": amount},
        "debtor": {"name": "Etat de Fribourg"},
        "credit_debit_indicator": indicator,
        "status": "BOOK",
        "booking_date": "2026-10-28",
        "value_date": "2026-10-28",
        "remittance_information": ["SALAIRE OCTOBRE 2026"],
    })
}

#[tokio::test(flavor = "current_thread")]
async fn all_transactions_follows_continuation_keys() {
    let server = MockServer::start().await;
    let transactions = format!("/accounts/{ACCOUNT}/transactions");
    Mock::given(method("GET"))
        .and(path(transactions.as_str()))
        .and(query_param("date_from", "2026-10-01"))
        .and(query_param_is_missing("continuation_key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "transactions": [transaction("t1", "7123.45", "CRDT")],
            "continuation_key": "page-2",
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(transactions.as_str()))
        .and(query_param("date_from", "2026-10-01"))
        .and(query_param("continuation_key", "page-2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "transactions": [transaction("t2", "12.50", "DBIT")],
            "continuation_key": null,
        })))
        .expect(1)
        .mount(&server)
        .await;

    let found = client(&server)
        .all_transactions(ACCOUNT, Some(date(2026, 10, 1)), None)
        .await
        .unwrap();

    let summary: Vec<_> = found
        .iter()
        .map(|transaction| {
            (
                transaction.entry_reference.as_deref().unwrap(),
                transaction.credit_debit_indicator,
                transaction.transaction_amount.clone(),
                transaction.booking_date,
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (
                "t1",
                CreditDebitIndicator::Crdt,
                Amount {
                    currency: "EUR".into(),
                    amount: Decimal::new(712_345, 2)
                },
                Some(date(2026, 10, 28)),
            ),
            (
                "t2",
                CreditDebitIndicator::Dbit,
                Amount {
                    currency: "EUR".into(),
                    amount: Decimal::new(1250, 2)
                },
                Some(date(2026, 10, 28)),
            ),
        ]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn documented_errors_come_back_typed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/accounts/{ACCOUNT}/transactions")))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "code": 422,
            "message": "Wrong transactions period requested",
            "error": "WRONG_TRANSACTIONS_PERIOD",
        })))
        .mount(&server)
        .await;

    let error = client(&server)
        .all_transactions(ACCOUNT, None, None)
        .await
        .unwrap_err();

    let Error::ErrorResponse(response) = error else {
        panic!("expected a documented error, got {error:?}");
    };
    assert_eq!(response.status().as_u16(), 422);
    assert_eq!(response.error, Some(ErrorCode::WrongTransactionsPeriod));
}

#[test]
fn a_key_that_is_not_rsa_pem_is_refused() {
    assert!(matches!(
        Signer::new(
            APP,
            b"-----BEGIN PRIVATE KEY-----\nnot a key\n-----END PRIVATE KEY-----\n"
        ),
        Err(SignError::Key(_))
    ));
}

#[test]
fn amounts_display_with_their_currency() {
    let amount = Amount {
        currency: "EUR".into(),
        amount: Decimal::new(372_973, 2),
    };
    assert_eq!(amount.to_string(), "3729.73 EUR");
}
