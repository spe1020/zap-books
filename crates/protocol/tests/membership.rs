//! Membership and permission rules, with an injected clock.

mod common;

use common::*;
use nostr::prelude::{EventId, Keys};
use protocol::{
    Actor, Command, History, MembershipChange, MembershipState, Outcome, ProtocolError, Recovery,
    Role, RuleInput, SettingsChange, OWNER_DELAY_SECS,
};

const T0: u64 = 1_800_000_000;

fn id(n: u8) -> EventId {
    EventId::from_slice(&[n; 32]).unwrap()
}

fn add(keys: &Keys, role: Role, history: History) -> MembershipChange {
    MembershipChange::Add {
        member: pk(keys),
        role,
        history,
    }
}

/// Owner plus backup key, with a bookkeeper, a viewer and a submitter, all
/// with full history.
fn team() -> MembershipState {
    let mut s = MembershipState::genesis(
        business_id(),
        pk(&owner()),
        pk(&service()),
        backup_recovery(),
    )
    .unwrap();
    for (n, (keys, role)) in [
        (bookkeeper(), Role::Bookkeeper),
        (viewer(), Role::Viewer),
        (submitter(), Role::Submitter),
    ]
    .iter()
    .enumerate()
    {
        let u = s
            .propose(
                id(n as u8 + 10),
                &pk(&owner()),
                &add(keys, *role, History::Full),
                T0,
            )
            .unwrap();
        assert_eq!(u.outcome, Outcome::Applied);
        s = u.state;
    }
    s
}

fn every_command() -> Vec<(&'static str, Command)> {
    let (b, _) = seed_business();
    let draft = b.drafts[0].clone();
    vec![
        (
            "SaveDraft",
            Command::SaveDraft {
                draft: draft.clone(),
                documents: vec![],
                extraction: None,
            },
        ),
        (
            "Approve",
            Command::Approve {
                expense_id: draft.id.clone(),
            },
        ),
        (
            "Correct",
            Command::Correct {
                original_id: draft.id.clone(),
                replacement: draft.clone(),
                reason: "Invented reason".into(),
            },
        ),
        (
            "CommitImport",
            Command::CommitImport {
                account_id: "a".into(),
                rows: vec![],
            },
        ),
        (
            "UpdateSettings",
            Command::UpdateSettings {
                change: SettingsChange::Profile {
                    name: "N".into(),
                    industry: "I".into(),
                },
            },
        ),
        (
            "SaveRule",
            Command::SaveRule {
                rule: RuleInput {
                    id: None,
                    vendor_includes: "v".into(),
                    category_id: "c".into(),
                    purpose: "p".into(),
                    enabled: true,
                },
            },
        ),
        (
            "ChangeMembership",
            Command::ChangeMembership {
                change: add(&key(20), Role::Viewer, History::Full),
            },
        ),
        (
            "ChangeMembership (owner-level)",
            Command::ChangeMembership {
                change: add(&key(20), Role::Owner, History::Full),
            },
        ),
        (
            "Recover",
            Command::ChangeMembership {
                change: MembershipChange::Recover {
                    lost: pk(&owner()),
                    replacement: pk(&key(20)),
                },
            },
        ),
        ("Veto", Command::Veto { pending: id(1) }),
        (
            "SubmitReceiptDraft",
            Command::SubmitReceiptDraft {
                draft,
                documents: vec![],
            },
        ),
    ]
}

#[test]
fn the_permission_table() {
    let s = team();
    // Columns: owner, bookkeeper, viewer, submitter, recovery key, outsider.
    let signers = [
        owner(),
        bookkeeper(),
        viewer(),
        submitter(),
        backup(),
        outsider(),
    ];
    let expected: [(&str, [bool; 6]); 11] = [
        ("SaveDraft", [true, true, false, false, false, false]),
        ("Approve", [true, true, false, false, false, false]),
        ("Correct", [true, true, false, false, false, false]),
        ("CommitImport", [true, true, false, false, false, false]),
        ("UpdateSettings", [true, true, false, false, false, false]),
        ("SaveRule", [true, true, false, false, false, false]),
        (
            "ChangeMembership",
            [true, false, false, false, false, false],
        ),
        (
            "ChangeMembership (owner-level)",
            [true, false, false, false, false, false],
        ),
        ("Recover", [false, false, false, false, true, false]),
        ("Veto", [true, false, false, false, true, false]),
        (
            "SubmitReceiptDraft",
            [true, true, false, true, false, false],
        ),
    ];
    for ((name, command), (row, allowed)) in every_command().iter().zip(expected) {
        assert_eq!(*name, row);
        for (signer, ok) in signers.iter().zip(allowed) {
            let result = s.authorize(&pk(signer), command);
            assert_eq!(result.is_ok(), ok, "{name} by {:?}", s.actor(&pk(signer)));
            if pk(signer) == pk(&outsider()) {
                assert_eq!(result, Err(ProtocolError::NotMember));
            }
        }
    }
}

