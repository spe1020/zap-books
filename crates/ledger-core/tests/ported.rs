//! One Rust test for each test in `src/domain/ledger.test.ts` and
//! `src/domain/portability.test.ts`. The original name is in the comment
//! above each test.
//!
//! The TypeScript builds the seed with `seedWorkspace()`; seeding is not
//! ported, so these load the same workspace as recorded by the fixture
//! generator (`fixtures/parity/states/seed.json`, sequential ids, clock frozen
//! at 2026-10-15). `currentPeriod()` is therefore `2026-10`.

mod common;

use common::{fresh, minor, seed, PERIOD};
use ledger_core::*;

fn usd(b: &BusinessData) -> i64 {
    totals(b, PERIOD).unwrap()[&Currency::Usd].get()
}

fn by_vendor<'a>(b: &'a BusinessData, vendor: &str, status: ExpenseStatus) -> &'a ExpenseDraft {
    b.drafts
        .iter()
        .find(|d| d.vendor == vendor && d.status() == status)
        .unwrap()
}

fn err_text<T: std::fmt::Debug>(result: Result<T>) -> String {
    result.unwrap_err().to_string()
}

fn payment_map() -> CsvMapping {
    CsvMapping {
        date: "date".into(),
        description: "description".into(),
        amount: "amount".into(),
        currency: "currency".into(),
        reference: "reference".into(),
        ..CsvMapping::default()
    }
}

fn strings(row: &[&str]) -> Vec<String> {
    row.iter().map(|s| s.to_string()).collect()
}

// ---------------------------------------------------------------------------
// src/domain/ledger.test.ts
// ---------------------------------------------------------------------------

// "parses decimal inputs exactly and rejects invalid precision"
#[test]
fn parses_decimal_inputs_exactly_and_rejects_invalid_precision() {
    assert_eq!(parse_money("0.29").unwrap(), minor(29));
    assert_eq!(parse_money("100.01").unwrap(), minor(10001));
    assert_eq!(parse_money("-4.05").unwrap(), minor(-405));
    assert_eq!(amount_input(minor(10001)), "100.01");
    for x in ["1.001", "NaN", "1e4", "9007199254740992", ""] {
        assert!(parse_money(x).is_err(), "{x}");
    }
}

// "all seeded journals balance, corrections net and transfers are excluded"
#[test]
fn all_seeded_journals_balance_corrections_net_and_transfers_are_excluded() {
    let (b, _) = fresh();
    for j in &b.journals {
        assert_balanced(&b, j).unwrap();
    }
    assert_eq!(usd(&b), 458559);
    let by_category: i64 = category_totals(&b, PERIOD, Currency::Usd)
        .unwrap()
        .iter()
        .map(|c| c.amount.get())
        .sum();
    assert_eq!(by_category, 458559);
    assert!(totals(&b, "1900-01").unwrap().is_empty());
    let mut no_transfers = b.clone();
    no_transfers
        .journals
        .retain(|j| j.kind != JournalKind::Transfer);
    assert_eq!(totals(&no_transfers, PERIOD), totals(&b, PERIOD));
    let mut j = b.journals[0].to_unchecked();
    j.lines[0].debit = j.lines[0].debit.checked_add(minor(1)).unwrap();
    assert!(err_text(assert_balanced(&b, &j)).contains("balance"));
}

// "required fields and invalid transitions prevent posting"
#[test]
fn required_fields_and_invalid_transitions_prevent_posting() {
    let (b, mut env) = fresh();
    let d = blank_draft(&b, &mut env);
    assert!(validation(&b, &d).len() > 3);
    let saved = save_draft(&b, &d, &mut env).unwrap();
    assert!(approve(&saved, &d.id, &mut env).is_err());
    let posted = b
        .drafts
        .iter()
        .find(|d| d.status() == ExpenseStatus::Posted)
        .unwrap();
    // `{ ...posted, status: "draft" }`
    assert!(save_draft(&b, &posted.replacement(posted.id.clone()), &mut env).is_err());
}

