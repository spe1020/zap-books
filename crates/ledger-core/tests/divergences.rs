//! Pins every place the Rust knowingly differs from the TypeScript. Each test
//! names its entry in PARITY_NOTES.md.

mod common;

use common::{fresh, minor};
use ledger_core::*;
use serde_json::json;

// D1: `correct` on a posted expense whose journal is missing. The TypeScript
// spreads `undefined` from `journals.find(...)!` and crashes with a TypeError.
#[test]
fn d1_correct_without_a_journal_is_an_internal_error() {
    let (mut b, mut env) = fresh();
    let original = b
        .drafts
        .iter()
        .find(|d| d.status() == ExpenseStatus::Posted)
        .unwrap()
        .clone();
    b.journals
        .retain(|j| Some(&j.id) != original.journal_id.as_ref());
    let replacement = original.replacement(env.new_id());
    assert_eq!(
        correct(&b, &original.id, &replacement, "Reason", &mut env),
        Err(LedgerError::Internal("posted expense has no journal"))
    );
}

// D1: `previewImport` with no rows. The TypeScript reads `rows[0].indexOf`
// and crashes with a TypeError.
#[test]
fn d1_preview_without_a_header_is_an_internal_error() {
    let (b, mut env) = fresh();
    let result = preview_import(
        &b,
        &[],
        &CsvMapping::default(),
        &b.accounts[0].id,
        SignConvention::Negative,
        &ImportOptions::default(),
        &mut env,
    );
    assert_eq!(
        result,
        Err(LedgerError::Internal("import has no header row"))
    );
}

// D2: amounts the TypeScript would hold as non-integer or unsafe numbers,
// and unsupported currencies, are rejected when JSON is read.
#[test]
fn d2_unrepresentable_values_are_rejected_at_deserialization() {
    let (b, _) = fresh();
    let draft = serde_json::to_value(&b.drafts[0]).unwrap();
    for (field, value) in [
        ("amountMinor", json!(1.5)),
        ("amountMinor", json!(9_007_199_254_740_992_i64)),
        ("currency", json!("JPY")),
    ] {
        let mut bad = draft.clone();
        bad[field] = value;
        assert!(serde_json::from_value::<ExpenseDraft>(bad).is_err());
    }
    // A transfer's type and currency are enums too, and its amount a `Minor`.
    let transfer = json!({
        "date": "2026-10-20",
        "amountMinor": 100,
        "currency": "USD",
        "fromAccountId": "a",
        "toAccountId": "b",
        "transferType": "ownerLoan",
        "purpose": "Invented",
    });
    assert!(serde_json::from_value::<TransferInput>(transfer.clone()).is_ok());
    for (field, value) in [
        ("amountMinor", json!(1.5)),
        ("currency", json!("JPY")),
        ("transferType", json!("ownerGift")),
    ] {
        let mut bad = transfer.clone();
        bad[field] = value;
        assert!(serde_json::from_value::<TransferInput>(bad).is_err());
    }
}

#[test]
fn d2_a_stored_size_that_isnt_a_whole_number_does_not_deserialize() {
    let stored = json!({
        "hash": "4d".repeat(32),
        "size": 2064,
        "alg": "AES-256-GCM",
        "key": format!("{}w=", "q".repeat(42)),
        "nonce": "Zm9yZXhhbXBsZTEy",
    });
    assert!(serde_json::from_value::<StoredFile>(stored.clone()).is_ok());
    for size in [json!(-1), json!(1.5), json!("2064")] {
        let mut bad = stored.clone();
        bad["size"] = size;
        assert!(serde_json::from_value::<StoredFile>(bad).is_err());
    }
}

// D3: a stored journal that does not balance cannot be loaded.
#[test]
fn d3_unbalanced_stored_journals_do_not_deserialize() {
    let (b, _) = fresh();
    let mut business = serde_json::to_value(&b).unwrap();
    business["journals"][0]["lines"][0]["debit"] = json!(1);
    let err = serde_json::from_value::<BusinessData>(business).unwrap_err();
    assert!(err.to_string().contains("Journal entry must balance."));
}

// D4: import sources are compared structurally, not by `JSON.stringify`, so
// field order cannot make identical evidence look changed.
#[test]
fn d4_import_source_comparison_ignores_key_order() {
    let (b, mut env) = fresh();
    let rows = parse_csv(
        "date,description,amount,currency,reference\n2026-10-24,Audit Vendor,-42.50,USD,R",
    )
    .unwrap();
    let preview = preview_import(
        &b,
        &rows,
        &guess_mapping(&rows[0]),
        &b.accounts[0].id,
        SignConvention::Negative,
        &ImportOptions {
            kind: Some(ImportKind::ExpenseHistory),
            ..Default::default()
        },
        &mut env,
    )
    .unwrap();
    let imported = commit_import(&b, &preview, &b.accounts[0].id, &mut env).unwrap();
    let mut d = imported.drafts.last().unwrap().clone();
    let source = serde_json::to_value(d.import_source.as_ref().unwrap()).unwrap();
    // The same evidence with its keys in reverse order.
    let reordered: serde_json::Map<String, serde_json::Value> = source
        .as_object()
        .unwrap()
        .iter()
        .rev()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    d.import_source = Some(serde_json::from_value(reordered.into()).unwrap());
    d.vendor = "Edited".into();
    assert!(save_draft(&imported, &d, &mut env).is_ok());
}

// D6: `parseMoney("-0")` is `-0` in JavaScript; Rust has no negative zero.
// Both serialize as `0` and format as `0.00`.
#[test]
fn d6_negative_zero_is_zero() {
    assert_eq!(parse_money("-0").unwrap(), minor(0));
    assert_eq!(amount_input(parse_money("-0.00").unwrap()), "0.00");
}
