//! Record changes from real `ledger-core` commands: rebuilding from them,
//! auditing by re-running with the recorded environment, and the `baseSeq`
//! conflict.

mod common;

use common::*;
use ledger_core::{approve, save_draft, save_rule, BusinessData, ExpenseStatus, RuleInput};
use protocol::{
    apply_changes, check_base_seq, diff, targets, Command, ProtocolError, Record, RecordChange,
    RecordKey, RecordType, RecordingEnv, Removal, Reordering, ReplayEnv, CHANGED_SINCE_OPENED,
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
    let types: Vec<RecordType> = changes
        .iter()
        .filter_map(|c| c.key())
        .map(|k| k.record_type)
        .collect();
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
fn a_reordering_is_recorded_and_replays() {
    let (before, _) = seed_business();
    let mut after = before.clone();
    assert!(after.drafts.len() > 1);
    after.drafts.reverse();
    let changes = diff(&before, &after).unwrap();
    assert_eq!(changes.len(), 1);
    assert!(matches!(&changes[0], RecordChange::Order(o) if o.of == RecordType::Draft));
    assert_eq!(apply_changes(&before, &changes).unwrap(), after);
}

#[test]
fn an_edited_rule_moves_to_the_end_through_its_changes() {
    let (b, _) = seed_business();
    let mut env = RecordingEnv::new(generator(), NOW);
    let input = |vendor: &str| RuleInput {
        id: None,
        vendor_includes: vendor.into(),
        category_id: b.categories[0].id.clone(),
        purpose: "Invented purpose".into(),
        enabled: true,
    };
    let two = save_rule(&b, &input("Invented Vendor A"), &mut env).unwrap();
    let three = save_rule(&two, &input("Invented Vendor B"), &mut env).unwrap();
    let first = three.rules[0].clone();
    let edited = save_rule(
        &three,
        &RuleInput {
            id: Some(first.id.clone()),
            enabled: false,
            ..input(&first.vendor_includes)
        },
        &mut env,
    )
    .unwrap();
    assert_eq!(edited.rules.last().unwrap().id, first.id);
    let changes = diff(&three, &edited).unwrap();
    assert!(changes
        .iter()
        .any(|c| matches!(c, RecordChange::Order(o) if o.of == RecordType::Rule)));
    assert_eq!(apply_changes(&three, &changes).unwrap(), edited);
}

#[test]
fn only_extraction_records_can_be_removed() {
    let (before, _) = seed_business();
    let mut after = before.clone();
    after.drafts.pop();
    assert_eq!(
        diff(&before, &after).unwrap_err(),
        ProtocolError::RecordRemoved("draft")
    );
    let removal = RecordChange::Remove(Removal {
        of: RecordType::Draft,
        id: before.drafts[0].id.clone(),
    });
    assert_eq!(
        apply_changes(&before, &[removal]).unwrap_err(),
        ProtocolError::RecordRemoved("draft")
    );
}

#[test]
fn a_reordering_must_name_every_record_once() {
    let (before, _) = seed_business();
    let ids: Vec<String> = before.drafts.iter().map(|d| d.id.clone()).collect();
    for bad in [
        ids[1..].to_vec(),
        [ids[..ids.len() - 1].to_vec(), vec![ids[0].clone()]].concat(),
        [ids[1..].to_vec(), vec!["missing".into()]].concat(),
    ] {
        let change = RecordChange::Order(Reordering {
            of: RecordType::Draft,
            ids: bad,
        });
        assert!(matches!(
            apply_changes(&before, &[change]).unwrap_err(),
            ProtocolError::BadChange(_)
        ));
    }
}

#[test]
fn record_changes_have_their_documented_json() {
    let remove = RecordChange::Remove(Removal {
        of: RecordType::Extraction,
        id: "x1".into(),
    });
    let order = RecordChange::Order(Reordering {
        of: RecordType::Rule,
        ids: vec!["r2".into(), "r1".into()],
    });
    assert_eq!(
        serde_json::to_value(&remove).unwrap(),
        serde_json::json!({"type": "remove", "of": "extraction", "id": "x1"})
    );
    assert_eq!(
        serde_json::to_value(&order).unwrap(),
        serde_json::json!({"type": "order", "of": "rule", "ids": ["r2", "r1"]})
    );
    for change in [remove, order] {
        let text = serde_json::to_string(&change).unwrap();
        assert_eq!(serde_json::from_str::<RecordChange>(&text).unwrap(), change);
    }
}

#[test]
fn an_unbalanced_journal_change_is_refused() {
    let (before, _) = seed_business();
    let mut journal = before.journals[0].to_unchecked();
    journal.id = "j-new".into();
    journal.lines.pop();
    let err = apply_changes(&before, &[RecordChange::Put(Record::Journal(journal))]).unwrap_err();
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