// "approval is balanced and can only occur once"
#[test]
fn approval_is_balanced_and_can_only_occur_once() {
    let (b, mut env) = fresh();
    let d = by_vendor(&b, "Figma", ExpenseStatus::Draft).clone();
    let posted = approve(&b, &d.id, &mut env).unwrap();
    assert_eq!(usd(&posted), 468159);
    assert!(err_text(approve(&posted, &d.id, &mut env)).contains("already been posted"));
    assert_eq!(
        posted
            .approvals
            .iter()
            .filter(|a| a.expense_id == d.id)
            .count(),
        1
    );
    let original = b.drafts.iter().find(|x| x.id == d.id).unwrap();
    assert_eq!(original.status(), ExpenseStatus::Draft);
}

// "a correction reverses the original and posts replacement atomically"
#[test]
fn a_correction_reverses_the_original_and_posts_replacement_atomically() {
    let (b, mut env) = fresh();
    let original = by_vendor(&b, "Adobe", ExpenseStatus::Posted).clone();
    let before = b
        .journals
        .iter()
        .find(|j| Some(&j.id) == original.journal_id.as_ref())
        .unwrap()
        .clone();
    let mut replacement = original.replacement(env.new_id());
    replacement.amount_minor = minor(7000);
    replacement.journal_id = None;
    let next = correct(
        &b,
        &original.id,
        &replacement,
        "Updated invoice amount",
        &mut env,
    )
    .unwrap();
    assert_eq!(
        next.journals.iter().find(|j| j.id == before.id),
        Some(&before)
    );
    assert_eq!(
        next.journals
            .iter()
            .filter(|j| j.reverses_id.as_ref() == Some(&before.id))
            .count(),
        1
    );
    let stored = next.drafts.iter().find(|d| d.id == replacement.id).unwrap();
    assert_eq!(stored.correction_of.as_ref(), Some(&original.id));
    assert_eq!(usd(&next), 456560);
    let again = replacement.replacement(env.new_id());
    assert!(correct(&next, &original.id, &again, "Again", &mut env).is_err());
    let mut invalid = replacement.clone();
    invalid.amount_minor = minor(0);
    assert!(correct(&b, &original.id, &invalid, "Invalid", &mut env).is_err());
    let unchanged = b.drafts.iter().find(|d| d.id == original.id).unwrap();
    assert_eq!(unchanged.status(), ExpenseStatus::Posted);
}

// "duplicate review uses vendor, date, amount, and currency; requires a reason"
#[test]
fn duplicate_review_uses_vendor_date_amount_and_currency_requires_a_reason() {
    let (mut b, mut env) = fresh();
    let d = by_vendor(&b, "Linear", ExpenseStatus::Draft).clone();
    assert_eq!(duplicates(&b, &d).len(), 1);
    assert!(err_text(approve(&b, &d.id, &mut env)).contains("duplicate"));
    let mut different = d.clone();
    different.vendor = "Different vendor".into();
    assert!(duplicates(&b, &different).is_empty());
    let reason = "Different invoice and service period";
    b.drafts
        .iter_mut()
        .find(|x| x.id == d.id)
        .unwrap()
        .duplicate_reason = reason.into();
    let posted = approve(&b, &d.id, &mut env).unwrap();
    assert_eq!(posted.approvals.last().unwrap().duplicate_reason, reason);
}

// "currencies never combine and a payment account must match the currency"
#[test]
fn currencies_never_combine_and_a_payment_account_must_match_the_currency() {
    let (b, mut env) = fresh();
    let template = &b.drafts[0];
    let mut d = blank_draft(&b, &mut env);
    d.vendor = "Euro vendor".into();
    d.description = "EUR service".into();
    d.purpose = "Client work".into();
    d.category_id = template.category_id.clone();
    d.amount_minor = minor(10001);
    d.currency = Currency::Eur;
    d.account_id = b
        .accounts
        .iter()
        .find(|a| a.currency == Currency::Eur)
        .unwrap()
        .id
        .clone();
    let saved = save_draft(&b, &d, &mut env).unwrap();
    let next = approve(&saved, &d.id, &mut env).unwrap();
    let expected = [
        (Currency::Usd, minor(458559)),
        (Currency::Eur, minor(10001)),
    ];
    assert_eq!(
        totals(&next, PERIOD).unwrap(),
        expected.into_iter().collect()
    );
    let mut wrong = d.clone();
    wrong.account_id = b.accounts[0].id.clone();
    assert!(validation(&b, &wrong)
        .contains(&"Payment account currency must match the expense.".to_string()));
}

