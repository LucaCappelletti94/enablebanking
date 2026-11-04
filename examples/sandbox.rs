//! Calls every endpoint against Enable Banking's sandbox, approving consents for the Nordea FI sandbox in a headless Chrome, and checks each GET response strictly against `spec/enablebanking-api.yaml`.
//!
//! Environment: `ENABLEBANKING_SANDBOX_APP` (application id), `ENABLEBANKING_SANDBOX_KEY` (its private key in PEM) and optionally `CHROME` (default `google-chrome`).
//!
//! No sandbox bank returns transaction ids, so `get_transaction` is checked for its documented error only.

use std::process::ExitCode;
use std::time::Duration;

use chromiumoxide::{Browser, BrowserConfig, Page};
use enablebanking::types::{
    Access, Aspsp, AuthorizeSessionRequest, CreatePaymentRequest, Environment, ErrorCode,
    PaymentStatus, PsuType, SessionStatus, StartAuthorizationRequest,
};
use enablebanking::{API_URL, Client, Error, Signer};
use futures_util::StreamExt;
use jiff::{SignedDuration, Timestamp};
use serde_json::{Map, Value};
use uuid::Uuid;

type Result<T, E = Box<dyn std::error::Error + Send + Sync>> = std::result::Result<T, E>;

const BANK: &str = "Nordea";
const COUNTRY: &str = "FI";
const REDIRECT: &str = "https://github.com/LucaCappelletti94/enablebanking";
const CREDITOR_IBAN: &str = "FI6593857450293470";

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run().await {
        Ok(failures) if failures.is_empty() => {
            println!("every endpoint answered as described");
            ExitCode::SUCCESS
        }
        Ok(failures) => {
            for failure in &failures {
                eprintln!("FAIL {failure}");
            }
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("sandbox: {error}");
            ExitCode::FAILURE
        }
    }
}

fn variable(name: &str) -> Result<String> {
    std::env::var(name).map_err(|_| format!("{name} is not set").into())
}

/// The run, returning every check that failed.
async fn run() -> Result<Vec<String>> {
    let signer = Signer::new(
        variable("ENABLEBANKING_SANDBOX_APP")?,
        variable("ENABLEBANKING_SANDBOX_KEY")?.as_bytes(),
    )?;
    let client = Client::new(API_URL, signer.clone());
    let mut check = Checker::new(signer);
    let chrome = std::env::var("CHROME").unwrap_or_else(|_| "google-chrome".to_owned());
    let consent = Consent::start(&chrome).await?;

    let application = client.get_application().send().await?;
    if application.environment != Environment::Sandbox {
        return Err(format!("{} is not a sandbox application", application.name).into());
    }
    check.get("/application", "GetApplicationResponse").await;
    client.get_aspsps().country(COUNTRY).send().await?;
    check.get("/aspsps", "GetAspspsResponse").await;

    accounts(&client, &mut check, &consent).await?;
    payments(&client, &mut check, &consent).await?;
    Ok(check.failures)
}

/// Links the sandbox bank, reads every account, then deletes the session.
async fn accounts(client: &Client, check: &mut Checker, consent: &Consent) -> Result<()> {
    let started = client
        .start_authorization()
        .body(
            StartAuthorizationRequest::builder()
                .access(
                    Access::builder().valid_until(Timestamp::now() + SignedDuration::from_hours(1)),
                )
                .aspsp(Aspsp::builder().name(BANK).country(COUNTRY))
                .psu_type(PsuType::Personal)
                .redirect_url(REDIRECT)
                .state("sandbox check"),
        )
        .send()
        .await?;
    let code = consent.approve_accounts(started.url.as_str()).await?;
    let session = client
        .authorize_session()
        .body(AuthorizeSessionRequest::builder().code(code))
        .send()
        .await?;
    let session_id = session.session_id.clone();
    client
        .get_session()
        .session_id(session_id.as_str())
        .send()
        .await?;
    check
        .get(&format!("/sessions/{session_id}"), "GetSessionResponse")
        .await;
    for account in session.accounts.iter().filter_map(|account| account.uid) {
        client
            .get_account_details()
            .account_id(account)
            .send()
            .await?;
        check
            .get(&format!("/accounts/{account}/details"), "AccountResource")
            .await;
        client
            .get_account_balances()
            .account_id(account)
            .send()
            .await?;
        check
            .get(&format!("/accounts/{account}/balances"), "HalBalances")
            .await;
        let transactions = client.all_transactions(account, None, None).await?;
        check
            .pages(&format!("/accounts/{account}/transactions"))
            .await;
        println!("account {account}: {} transactions", transactions.len());
        match client
            .get_transaction()
            .account_id(account)
            .transaction_id("enablebanking-crate-check-missing")
            .send()
            .await
        {
            Err(Error::ErrorResponse(response))
                if response.error == Some(ErrorCode::TransactionDoesNotExist) => {}
            other => check.fail(format!("get_transaction of a missing id: {other:?}")),
        }
    }
    client
        .delete_session()
        .session_id(session_id.as_str())
        .send()
        .await?;
    let closed = client
        .get_session()
        .session_id(session_id.as_str())
        .send()
        .await?;
    if closed.status != SessionStatus::Closed {
        check.fail(format!(
            "session {session_id} is {:?} after deletion",
            closed.status
        ));
    }
    Ok(())
}

