//! Record changes from real `ledger-core` commands: rebuilding from them,
//! auditing by re-running with the recorded environment, and the `baseSeq`
//! conflict.

mod common;

use common::*;
use ledger_core::{approve, save_draft, BusinessData, ExpenseStatus};
use protocol::{
    apply_changes, check_base_seq, diff, targets, Command, ProtocolError, Record, RecordChange,
    RecordKey, RecordType, RecordingEnv, ReplayEnv, CHANGED_SINCE_OPENED,
};

const NOW: &str = "2026-05-06T07:08:09.000Z";

fn generator() -> impl FnMut() -> String {
    let mut n = 0;
    move || {
        n += 1;
        format!("svc-{n:04}")
    }
}

/// The first seed draft that approves cleanly.
fn approvable(b: &BusinessData) -> String {
    b.drafts
        .iter()
        .filter(|d| d.status() == ExpenseStatus::Draft)
        .find(|d| {
            approve(
                b,
                &d.id,
                &mut ledger_core::SequentialEnv::new(1, NOW, "2026-05-06"),
            )
            .is_ok()
        })
        .map(|d| d.id.clone())
        .unwrap()
}

#[test]
fn approval_changes_rebuild_the_state_and_audit_exactly() {
    let (before, _) = seed_business();
    let id = approvable(&before);

    let mut env = RecordingEnv::new(generator(), NOW);
    let after = approve(&before, &id, &mut env).unwrap();
    let record = env.into_record();
    assert!(!record.ids.is_empty());

    let changes = diff(&before, &after).unwrap();
    let types: Vec<RecordType> = changes.iter().map(|c| c.key().record_type).collect();
    assert!(types.contains(&RecordType::Draft));
    assert!(types.contains(&RecordType::Journal));
    assert!(types.contains(&RecordType::Approval));

    // Rebuild: the changes alone, through JSON, reproduce the new state.
    let wire = serde_json::to_string(&changes).unwrap();
    let read: Vec<RecordChange> = serde_json::from_str(&wire).unwrap();
    assert_eq!(apply_changes(&before, &read).unwrap(), after);

    // Audit: re-running with the recorded ids and time gives the same changes.
    let mut replay = ReplayEnv::new(&record);
    let again = approve(&before, &id, &mut replay).unwrap();
    replay.finish().unwrap();
    assert_eq!(diff(&before, &again).unwrap(), changes);
}

#[test]
fn a_chain_of_changes_rebuilds_from_the_start() {
    let (start, _) = seed_business();
    let id = approvable(&start);
    let mut draft = start.drafts.iter().find(|d| d.id == id).unwrap().clone();
    draft.purpose = "Invented purpose".into();

    let mut env = RecordingEnv::new(generator(), NOW);
    let saved = save_draft(&start, &draft, &mut env).unwrap();
    let posted = approve(&saved, &id, &mut env).unwrap();
    let steps = [
        diff(&start, &saved).unwrap(),
        diff(&saved, &posted).unwrap(),
    ];

    let mut rebuilt = start.clone();
    for step in &steps {
        rebuilt = apply_changes(&rebuilt, step).unwrap();
    }
    assert_eq!(rebuilt, posted);
    assert_eq!(
        serde_json::to_value(&rebuilt).unwrap(),
        serde_json::to_value(&posted).unwrap()
    );
}

#[test]
fn a_removed_record_cannot_be_expressed() {
    let (before, _) = seed_business();
    let mut after = before.clone();
    after.rules.clear();
    assert!(!before.rules.is_empty());
    assert_eq!(
        diff(&before, &after).unwrap_err(),
        ProtocolError::RecordRemoved("rule")
    );
}

#[test]
fn a_reordering_cannot_be_expressed() {
    let (before, _) = seed_business();
    let mut after = before.clone();
    assert!(after.drafts.len() > 1);
    after.drafts.reverse();
    assert_eq!(
        diff(&before, &after).unwrap_err(),
        ProtocolError::ChangesIncomplete
    );
}

#[test]
fn an_unbalanced_journal_change_is_refused() {
    let (before, _) = seed_business();
    let mut journal = before.journals[0].to_unchecked();
    journal.id = "j-new".into();
    journal.lines.pop();
    let err = apply_changes(&before, &[Record::Journal(journal)]).unwrap_err();
    assert!(matches!(err, ProtocolError::BadChange(_)));
}

#[test]
fn the_changed_since_you_opened_it_conflict() {
    let (b, _) = seed_business();
    let id = approvable(&b);
    let command = Command::Approve {
        expense_id: id.clone(),
    };
    let t = targets(&command, &b);
    assert_eq!(t, vec![RecordKey::new(RecordType::Draft, id.clone())]);

    // Two people open the draft at seq 7. The first approval is accepted
    // at seq 8 and changes the draft; the second, still based on 7, is stale.
    let last_changed = |k: &RecordKey| (k.id == id).then_some(8);
    let err = check_base_seq(7, 8, &t, last_changed).unwrap_err();
    assert_eq!(err, ProtocolError::Changed);
    assert_eq!(err.to_string(), CHANGED_SINCE_OPENED);
    // Someone who has seen seq 8 is not.
    assert!(check_base_seq(8, 8, &t, last_changed).is_ok());
}