// "business separation rejects foreign drafts, accounts, evidence, and journal context"
//
// The TypeScript test also exercises `demoAI` and `MockTransport`. Those are
// adapters, outside this crate; only the domain assertions are ported.
#[test]
fn business_separation_rejects_foreign_drafts_accounts_evidence_and_journal_context() {
    let (businesses, mut env) = seed();
    let (a, b) = (&businesses[0], &businesses[1]);
    assert!(save_draft(b, &a.drafts[0], &mut env).is_err());
    assert!(approve(b, &a.drafts[0].id, &mut env).is_err());
    let mut foreign_account = a.drafts[0].clone();
    foreign_account.account_id = b.accounts[0].id.clone();
    assert!(validation(a, &foreign_account).contains(&"Payment account is required.".to_string()));
    let mut foreign_receipt = a.drafts[0].clone();
    foreign_receipt.document_ids = vec![b.documents[0].id.clone()];
    assert!(save_draft(a, &foreign_receipt, &mut env).is_err());
    assert!(assert_balanced(b, &a.journals[0]).is_err());
}

// "quoted CSV, duplicate imports and equal amounts with distinct identity"
#[test]
fn quoted_csv_duplicate_imports_and_equal_amounts_with_distinct_identity() {
    let (b, mut env) = fresh();
    let account = b.accounts[0].id.clone();
    let rows = parse_csv(
        "date,description,amount,currency,reference\r\n2026-10-20,\"Tools, Inc.\",-42.50,USD,R1\r\n2026-10-21,\"Tools, Inc.\",-42.50,USD,R2\r\n",
    )
    .unwrap();
    let options = ImportOptions::default();
    let preview = preview_import(
        &b,
        &rows,
        &payment_map(),
        &account,
        SignConvention::Negative,
        &options,
        &mut env,
    )
    .unwrap();
    assert!(preview.iter().all(|r| r.error.is_none()));
    let next = commit_import(&b, &preview, &account, &mut env).unwrap();
    assert_eq!(next.payments.len(), b.payments.len() + 2);
    let repeated = commit_import(&next, &preview, &account, &mut env).unwrap();
    assert_eq!(repeated.drafts.len(), next.drafts.len());
    let re_preview = preview_import(
        &next,
        &rows,
        &payment_map(),
        &account,
        SignConvention::Negative,
        &options,
        &mut env,
    )
    .unwrap();
    assert!(re_preview.iter().all(|r| r.duplicate));
    assert!(parse_csv("a,b\n\"unterminated").is_err());
}

// "malformed dates, income, mismatched currencies and conflicting references are rejected"
#[test]
fn malformed_dates_income_mismatched_currencies_and_conflicting_references_are_rejected() {
    let (b, mut env) = fresh();
    let account = b.accounts[0].id.clone();
    let header = strings(&["date", "description", "amount", "currency", "reference"]);
    let preview = |b: &BusinessData, rows: &[Vec<String>], env: &mut SequentialEnv| {
        preview_import(
            b,
            rows,
            &payment_map(),
            &account,
            SignConvention::Negative,
            &ImportOptions::default(),
            env,
        )
        .unwrap()
    };
    for r in [
        ["2026-02-30", "A", "-1", "USD", "X"],
        ["2026-10-01", "A", "10", "USD", "Y"],
        ["2026-10-01", "A", "-10", "EUR", "Z"],
    ] {
        let rows = vec![header.clone(), strings(&r)];
        assert!(preview(&b, &rows, &mut env)[0].error.is_some());
    }
    let mut rows = vec![
        header.clone(),
        strings(&["2026-10-01", "Vendor", "-10", "USD", "ABC"]),
    ];
    let first = preview(&b, &rows, &mut env);
    let next = commit_import(&b, &first, &account, &mut env).unwrap();
    rows[1][2] = "-20".into();
    let conflict = preview(&next, &rows, &mut env);
    assert!(conflict[0].error.as_ref().unwrap().contains("conflicting"));
    assert!(commit_import(&next, &conflict, &account, &mut env).is_err());
}