/// A credit transfer to the sandbox creditor, tagged so its ids are recognisable.
fn transfer(amount: &str, tag: &str) -> Value {
    serde_json::json!({
        "instructed_amount": {"currency": "EUR", "amount": amount},
        "beneficiary": {
            "creditor": {"name": "Sandbox Creditor"},
            "creditor_account": {"scheme_name": "IBAN", "identification": CREDITOR_IBAN},
        },
        "remittance_information": [format!("enablebanking crate check {tag}")],
        "payment_id": {"instruction_id": format!("instr-{tag}"), "end_to_end_id": format!("e2e-{tag}")},
    })
}

/// A payment request from the sandbox bank, typed by the client's own request type.
fn payment(kind: &str, request: &Value) -> Result<CreatePaymentRequest> {
    Ok(serde_json::from_value(serde_json::json!({
        "aspsp": {"name": BANK, "country": COUNTRY},
        "payment_type": kind,
        "psu_type": "personal",
        "redirect_url": REDIRECT,
        "state": "sandbox check",
        "payment_request": request,
    }))?)
}

/// Pays once and in bulk from the sandbox bank, reads the payments and their transactions, then deletes them.
async fn payments(client: &Client, check: &mut Checker, consent: &Consent) -> Result<()> {
    let single = client
        .create_payment()
        .body(payment(
            "SEPA",
            &serde_json::json!({"credit_transfer_transaction": [transfer("1.00", "single")]}),
        )?)
        .send()
        .await?;
    consent.approve_payment(single.url.as_str()).await?;
    let paid = settle(client, single.payment_id).await?;
    check
        .get(
            &format!("/payments/{}", single.payment_id),
            "GetPaymentResponse",
        )
        .await;
    match client
        .submit_payment()
        .payment_id(single.payment_id)
        .body(Map::new())
        .send()
        .await
    {
        Err(Error::ErrorResponse(response))
            if response.error == Some(ErrorCode::PaymentSubmissionNotDeferred) => {}
        other => check.fail(format!(
            "submit_payment on a payment without deferred submission: {other:?}"
        )),
    }
    if paid {
        client
            .delete_payment()
            .payment_id(single.payment_id)
            .send()
            .await?;
    } else {
        check.fail(format!(
            "payment {} never reached a final status",
            single.payment_id
        ));
    }

    let bulk = client
        .create_payment()
        .body(payment(
            "BULK_SEPA",
            &serde_json::json!({
                "payment_information_id": "enablebanking crate check",
                "credit_transfer_transaction": [transfer("1.00", "bulk-1"), transfer("2.00", "bulk-2")],
            }),
        )?)
        .send()
        .await?;
    consent.approve_payment(bulk.url.as_str()).await?;
    let payment = client
        .get_payment()
        .payment_id(bulk.payment_id)
        .send()
        .await?;
    check
        .get(
            &format!("/payments/{}", bulk.payment_id),
            "GetPaymentResponse",
        )
        .await;
    let ids: Vec<String> = payment
        .payment_details
        .credit_transfer_transaction
        .iter()
        .flatten()
        .filter_map(|transfer| transfer.transaction_id.clone())
        .collect();
    if ids.is_empty() {
        check.fail(format!(
            "bulk payment {} lists no transaction ids",
            bulk.payment_id
        ));
    }
    for id in ids {
        client
            .get_payment_transaction()
            .payment_id(bulk.payment_id)
            .transaction_id(id.as_str())
            .send()
            .await?;
        check
            .get(
                &format!("/payments/{}/transactions/{id}", bulk.payment_id),
                "GetPaymentTransactionResponse",
            )
            .await;
    }
    Ok(())
}