#[test]
fn a_removed_member_is_refused_at_once_and_the_epoch_rotates() {
    let s = team();
    assert_eq!(s.epoch, 0);
    let u = s
        .propose(
            id(1),
            &pk(&owner()),
            &MembershipChange::Remove {
                member: pk(&bookkeeper()),
            },
            T0,
        )
        .unwrap();
    assert_eq!(u.outcome, Outcome::Applied);
    assert!(u.new_epoch);
    assert_eq!(u.state.epoch, 1);
    assert_eq!(u.state.actor(&pk(&bookkeeper())), Actor::Outsider);
    assert_eq!(
        u.state.authorize(
            &pk(&bookkeeper()),
            &Command::Approve {
                expense_id: "x".into()
            }
        ),
        Err(ProtocolError::NotMember)
    );
    assert_eq!(u.state.readable_epochs(&pk(&bookkeeper())), None);
    assert!(!u.state.readers().any(|k| *k == pk(&bookkeeper())));
}

#[test]
fn history_choice_decides_which_epochs_a_new_member_reads() {
    let s = team();
    let full = s
        .propose(
            id(1),
            &pk(&owner()),
            &add(&key(21), Role::Viewer, History::Full),
            T0,
        )
        .unwrap();
    assert!(!full.new_epoch);
    assert_eq!(full.state.readable_epochs(&pk(&key(21))), Some(0..=0));

    let from_join = full
        .state
        .propose(
            id(2),
            &pk(&owner()),
            &add(&key(22), Role::Viewer, History::FromJoin),
            T0,
        )
        .unwrap();
    assert!(from_join.new_epoch);
    let s = from_join.state;
    assert_eq!(s.epoch, 1);
    assert_eq!(s.readable_epochs(&pk(&key(22))), Some(1..=1));
    assert_eq!(s.readable_epochs(&pk(&key(21))), Some(0..=1));
}

#[test]
fn a_submitter_receives_no_epoch_keys() {
    let s = team();
    assert_eq!(s.readable_epochs(&pk(&submitter())), None);
    assert!(!s.readers().any(|k| *k == pk(&submitter())));
    assert_eq!(s.readable_epochs(&pk(&backup())), None);

    // Demoting a reader to submitter is a removal for the keys.
    let u = s
        .propose(
            id(1),
            &pk(&owner()),
            &MembershipChange::ChangeRole {
                member: pk(&viewer()),
                role: Role::Submitter,
            },
            T0,
        )
        .unwrap();
    assert!(u.new_epoch);
    assert_eq!(u.state.readable_epochs(&pk(&viewer())), None);
}

#[test]
fn an_owner_level_change_waits_48_hours() {
    let s = team();
    let change = add(&key(23), Role::Owner, History::Full);
    let u = s.propose(id(1), &pk(&owner()), &change, T0).unwrap();
    assert_eq!(
        u.outcome,
        Outcome::Pending {
            effective_at: T0 + OWNER_DELAY_SECS
        }
    );
    assert!(!u.new_epoch);
    let s = u.state;
    assert_eq!(s.actor(&pk(&key(23))), Actor::Outsider);
    assert_eq!(s.pending.len(), 1);

    let just_before = T0 + OWNER_DELAY_SECS - 1;
    assert!(s.due(just_before).is_empty());
    assert_eq!(
        s.activate(&id(1), just_before).unwrap_err(),
        ProtocolError::NotDue
    );

    let due = T0 + OWNER_DELAY_SECS;
    assert_eq!(s.due(due), vec![id(1)]);
    let (active, _) = s.activate(&id(1), due).unwrap();
    assert_eq!(active.actor(&pk(&key(23))), Actor::Member(Role::Owner));
    assert!(active.pending.is_empty());
}