// "attaching a receipt to a payment does not create another expense"
#[test]
fn attaching_a_receipt_to_a_payment_does_not_create_another_expense() {
    let (b, mut env) = fresh();
    let d = b
        .drafts
        .iter()
        .find(|d| d.payment_id.is_some())
        .unwrap()
        .clone();
    let mut with_receipt = d.clone();
    with_receipt.document_ids.push(b.documents[0].id.clone());
    let next = save_draft(&b, &with_receipt, &mut env).unwrap();
    assert_eq!(next.drafts.len(), b.drafts.len());
    assert_eq!(next.payments.len(), b.payments.len());
    let posted = approve(&next, &d.id, &mut env).unwrap();
    assert_eq!(posted.journals.len(), b.journals.len() + 1);
    let mut changed = d.clone();
    changed.amount_minor = d.amount_minor.checked_add(minor(1)).unwrap();
    assert!(validation(&b, &changed).contains(
        &"Expense must match its imported payment amount, date, currency, and account.".to_string()
    ));
    let mut replacement = d.replacement(env.new_id());
    replacement.amount_minor = changed.amount_minor;
    assert!(err_text(correct(
        &posted,
        &d.id,
        &replacement,
        "Changed receipt total",
        &mut env
    ))
    .contains("Expense must match its imported payment"));
    assert_eq!(posted.payments, b.payments);
    let stored = posted.drafts.iter().find(|x| x.id == d.id).unwrap();
    assert_eq!(stored.status(), ExpenseStatus::Posted);
}

// "conflicting references in one file are rejected without partial import"
#[test]
fn conflicting_references_in_one_file_are_rejected_without_partial_import() {
    let (b, mut env) = fresh();
    let rows = parse_csv(
        "date,description,amount,currency,reference\n2026-10-20,Tools,-42.50,USD,REF-1\n2026-10-21,Tools,-55.00,USD,REF-1",
    )
    .unwrap();
    let preview = preview_import(
        &b,
        &rows,
        &payment_map(),
        &b.accounts[0].id,
        SignConvention::Negative,
        &ImportOptions::default(),
        &mut env,
    )
    .unwrap();
    assert!(preview[1].error.as_ref().unwrap().contains("conflicting"));
    assert!(commit_import(&b, &preview, &b.accounts[0].id, &mut env).is_err());
    assert_eq!(b.payments.len(), 1);
}

// ---------------------------------------------------------------------------
// src/domain/portability.test.ts
// ---------------------------------------------------------------------------

const HISTORY_CSV: &str = "Expense Date,Vendor Name,Description,Expense Amount,Currency Code,Expense Category,Reference Number,Business Purpose\n10/23/2026,Paper & Pine,Workshop materials,\"1,234.56\",USD,Office & supplies,QB-901,Client workshop";

fn all() -> ExportFilter {
    ExportFilter::default()
}

fn history_options(
    provider: Option<ImportProvider>,
    date_format: Option<DateFormat>,
) -> ImportOptions {
    ImportOptions {
        kind: Some(ImportKind::ExpenseHistory),
        provider,
        date_format,
        filename: None,
    }
}

fn history_preview(
    b: &BusinessData,
    csv: &str,
    options: &ImportOptions,
    env: &mut SequentialEnv,
) -> Vec<ImportRow> {
    let rows = parse_csv(csv).unwrap();
    preview_import(
        b,
        &rows,
        &guess_mapping(&rows[0]),
        &b.accounts[0].id,
        SignConvention::Positive,
        options,
        env,
    )
    .unwrap()
}

