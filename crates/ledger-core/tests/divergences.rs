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
}

// D3: a stored journal that does not balance cannot be loaded.
#[test]
fn d3_unbalanced_stored_journals_do_not_deserialize() {
    let (b, _) = fresh();
    let mut journal = serde_json::to_value(&b.journals[0]).unwrap();
    journal["lines"][0]["debit"] = json!(1);
    let err = serde_json::from_value::<JournalEntry>(journal).unwrap_err();
    assert!(err.to_string().contains("Journal entry must balance."));
}

// D4: import sources are compared structurally, not by `JSON.stringify`, so
// field order cannot make identical evidence look changed.
#[test]
fn d4_import_source_comparison_ignores_key_order() {
    let (b, mut env) = fresh();
    let mut d = blank_draft(&b, &mut env);
    let source = json!({
        "id": "s", "businessId": b.profile.id, "provider": "csv", "reference": "R",
        "fingerprint": "f", "date": "2026-10-01", "amountMinor": 1, "currency": "USD",
        "vendor": "", "category": "", "filename": ""
    });
    d.import_source = Some(serde_json::from_value(source).unwrap());
    let saved = save_draft(&b, &d, &mut env).unwrap();
    let reordered = json!({
        "filename": "", "category": "", "vendor": "", "currency": "USD", "amountMinor": 1,
        "date": "2026-10-01", "fingerprint": "f", "reference": "R", "provider": "csv",
        "businessId": b.profile.id, "id": "s"
    });
    d.import_source = Some(serde_json::from_value(reordered).unwrap());
    d.vendor = "Edited".into();
    assert!(save_draft(&saved, &d, &mut env).is_ok());
}

// D6: `parseMoney("-0")` is `-0` in JavaScript; Rust has no negative zero.
// Both serialize as `0` and format as `0.00`.
#[test]
fn d6_negative_zero_is_zero() {
    assert_eq!(parse_money("-0").unwrap(), minor(0));
    assert_eq!(amount_input(parse_money("-0.00").unwrap()), "0.00");
}