#[test]
fn which_changes_are_owner_level() {
    let s = team();
    let owner_level = [
        add(&key(23), Role::Owner, History::Full),
        MembershipChange::ChangeRole {
            member: pk(&viewer()),
            role: Role::Owner,
        },
        MembershipChange::ReplaceRecoveryKey {
            key: Some(pk(&key(24))),
        },
        MembershipChange::ReplaceServiceKey { key: pk(&key(25)) },
    ];
    for change in owner_level {
        assert!(s.is_owner_level(&change), "{change:?}");
    }
    let immediate = [
        add(&key(23), Role::Bookkeeper, History::Full),
        MembershipChange::Remove {
            member: pk(&viewer()),
        },
        MembershipChange::ChangeRole {
            member: pk(&viewer()),
            role: Role::Bookkeeper,
        },
    ];
    for change in immediate {
        assert!(!s.is_owner_level(&change), "{change:?}");
    }
}

#[test]
fn a_veto_from_another_owner_or_the_recovery_key_cancels() {
    // Two owners and a backup key.
    let s = team();
    let s = s
        .propose(
            id(1),
            &pk(&owner()),
            &add(&key(30), Role::Owner, History::Full),
            T0,
        )
        .unwrap()
        .state;
    let s = s.activate(&id(1), T0 + OWNER_DELAY_SECS).unwrap().0;

    let change = MembershipChange::ReplaceRecoveryKey {
        key: Some(pk(&key(31))),
    };
    let s = s.propose(id(2), &pk(&owner()), &change, T0).unwrap().state;

    // The proposer can't veto their own change; others can.
    assert_eq!(
        s.veto(&pk(&owner()), &id(2)).unwrap_err(),
        ProtocolError::OwnVeto
    );
    assert_eq!(
        s.veto(&pk(&bookkeeper()), &id(2)).unwrap_err(),
        ProtocolError::Forbidden
    );
    for vetoer in [key(30), backup()] {
        let vetoed = s.veto(&pk(&vetoer), &id(2)).unwrap();
        assert!(vetoed.pending.is_empty());
        assert!(vetoed.due(T0 + OWNER_DELAY_SECS).is_empty());
        assert_eq!(
            vetoed.activate(&id(2), T0 + OWNER_DELAY_SECS).unwrap_err(),
            ProtocolError::NoSuchPending
        );
    }
}

#[test]
fn a_business_keeps_an_owner_and_a_recovery_key() {
    let s = team();
    // The only owner can't step down to a lesser role or leave.
    let demote = MembershipChange::ChangeRole {
        member: pk(&owner()),
        role: Role::Viewer,
    };
    assert_eq!(
        s.propose(id(1), &pk(&owner()), &demote, T0).unwrap_err(),
        ProtocolError::NoOwner
    );
    let leave = MembershipChange::Remove {
        member: pk(&owner()),
    };
    assert_eq!(
        s.propose(id(1), &pk(&owner()), &leave, T0).unwrap_err(),
        ProtocolError::NoOwner
    );
    // With one owner the backup key can't be dropped.
    let drop = MembershipChange::ReplaceRecoveryKey { key: None };
    assert_eq!(
        s.propose(id(1), &pk(&owner()), &drop, T0).unwrap_err(),
        ProtocolError::NoRecovery
    );

    // Genesis without a usable recovery party.
    assert_eq!(
        MembershipState::genesis(
            business_id(),
            pk(&owner()),
            pk(&service()),
            Recovery::BackupKey {
                pubkey: pk(&owner())
            }
        )
        .unwrap_err(),
        ProtocolError::AlreadyMember
    );
    assert_eq!(
        MembershipState::genesis(
            business_id(),
            pk(&owner()),
            pk(&service()),
            Recovery::SecondOwner {
                pubkey: pk(&owner())
            }
        )
        .unwrap_err(),
        ProtocolError::AlreadyMember
    );

    // With a second owner instead of a backup key, neither owner can leave
    // while the other is the only recovery party.
    let two = MembershipState::genesis(
        business_id(),
        pk(&owner()),
        pk(&service()),
        Recovery::SecondOwner {
            pubkey: pk(&key(40)),
        },
    )
    .unwrap();
    assert_eq!(two.recovery_key, None);
    assert_eq!(
        two.propose(
            id(1),
            &pk(&owner()),
            &MembershipChange::Remove {
                member: pk(&key(40))
            },
            T0
        )
        .unwrap_err(),
        ProtocolError::NoRecovery
    );
}

