//! Checks the client against Enable Banking's sandbox, reading the Mock ASPSP accounts of a session approved once in the browser.
//!
//! Environment: `ENABLEBANKING_SANDBOX_APP` (application id), `ENABLEBANKING_SANDBOX_KEY` (its private key in PEM) and, for the check, `ENABLEBANKING_SANDBOX_SESSION`.
//!
//! - `cargo run --example sandbox -- link <redirect url>` prints the page where access is approved.
//! - `cargo run --example sandbox -- session <code>` turns the code in the redirect into a session id.
//! - `cargo run --example sandbox` reads every account of the session and fails on any error.

use std::process::ExitCode;

use enablebanking::types::{
    Access, Aspsp, AuthorizeSessionRequest, Environment, StartAuthorizationRequest,
};
use enablebanking::{API_URL, Client, Signer};
use jiff::{SignedDuration, Timestamp};

type Result<T, E = Box<dyn std::error::Error + Send + Sync>> = std::result::Result<T, E>;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let outcome = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["link", redirect] => link(redirect).await,
        ["session", code] => session(code).await,
        [] => check().await,
        _ => Err("usage: sandbox [link <redirect url> | session <code>]".into()),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("sandbox: {error}");
            ExitCode::FAILURE
        }
    }
}

fn variable(name: &str) -> Result<String> {
    std::env::var(name).map_err(|_| format!("{name} is not set").into())
}

fn client() -> Result<Client> {
    let signer = Signer::new(
        variable("ENABLEBANKING_SANDBOX_APP")?,
        variable("ENABLEBANKING_SANDBOX_KEY")?.as_bytes(),
    )?;
    Ok(Client::new(API_URL, signer))
}

async fn link(redirect: &str) -> Result<()> {
    let started = client()?
        .start_authorization()
        .body(
            StartAuthorizationRequest::builder()
                .access(
                    Access::builder()
                        .valid_until(Timestamp::now() + SignedDuration::from_hours(180 * 24)),
                )
                .aspsp(Aspsp::builder().name("Mock ASPSP").country("FI"))
                .redirect_url(redirect)
                .state("sandbox check"),
        )
        .send()
        .await?;
    println!("{}", started.url.as_str());
    Ok(())
}

async fn session(code: &str) -> Result<()> {
    let session = client()?
        .authorize_session()
        .body(AuthorizeSessionRequest::builder().code(code))
        .send()
        .await?;
    println!(
        "{} valid until {}",
        session.session_id, session.access.valid_until
    );
    Ok(())
}

async fn check() -> Result<()> {
    let client = client()?;
    let application = client.get_application().send().await?;
    if application.environment != Environment::Sandbox {
        return Err(format!("{} is not a sandbox application", application.name).into());
    }
    let session_id = variable("ENABLEBANKING_SANDBOX_SESSION")?;
    let session = client
        .get_session()
        .session_id(session_id.as_str())
        .send()
        .await?;
    if session.accounts.is_empty() {
        return Err(format!("session {session_id} has no accounts").into());
    }
    for &account in &session.accounts {
        client
            .get_account_details()
            .account_id(account)
            .send()
            .await?;
        let balances = client
            .get_account_balances()
            .account_id(account)
            .send()
            .await?;
        let transactions = client.all_transactions(account, None, None).await?;
        println!(
            "account {account}: {} balances, {} transactions",
            balances.balances.len(),
            transactions.len()
        );
    }
    println!("session valid until {}", session.access.valid_until);
    Ok(())
}