/// Waits up to a minute for `payment` to reach a final status, and tells whether it did.
async fn settle(client: &Client, payment: Uuid) -> Result<bool> {
    for _ in 0..30 {
        let current = client.get_payment().payment_id(payment).send().await?;
        if current.final_status {
            return Ok(current.status == PaymentStatus::Acsc);
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    Ok(false)
}

/// A headless Chrome that clicks through the sandbox consent pages.
struct Consent {
    browser: Browser,
}

impl Consent {
    async fn start(chrome: &str) -> Result<Self> {
        let profile =
            std::env::temp_dir().join(format!("enablebanking-sandbox-{}", std::process::id()));
        let config = BrowserConfig::builder()
            .chrome_executable(chrome)
            .user_data_dir(profile)
            .request_timeout(Duration::from_secs(60))
            .build()?;
        let (browser, mut events) = Browser::launch(config).await?;
        tokio::spawn(async move { while events.next().await.is_some() {} });
        Ok(Self { browser })
    }

    /// Approves account access at `url` and returns the code the bank redirects with.
    async fn approve_accounts(&self, url: &str) -> Result<String> {
        let page = self.browser.new_page(url).await?;
        click(&page, "Continue authentication").await?;
        let redirected = wait_for(&page, |url| url.starts_with(REDIRECT)).await?;
        page.close().await?;
        url::Url::parse(&redirected)?
            .query_pairs()
            .find(|(key, _)| key == "code")
            .map(|(_, code)| code.into_owned())
            .ok_or_else(|| format!("{redirected} carries no code").into())
    }

    /// Approves the payment at `url`, paying from the first account offered.
    async fn approve_payment(&self, url: &str) -> Result<()> {
        let page = self.browser.new_page(url).await?;
        click(&page, "Continue authentication").await?;
        wait_for_script(
            &page,
            "document.querySelector('input[type=radio]') !== null",
        )
        .await?;
        page.evaluate("document.querySelector('input[type=radio]').click()")
            .await?;
        click(&page, "Select account").await?;
        wait_for(&page, |url| !url.contains("tilisy-sandbox")).await?;
        tokio::time::sleep(Duration::from_secs(3)).await;
        page.close().await?;
        Ok(())
    }
}

/// Clicks the button labelled `label` once it appears.
async fn click(page: &Page, label: &str) -> Result<()> {
    let find = format!(
        "Array.from(document.querySelectorAll('button')).find(b => b.textContent.trim() === {label:?})"
    );
    wait_for_script(page, &format!("{find} !== undefined")).await?;
    page.evaluate(format!("{find}.click()")).await?;
    Ok(())
}

async fn wait_for_script(page: &Page, condition: &str) -> Result<()> {
    for _ in 0..60 {
        if page
            .evaluate(condition)
            .await
            .ok()
            .and_then(|result| result.into_value::<bool>().ok())
            == Some(true)
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(format!("timed out waiting for `{condition}`").into())
}

async fn wait_for(page: &Page, done: impl Fn(&str) -> bool) -> Result<String> {
    for _ in 0..60 {
        if let Some(url) = page.url().await?.filter(|url| done(url)) {
            return Ok(url);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(format!("timed out on {:?}", page.url().await?).into())
}

/// Re-fetches GET responses raw and records how they depart from the description.
struct Checker {
    signer: Signer,
    http: reqwest::Client,
    conformance: Conformance,
    failures: Vec<String>,
}

impl Checker {
    fn new(signer: Signer) -> Self {
        Self {
            signer,
            http: reqwest::Client::new(),
            conformance: Conformance::new(),
            failures: Vec::new(),
        }
    }

    fn fail(&mut self, failure: String) {
        self.failures.push(failure);
    }

    async fn fetch(&self, path: &str) -> Result<Value> {
        let response = self
            .http
            .get(format!("{API_URL}{path}"))
            .bearer_auth(self.signer.token()?)
            .send()
            .await?
            .error_for_status()?;
        Ok(response.json().await?)
    }

    /// Fetches `path` again and validates it as `schema`.
    async fn get(&mut self, path: &str, schema: &str) {
        match self.fetch(path).await {
            Ok(body) => self.validate(path, schema, &body),
            Err(error) => self.fail(format!("GET {path}: {error}")),
        }
    }

    /// Validates every page of the transactions at `path`.
    async fn pages(&mut self, path: &str) {
        let mut key: Option<String> = None;
        loop {
            let page_path = key.as_ref().map_or_else(
                || path.to_owned(),
                |key| format!("{path}?continuation_key={key}"),
            );
            let body = match self.fetch(&page_path).await {
                Ok(body) => body,
                Err(error) => return self.fail(format!("GET {page_path}: {error}")),
            };
            self.validate(path, "HalTransactions", &body);
            key = body
                .get("continuation_key")
                .and_then(Value::as_str)
                .filter(|key| !key.is_empty())
                .map(str::to_owned);
            if key.is_none() {
                return;
            }
        }
    }

    fn validate(&mut self, path: &str, schema: &str, body: &Value) {
        for violation in self
            .conformance
            .violations(schema, body)
            .into_iter()
            .take(10)
        {
            self.fail(format!("GET {path} ({schema}): {violation}"));
        }
    }
}

include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/support/conformance.rs"
));
