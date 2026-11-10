# enablebanking

[![CI](https://github.com/LucaCappelletti94/enablebanking/actions/workflows/ci.yml/badge.svg)](https://github.com/LucaCappelletti94/enablebanking/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/LucaCappelletti94/enablebanking/graph/badge.svg)](https://codecov.io/gh/LucaCappelletti94/enablebanking)
[![crates.io](https://img.shields.io/crates/v/enablebanking.svg)](https://crates.io/crates/enablebanking)
[![docs.rs](https://docs.rs/enablebanking/badge.svg)](https://docs.rs/enablebanking)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/LucaCappelletti94/enablebanking/blob/main/LICENSE)

Async Rust client for the [Enable Banking API](https://enablebanking.com/docs/api/reference/), generated from its OpenAPI description. Unofficial.

```rust
use enablebanking::types::{Access, Aspsp, AuthorizeSessionRequest, StartAuthorizationRequest};
use enablebanking::{API_URL, Client, Signer};

# include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/readme_mock.rs"));
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
// The application id and the private key of the certificate uploaded when registering the application.
let key = std::fs::read("tests/fixtures/test-key.pem")?;
let client = Client::new(API_URL, Signer::new(APPLICATION_ID, &key)?);
# let bank = mock_bank().await;
# let client = Client::new(&bank.uri(), Signer::new(APPLICATION_ID, &key)?);

let url = link(&client).await?;
println!("approve access at {url}");
// The bank then sends the user to the redirect URL with `?code=…`.
# let code = "redirect code";
transactions(&client, code).await?;
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
        for transaction in client.all_transactions(account, |request| request).await? {
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

Every endpoint is a method named after the API, with optional parameters as builder calls. Amounts are `rust_decimal::Decimal`, dates are `jiff` types, and enumerations keep codes the description does not list as `Unknown`. `Client::new` gives up on a request after 15 seconds, so slower banks need a `reqwest::Client` of your own through `Client::new_with_client`.

Weekly jobs regenerate the client from Enable Banking's description and check sandbox responses, accepting `null` for optional fields. Until 1.0, a release that follows a changed description is a new minor version. The release workflow publishes `v*` tags to crates.io and GitHub using Trusted Publishing (`release.yml`, environment `crates-io`), and accepts existing tags through manual dispatch.
