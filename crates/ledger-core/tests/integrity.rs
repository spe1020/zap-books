//! Regression tests for the PR audit's ledger boundary failures.
mod common;
use common::{fresh, seed};
use ledger_core::*;
use serde_json::json;

fn preview(
    b: &BusinessData,
    account: &str,
    history: bool,
    reference: &str,
    env: &mut SequentialEnv,
) -> Vec<ImportRow> {
    let rows = parse_csv(&format!("date,description,amount,currency,reference\n2026-10-24,Audit Vendor,-42.50,USD,{reference}")).unwrap();
    preview_import(
        b,
        &rows,
        &guess_mapping(&rows[0]),
        account,
        SignConvention::Negative,
        &ImportOptions {
            kind: history.then_some(ImportKind::ExpenseHistory),
            ..Default::default()
        },
        env,
    )
    .unwrap()
}

#[test]
fn forged_correction_cannot_post_payment_or_import_source_twice() {
    for history in [false, true] {
        let (b, mut env) = fresh();
        let rows = preview(&b, &b.accounts[0].id, history, "AUDIT-ONE", &mut env);
        let imported = commit_import(&b, &rows, &b.accounts[0].id, &mut env).unwrap();
        let mut draft = imported.drafts.last().unwrap().clone();
        draft.category_id = b.categories[0].id.clone();
        draft.purpose = "Client work".into();
        let saved = save_draft(&imported, &draft, &mut env).unwrap();
        let posted = approve(&saved, &draft.id, &mut env).unwrap();
        let original = posted.drafts.iter().find(|d| d.id == draft.id).unwrap();
        let mut replacement = original.replacement(env.new_id());
        replacement.correction_of = Some(original.id.clone());
        let before = posted.clone();
        // A manual save cannot attach existing evidence to a new draft. The
        // approve checks behind it are covered by the parity fixtures
        // `integrity/*: forged correction`.
        assert_eq!(
            save_draft(&posted, &replacement, &mut env),
            Err(if history {
                LedgerError::ImportSourceNotFromImport
            } else {
                LedgerError::PaymentNotFromImport
            })
        );
        assert_eq!(posted, before);
        let corrected = correct(
            &posted,
            &original.id,
            &replacement,
            "Clarify purchase",
            &mut env,
        )
        .unwrap();
        assert_eq!(
            corrected
                .drafts
                .iter()
                .find(|d| d.id == original.id)
                .unwrap()
                .status(),
            ExpenseStatus::Corrected
        );
        assert_eq!(
            corrected
                .drafts
                .iter()
                .find(|d| d.id == replacement.id)
                .unwrap()
                .status(),
            ExpenseStatus::Posted
        );
        assert_eq!(
            corrected
                .journals
                .iter()
                .filter(|j| j.reverses_id == original.journal_id)
                .count(),
            1
        );
    }
}

#[test]
fn previews_bind_destination_before_deduplication_and_import_is_atomic() {
    for history in [false, true] {
        for reference in ["AUDIT-ONE", ""] {
            let (businesses, mut env) = seed();
            let (b, other) = (&businesses[0], &businesses[1]);
            let (from, to) = (&b.accounts[0].id, &b.accounts[1].id);
            let rows = preview(b, from, history, reference, &mut env);
            let before = b.clone();
            assert_eq!(
                commit_import(b, &rows, to, &mut env),
                Err(LedgerError::PreviewedElsewhere)
            );
            assert_eq!(
                commit_import(other, &rows, &other.accounts[0].id, &mut env),
                Err(LedgerError::PreviewedElsewhere)
            );
            let once = commit_import(b, &rows, from, &mut env).unwrap();
            assert_eq!(
                commit_import(&once, &rows, to, &mut env),
                Err(LedgerError::PreviewedElsewhere)
            );
            assert_eq!(commit_import(&once, &rows, from, &mut env).unwrap(), once);
            let destination = preview(b, to, history, reference, &mut env);
            let mut mixed = destination.clone();
            mixed.extend(rows);
            assert_eq!(
                commit_import(b, &mixed, to, &mut env),
                Err(LedgerError::PreviewedElsewhere)
            );
            assert_eq!(*b, before);
            let imported = commit_import(b, &destination, to, &mut env).unwrap();
            assert_eq!(imported.drafts.len(), b.drafts.len() + 1);
            let repeated = preview(&imported, to, history, reference, &mut env);
            assert!(repeated[0].duplicate);
            assert_eq!(
                commit_import(&imported, &repeated, to, &mut env).unwrap(),
                imported
            );
        }
    }
}

