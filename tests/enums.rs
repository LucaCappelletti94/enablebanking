//! Enumerations keep values the description does not list, so a new code from a bank does not make a whole response unreadable.

#![expect(clippy::unwrap_used, reason = "a test panics on any unexpected value")]

use enablebanking::types::{Transaction, TransactionStatus};
use serde_json::json;

#[test]
fn listed_values_become_variants_and_others_are_kept_verbatim() {
    let booked: TransactionStatus = serde_json::from_value(json!("BOOK")).unwrap();
    assert_eq!(booked, TransactionStatus::Book);
    let new: TransactionStatus = serde_json::from_value(json!("NEWS")).unwrap();
    assert_eq!(new, TransactionStatus::Unknown("NEWS".into()));
    assert_eq!(serde_json::to_value(&new).unwrap(), json!("NEWS"));
    assert_eq!(serde_json::to_value(&booked).unwrap(), json!("BOOK"));
    assert_eq!(new.to_string(), "NEWS");
    assert_eq!(
        "BOOK".parse::<TransactionStatus>(),
        Ok(TransactionStatus::Book)
    );
}

#[test]
fn a_transaction_with_an_unlisted_status_still_parses() {
    let transaction: Transaction = serde_json::from_value(json!({
        "transaction_amount": {"currency": "EUR", "amount": "10.00"},
        "credit_debit_indicator": "CRDT",
        "status": "PART",
    }))
    .unwrap();
    assert_eq!(
        transaction.status,
        TransactionStatus::Unknown("PART".into())
    );
}