struct Migration {
    b: BusinessData,
    preview: Vec<ImportRow>,
    account: String,
    env: SequentialEnv,
}

fn migration() -> Migration {
    let (b, mut env) = fresh();
    let options = ImportOptions {
        filename: Some("expenses.csv".into()),
        ..history_options(Some(ImportProvider::Quickbooks), Some(DateFormat::Mdy))
    };
    let preview = history_preview(&b, HISTORY_CSV, &options, &mut env);
    let account = b.accounts[0].id.clone();
    Migration {
        b,
        preview,
        account,
        env,
    }
}

fn column(rows: &[Vec<String>], name: &str) -> usize {
    rows[0].iter().position(|h| h == name).unwrap()
}

// "exports current expenses once, excludes drafts/transfers and keeps corrected history in the journal"
#[test]
fn exports_current_expenses_once_excludes_drafts_transfers_and_keeps_corrected_history_in_the_journal(
) {
    let (b, _) = fresh();
    let csv = export_records(&b, ExportFormat::Expenses, &all()).unwrap();
    let rows = parse_csv(&csv.csv).unwrap();
    let amount = column(&rows, "Amount");
    let sum: i64 = rows[1..]
        .iter()
        .map(|r| parse_money(&r[amount]).unwrap().get())
        .sum();
    assert_eq!(sum, usd(&b));
    assert_eq!(csv.rows.len(), 6);
    assert_eq!(
        csv.rows.iter().filter(|r| r[4] == "Atlas Creative").count(),
        1
    );
    let journal = export_records(&b, ExportFormat::Journal, &all()).unwrap();
    assert!(journal
        .rows
        .iter()
        .any(|r| r[4] == "reversal" && !r[6].is_empty()));
    assert!(journal.rows.iter().any(|r| r[4] == "transfer"));
    for currency in ["USD", "EUR", "GBP"] {
        let rows: Vec<_> = journal.rows.iter().filter(|r| r[12] == currency).collect();
        let total =
            |i: usize| -> i64 { rows.iter().map(|r| parse_money(&r[i]).unwrap().get()).sum() };
        assert_eq!(total(10), total(11));
    }
}

// "exports only the selected business, dates, and currency"
#[test]
fn exports_only_the_selected_business_dates_and_currency() {
    let (businesses, mut env) = seed();
    let (b, other) = (&businesses[0], &businesses[1]);
    let mut d = blank_draft(b, &mut env);
    d.vendor = "Euro vendor".into();
    d.description = "Service".into();
    d.purpose = "Client work".into();
    d.category_id = b.categories[0].id.clone();
    d.account_id = b
        .accounts
        .iter()
        .find(|a| a.currency == Currency::Eur)
        .unwrap()
        .id
        .clone();
    d.currency = Currency::Eur;
    d.amount_minor = minor(1234);
    let saved = save_draft(b, &d, &mut env).unwrap();
    let next = approve(&saved, &d.id, &mut env).unwrap();
    let eur = ExportFilter {
        currency: Some(Currency::Eur),
        ..all()
    };
    let rows = export_records(&next, ExportFormat::Expenses, &eur)
        .unwrap()
        .rows;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][6], "12.34");
    let mut mixed = next.clone();
    mixed.drafts.extend(other.drafts.iter().cloned());
    mixed.journals.extend(other.journals.iter().cloned());
    for format in [ExportFormat::Expenses, ExportFormat::Journal] {
        let rows = export_records(&mixed, format, &all()).unwrap().rows;
        assert!(rows.iter().all(|r| r[0] == b.profile.id));
    }
    let empty = ExportFilter {
        from: "1900-01-01".into(),
        to: "1900-02-01".into(),
        ..all()
    };
    assert!(export_records(b, ExportFormat::Expenses, &empty)
        .unwrap()
        .rows
        .is_empty());
    let invalid = ExportFilter {
        from: "2026-02-30".into(),
        ..all()
    };
    assert!(export_records(b, ExportFormat::Expenses, &invalid).is_err());
    let reversed = ExportFilter {
        from: "2026-12-31".into(),
        to: "2026-01-01".into(),
        ..all()
    };
    assert!(export_records(b, ExportFormat::Expenses, &reversed).is_err());
}