#[test]
fn forged_fingerprint_is_rejected_even_in_scope() {
    for history in [false, true] {
        let (b, mut env) = fresh();
        let mut rows = preview(&b, &b.accounts[0].id, history, "", &mut env);
        let forged = r#"["northstar","elsewhere"]"#.to_string();
        if let Some(source) = rows[0].import_source.as_mut() {
            source.fingerprint = forged.clone();
        }
        rows[0].fingerprint = forged;
        assert_eq!(
            commit_import(&b, &rows, &b.accounts[0].id, &mut env),
            Err(LedgerError::InvalidImportScope)
        );
    }
}

#[test]
fn repeated_history_rows_still_validate_source_ownership() {
    let (b, mut env) = fresh();
    let mut rows = preview(&b, &b.accounts[0].id, true, "AUDIT-ONE", &mut env);
    let once = commit_import(&b, &rows, &b.accounts[0].id, &mut env).unwrap();
    rows[0].import_source.as_mut().unwrap().business_id = "fieldwork".into();
    assert_eq!(
        commit_import(&once, &rows, &b.accounts[0].id, &mut env),
        Err(LedgerError::InvalidImportSource)
    );
}

#[test]
fn stored_journals_validate_business_accounts_categories_and_currency() {
    let (b, _) = fresh();
    let raw = serde_json::to_value(&b).unwrap();
    let payment_line = b.journals[0]
        .lines
        .iter()
        .position(|l| l.category_id.is_none())
        .unwrap();
    let category_line = b.journals[0]
        .lines
        .iter()
        .position(|l| l.category_id.is_some())
        .unwrap();
    for (pointer, value, message) in [
        (
            format!("/journals/0/lines/{payment_line}/accountId"),
            json!("northstar-eur"),
            "Invalid journal account or currency.",
        ),
        (
            format!("/journals/0/lines/{payment_line}/accountId"),
            json!("missing"),
            "Invalid journal account or currency.",
        ),
        (
            format!("/journals/0/lines/{category_line}/categoryId"),
            json!("missing"),
            "Invalid journal category.",
        ),
        (
            "/journals/0/businessId".into(),
            json!("fieldwork"),
            "Invalid journal context.",
        ),
    ] {
        let mut invalid = raw.clone();
        *invalid.pointer_mut(&pointer).unwrap() = value;
        let entry: UncheckedJournal =
            serde_json::from_value(invalid["journals"][0].clone()).unwrap();
        assert_eq!(
            JournalEntry::new(&b, entry).unwrap_err().to_string(),
            message
        );
        assert!(serde_json::from_value::<BusinessData>(invalid.clone())
            .unwrap_err()
            .to_string()
            .contains(message));
        let workspace = json!({"schemaVersion":1,"revision":0,"businesses":[raw.clone(),invalid]});
        assert!(serde_json::from_value::<Workspace>(workspace)
            .unwrap_err()
            .to_string()
            .contains(message));
    }
    let loaded: BusinessData = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(serde_json::to_value(loaded).unwrap(), raw);
}

// W12: a correction hands the payment to the replacement, and approve only
// posts a draft that its payment records.
#[test]
fn corrections_hand_the_payment_to_the_replacement() {
    let (b, mut env) = fresh();
    let account = b.accounts[0].id.clone();
    let rows = preview(&b, &account, false, "W12", &mut env);
    let imported = commit_import(&b, &rows, &account, &mut env).unwrap();
    let mut draft = imported.drafts.last().unwrap().clone();
    let payment_id = draft.payment_id.clone().unwrap();
    draft.category_id = b.categories[0].id.clone();
    draft.purpose = "Client work".into();
    let saved = save_draft(&imported, &draft, &mut env).unwrap();
    let posted = approve(&saved, &draft.id, &mut env).unwrap();
    let original = posted.drafts.iter().find(|d| d.id == draft.id).unwrap();
    let mut replacement = original.replacement(env.new_id());
    replacement.description = "Corrected".into();
    let corrected = correct(&posted, &original.id, &replacement, "Clarify", &mut env).unwrap();
    let owner = |b: &BusinessData| {
        b.payments
            .iter()
            .find(|p| p.id == payment_id)
            .unwrap()
            .draft_id
            .clone()
    };
    assert_eq!(owner(&corrected), replacement.id);

    let mut handed_off = saved.clone();
    for p in &mut handed_off.payments {
        if p.id == payment_id {
            p.draft_id = "someone-else".into();
        }
    }
    assert_eq!(
        approve(&handed_off, &draft.id, &mut env),
        Err(LedgerError::PaymentOfOtherExpense)
    );
}