#[test]
fn an_owner_level_change_is_checked_again_when_due() {
    // Two pending removals that together would leave no recovery party.
    let s = MembershipState::genesis(
        business_id(),
        pk(&owner()),
        pk(&service()),
        Recovery::SecondOwner {
            pubkey: pk(&key(40)),
        },
    )
    .unwrap();
    let s = s
        .propose(
            id(1),
            &pk(&owner()),
            &MembershipChange::ReplaceRecoveryKey {
                key: Some(pk(&backup())),
            },
            T0,
        )
        .unwrap()
        .state;
    let s = s.activate(&id(1), T0 + OWNER_DELAY_SECS).unwrap().0;
    let s = s
        .propose(
            id(2),
            &pk(&owner()),
            &MembershipChange::Remove {
                member: pk(&key(40)),
            },
            T0,
        )
        .unwrap()
        .state;
    let s = s
        .propose(
            id(3),
            &pk(&key(40)),
            &MembershipChange::ReplaceRecoveryKey { key: None },
            T0,
        )
        .unwrap()
        .state;
    let due = T0 + OWNER_DELAY_SECS;
    // id(3)'s proposer is removed by id(2), which cancels id(3).
    let (s, _) = s.activate(&id(2), due).unwrap();
    assert!(s.pending.is_empty());
    assert_eq!(s.recovery_key, Some(pk(&backup())));
}

#[test]
fn recovery_replaces_a_lost_owner_after_the_wait() {
    let s = team();
    let replacement = key(50);
    let change = MembershipChange::Recover {
        lost: pk(&owner()),
        replacement: pk(&replacement),
    };
    // Only the recovery key may recover.
    assert_eq!(
        s.propose(id(1), &pk(&owner()), &change, T0).unwrap_err(),
        ProtocolError::Forbidden
    );
    let u = s.propose(id(1), &pk(&backup()), &change, T0).unwrap();
    assert!(matches!(u.outcome, Outcome::Pending { .. }));

    // The owner named as lost can't veto it: the key may be a thief's.
    assert_eq!(
        u.state.veto(&pk(&owner()), &id(1)).unwrap_err(),
        ProtocolError::RecoveredVeto
    );
    assert_eq!(
        u.state.veto(&pk(&backup()), &id(1)).unwrap_err(),
        ProtocolError::OwnVeto
    );

    let (s, new_epoch) = u.state.activate(&id(1), T0 + OWNER_DELAY_SECS).unwrap();
    assert!(new_epoch);
    assert_eq!(s.actor(&pk(&owner())), Actor::Outsider);
    assert_eq!(s.actor(&pk(&replacement)), Actor::Member(Role::Owner));
    assert_eq!(s.readable_epochs(&pk(&replacement)), Some(0..=1));
}

#[test]
fn replacing_the_service_key_starts_an_epoch() {
    let s = team();
    let change = MembershipChange::ReplaceServiceKey { key: pk(&key(60)) };
    let s = s.propose(id(1), &pk(&owner()), &change, T0).unwrap().state;
    let (s, new_epoch) = s.activate(&id(1), T0 + OWNER_DELAY_SECS).unwrap();
    assert!(new_epoch);
    assert_eq!(s.service, pk(&key(60)));
}

#[test]
fn members_must_be_new_and_present() {
    let s = team();
    assert_eq!(
        s.propose(
            id(1),
            &pk(&owner()),
            &add(&viewer(), Role::Viewer, History::Full),
            T0
        )
        .unwrap_err(),
        ProtocolError::AlreadyMember
    );
    assert_eq!(
        s.propose(
            id(1),
            &pk(&owner()),
            &add(&backup(), Role::Viewer, History::Full),
            T0
        )
        .unwrap_err(),
        ProtocolError::AlreadyMember
    );
    assert_eq!(
        s.propose(
            id(1),
            &pk(&owner()),
            &add(&service(), Role::Viewer, History::Full),
            T0
        )
        .unwrap_err(),
        ProtocolError::AlreadyMember
    );
    assert_eq!(
        s.propose(
            id(1),
            &pk(&owner()),
            &MembershipChange::Remove {
                member: pk(&outsider())
            },
            T0
        )
        .unwrap_err(),
        ProtocolError::UnknownMember
    );
}