// "CSV quotes delimiters and prevents spreadsheet formulas in user-controlled text"
#[test]
fn csv_quotes_delimiters_and_prevents_spreadsheet_formulas_in_user_controlled_text() {
    let (mut b, _) = fresh();
    let d = b
        .drafts
        .iter_mut()
        .find(|d| d.status() == ExpenseStatus::Posted)
        .unwrap();
    d.vendor = "=HYPERLINK(\"https://invalid.example\", \"open\")".into();
    d.description = "Quoted \"receipt\", with\nnewline".into();
    let d = d.clone();
    let rows = parse_csv(
        &export_records(&b, ExportFormat::Expenses, &all())
            .unwrap()
            .csv,
    )
    .unwrap();
    let row = rows.iter().find(|r| r[2] == d.id).unwrap();
    assert_eq!(row[4], format!("'{}", d.vendor));
    assert_eq!(row[5], d.description);
}

// "maps common export columns, explicit regional dates, category and exact minor units without claiming bank evidence"
#[test]
fn maps_common_export_columns_explicit_regional_dates_category_and_exact_minor_units_without_claiming_bank_evidence(
) {
    let Migration {
        b,
        preview,
        account,
        mut env,
    } = migration();
    assert_eq!(preview[0].error, None);
    assert_eq!(preview[0].date, "2026-10-23");
    assert_eq!(preview[0].amount_minor, minor(123456));
    let next = commit_import(&b, &preview, &account, &mut env).unwrap();
    let draft = next.drafts.last().unwrap();
    assert_eq!(draft.vendor, "Paper & Pine");
    assert_eq!(draft.category_id, b.categories[1].id);
    assert_eq!(draft.purpose, "Client workshop");
    assert_eq!(draft.status(), ExpenseStatus::Draft);
    assert_eq!(draft.payment_id, None);
    assert_eq!(next.payments.len(), b.payments.len());
    assert_eq!(next.journals, b.journals);
    assert_eq!(
        draft.import_source.as_ref().unwrap().provider,
        ImportProvider::Quickbooks
    );
}

// "repeat import stays safe after edits, posting and correction, and original source remains immutable"
#[test]
fn repeat_import_stays_safe_after_edits_posting_and_correction_and_original_source_remains_immutable(
) {
    let Migration {
        b,
        preview,
        account,
        mut env,
    } = migration();
    let initial = commit_import(&b, &preview, &account, &mut env).unwrap();
    let d = initial.drafts.last().unwrap().clone();
    let mut edited = d.clone();
    edited.vendor = "Edited vendor".into();
    let mut next = save_draft(&initial, &edited, &mut env).unwrap();
    next = approve(&next, &d.id, &mut env).unwrap();
    let original = next.drafts.iter().find(|x| x.id == d.id).unwrap().clone();
    let mut replacement = original.replacement(env.new_id());
    replacement.amount_minor = minor(123000);
    next = correct(
        &next,
        &d.id,
        &replacement,
        "Corrected expense total",
        &mut env,
    )
    .unwrap();
    assert_eq!(
        commit_import(&next, &preview, &account, &mut env)
            .unwrap()
            .drafts,
        next.drafts
    );
    let mut tampered = d.clone();
    tampered.import_source.as_mut().unwrap().reference = "different".into();
    assert!(err_text(save_draft(&initial, &tampered, &mut env)).contains("cannot be changed"));
    let mut duplicate = d.replacement(env.new_id());
    duplicate.vendor = "Unrelated vendor".into();
    assert!(err_text(save_draft(&next, &duplicate, &mut env))
        .contains("Imported source evidence can only be added by an import."));
    // The TypeScript test also approves this duplicate through `writeDraft`'s
    // import path, which is crate-private here; the parity fixture
    // `history/approve: import source already posted` covers that check.
}

