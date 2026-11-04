//! Responses recorded from Enable Banking's sandbox (Mock ASPSP holding Enable Banking's synthetic Danske Bank sample), which send `null` for many optional properties.

#![expect(clippy::unwrap_used, reason = "a test panics on any unexpected value")]

use enablebanking::types::{
    AccountResource, CreditDebitIndicator, Environment, GetApplicationResponse, GetSessionResponse,
    HalBalances, HalTransactions,
};

fn parse<T: serde::de::DeserializeOwned>(json: &str) -> T {
    serde_json::from_str(json).unwrap()
}

#[test]
fn a_session_with_null_optionals_parses() {
    let session: GetSessionResponse = parse(include_str!("fixtures/sandbox/session.json"));
    assert_eq!(session.accounts.len(), 1);
    assert_eq!(session.access.accounts, None);
    assert_eq!(session.closed, None);
}

#[test]
fn a_page_of_transactions_with_null_optionals_parses() {
    let page: HalTransactions = parse(include_str!("fixtures/sandbox/transactions.json"));
    assert_eq!(page.transactions.len(), 100);
    assert!(page.continuation_key.is_some());
    let credits = page
        .transactions
        .iter()
        .filter(|transaction| transaction.credit_debit_indicator == CreditDebitIndicator::Crdt)
        .count();
    assert!(credits > 0 && credits < 100);
    assert!(page.transactions.iter().all(|transaction| {
        transaction
            .debtor_account_additional_identification
            .is_none()
    }));
}

#[test]
fn account_details_balances_and_application_parse() {
    let account: AccountResource = parse(include_str!("fixtures/sandbox/details.json"));
    assert_eq!(account.currency, "DKK");
    let balances: HalBalances = parse(include_str!("fixtures/sandbox/balances.json"));
    assert_eq!(balances.balances.len(), 3);
    let application: GetApplicationResponse =
        parse(include_str!("fixtures/sandbox/application.json"));
    assert_eq!(application.environment, Environment::Sandbox);
    assert_eq!(application.description, None);
}
