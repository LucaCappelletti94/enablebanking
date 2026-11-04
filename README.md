# enablebanking

[![CI](https://github.com/LucaCappelletti94/enablebanking/actions/workflows/ci.yml/badge.svg)](https://github.com/LucaCappelletti94/enablebanking/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/LucaCappelletti94/enablebanking/graph/badge.svg)](https://codecov.io/gh/LucaCappelletti94/enablebanking)
[![crates.io](https://img.shields.io/crates/v/enablebanking.svg)](https://crates.io/crates/enablebanking)
[![docs.rs](https://docs.rs/enablebanking/badge.svg)](https://docs.rs/enablebanking)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/LucaCappelletti94/enablebanking/blob/main/LICENSE)

Async client for the [Enable Banking API](https://enablebanking.com/docs/api/reference/), which reads accounts and starts payments at about 2,500 European banks through PSD2. Every endpoint and type is generated from Enable Banking's own OpenAPI description, and a weekly job regenerates the client when that description changes. Requests are signed with your application's RSA key. Not affiliated with Enable Banking.

```rust
use enablebanking::types::{Access, Aspsp, AuthorizeSessionRequest, StartAuthorizationRequest};
use enablebanking::{API_URL, Client, Signer};

# const APPLICATION_ID: &str = "cf589be3-3755-465b-a8df-a90a16a31403";
# fn main() -> Result<(), Box<dyn std::error::Error>> {
// The application id and the private key of the certificate uploaded when registering the application.
let key = std::fs::read("tests/fixtures/test-key.pem")?;
let client = Client::new(API_URL, Signer::new(APPLICATION_ID, &key)?);
# let _ = (link, transactions);
# Ok(())
# }

/// Sends the user to their bank to grant 90 days of access.
async fn link(client: &Client) -> Result<String, Box<dyn std::error::Error>> {
    let valid_until = jiff::Timestamp::now() + jiff::SignedDuration::from_hours(90 * 24);
    let started = client
        .start_authorization()
        .body(
            StartAuthorizationRequest::builder()
                .access(Access::builder().valid_until(valid_until))
                .aspsp(Aspsp::builder().name("CrediFriuli").country("IT"))
                .redirect_url("https://example.com/callback")
                .state("any opaque value"),
        )
        .send()
        .await?;
    Ok(started.url.to_string())
}

/// Reads every account the user granted, after the bank redirected back with `code`.
async fn transactions(client: &Client, code: &str) -> Result<(), Box<dyn std::error::Error>> {
    let session = client
        .authorize_session()
        .body(AuthorizeSessionRequest::builder().code(code))
        .send()
        .await?;
    for account in session.accounts.iter().filter_map(|account| account.uid) {
        for transaction in client.all_transactions(account, None, None).await? {
            println!(
                "{:?} {} {:?}",
                transaction.booking_date,
                transaction.transaction_amount,
                transaction.remittance_information
            );
        }
    }
    Ok(())
}
```

The method names follow the API: `start_authorization`, `authorize_session`, `get_session`, `get_account_balances`, `get_account_transactions`, `create_payment`, `get_aspsps` and so on. Optional parameters are builder methods, and documented error responses come back as `Error::ErrorResponse` holding the typed body. Amounts are `rust_decimal::Decimal` and dates are `jiff` types.

## Keeping up with the API

`spec/enablebanking-api.yaml` is a verbatim copy of the published description, and `src/generated.rs` is produced from it by `cargo run -p codegen`. Continuous integration fails when the generated file is stale. A weekly workflow downloads the published description and, when it changed, opens a pull request with the regenerated client. A new endpoint stops generation until it is given a method name in `codegen`, so names stay readable.