// "unknown categories stay unresolved, conflicts and unsupported document types block import"
#[test]
fn unknown_categories_stay_unresolved_conflicts_and_unsupported_document_types_block_import() {
    let Migration {
        b,
        account,
        mut env,
        ..
    } = migration();
    let options = history_options(None, Some(DateFormat::Mdy));
    let legacy = HISTORY_CSV.replace("Office & supplies", "Legacy category");
    let preview = history_preview(&b, &legacy, &options, &mut env);
    assert_eq!(preview[0].category_id, None);
    assert!(preview[0]
        .warnings
        .as_ref()
        .unwrap()
        .join(" ")
        .contains("Legacy category"));
    let unresolved = commit_import(&b, &preview, &account, &mut env).unwrap();
    let last = unresolved.drafts.last().unwrap().id.clone();
    assert!(err_text(approve(&unresolved, &last, &mut env)).contains("Category is required"));
    let zoho = history_options(Some(ImportProvider::Zoho), Some(DateFormat::Dmy));
    let preview = history_preview(
        &b,
        "Date,Name,Memo/Description,Amount,Transaction Type,Num\n23/10/2026,Vendor,Old bill,12.00,Bill,Z1",
        &zoho,
        &mut env,
    );
    assert!(preview[0]
        .error
        .as_ref()
        .unwrap()
        .contains("Only paid expense"));
    let repeated = format!(
        "{HISTORY_CSV}\n10/23/2026,Paper & Pine,Workshop materials,500.00,USD,Office & supplies,QB-901,Client workshop"
    );
    let preview = history_preview(&b, &repeated, &options, &mut env);
    assert!(preview[1].error.as_ref().unwrap().contains("conflicting"));
}

// "source evidence and approval remain business scoped"
#[test]
fn source_evidence_and_approval_remain_business_scoped() {
    let Migration {
        b,
        preview,
        account,
        mut env,
    } = migration();
    let other = seed().0.remove(1);
    assert!(err_text(commit_import(
        &other,
        &preview,
        &other.accounts[0].id,
        &mut env
    ))
    .contains("Import rows were previewed for a different business or account"));
    let next = commit_import(&b, &preview, &account, &mut env).unwrap();
    let mut moved = next.drafts.last().unwrap().clone();
    moved.business_id = other.profile.id.clone();
    moved.account_id = other.accounts[0].id.clone();
    moved.category_id = other.categories[0].id.clone();
    assert!(err_text(save_draft(&other, &moved, &mut env)).contains("another business"));
}

// "our expense CSV maps back into reviewable drafts with preserved exact amounts"
#[test]
fn our_expense_csv_maps_back_into_reviewable_drafts_with_preserved_exact_amounts() {
    let (b, mut env) = fresh();
    let csv = export_records(&b, ExportFormat::Expenses, &all()).unwrap();
    let preview = history_preview(&b, &csv.csv, &history_options(None, None), &mut env);
    assert!(preview.iter().all(|r| r.error.is_none()));
    let sum: i64 = preview.iter().map(|r| r.amount_minor.get()).sum();
    assert_eq!(sum, 458559);
}

// "a mixed-account migration is blocked before any drafts are created"
#[test]
fn a_mixed_account_migration_is_blocked_before_any_drafts_are_created() {
    let (b, mut env) = fresh();
    let preview = history_preview(
        &b,
        "Date,Vendor,Description,Amount,Currency,Payment account,Reference\n2026-10-01,A,Materials,12.00,USD,Checking,A1\n2026-10-02,B,Service,15.00,USD,Credit card,B1",
        &history_options(None, None),
        &mut env,
    );
    assert!(preview.iter().all(|r| r
        .error
        .as_ref()
        .is_some_and(|e| e.contains("multiple payment accounts"))));
    assert!(
        err_text(commit_import(&b, &preview, &b.accounts[0].id, &mut env))
            .contains("Resolve invalid")
    );
    assert_eq!(b.drafts.len(), fresh().0.drafts.len());
}
