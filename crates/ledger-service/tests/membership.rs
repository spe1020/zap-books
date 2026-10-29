//! Membership and keys in the service (stage 2 brief, PR 3), against
//! Postgres: epochs, deliveries, the 48-hour scheduler, vetoes, recovery,
//! revocation and history access. Every time-based test moves the ledger's
//! hand-set clock and runs the scheduler's work directly; nothing waits.

mod common;

use std::collections::BTreeMap;

use common::*;
use ledger_service::clock::Clock;
use ledger_service::ledger::KEY_NOT_HELD;
use ledger_service::Status;
use nostr::nips::nip98::{HttpData, HttpMethod};
use nostr::prelude::{Event, EventId, Keys};
use nostr::types::url::Url;
use protocol::{
    audit, decrypt_from, parse_acceptance, rebuild, verify_chain, Actor, Command, EpochKey,
    History, MembershipChange, MembershipState, Outcome, ProtocolError, Recovery, Role,
    OWNER_DELAY_SECS,
};

fn add(keys: &Keys, role: Role, history: History) -> MembershipChange {
    MembershipChange::Add {
        member: pk(keys),
        role,
        history,
    }
}

fn remove(keys: &Keys) -> MembershipChange {
    MembershipChange::Remove { member: pk(keys) }
}

fn veto(pending: EventId) -> Command {
    Command::Veto { pending }
}

/// The proposal a pending acceptance is for: what a veto names.
fn pending_id(ack: &ledger_service::Acknowledgment) -> EventId {
    parse_acceptance(&acceptance_event(ack)).unwrap().proposal
}

fn message(error: ProtocolError) -> String {
    error.to_string()
}

fn assert_accepted(ack: &ledger_service::Acknowledgment) {
    assert_eq!(ack.status, Status::Accepted, "{}", ack.text);
}

fn assert_rejected(ack: &ledger_service::Acknowledgment, author: &Keys, error: ProtocolError) {
    assert_eq!(rejection(ack, author).reason.message, message(error));
}

/// The seqs whose content any of `keys` decrypts. Every key is tried on
/// every acceptance, whatever its epoch tag says.
fn readable(chain: &[Event], keys: &BTreeMap<u64, EpochKey>) -> Vec<u64> {
    chain
        .iter()
        .filter(|e| {
            keys.values()
                .any(|k| decrypt_from(k.secret_key(), &e.pubkey, &e.content).is_ok())
        })
        .map(|e| parse_acceptance(e).unwrap().seq)
        .collect()
}

fn epochs(keys: &BTreeMap<u64, EpochKey>) -> Vec<u64> {
    keys.keys().copied().collect()
}

async fn auth(keys: &Keys, url: &str) -> String {
    HttpData::new(Url::parse(url).unwrap(), HttpMethod::GET)
        .to_authorization(keys)
        .await
        .unwrap()
}

/// A NIP-98 GET of `path` signed by `keys`.
async fn get(base: &str, keys: &Keys, path: &str) -> (u16, String) {
    let header = auth(keys, &format!("{base}{path}")).await;
    http(base, "GET", path, Some(&header), "").await
}

#[tokio::test]
async fn a_removed_member_is_refused_at_once_and_cant_read_after_rotation() {
    let w = World::new().await;
    w.create().await; // seq 0, epoch 0
    let bookkeeper = key(20);
    assert_accepted(
        &w.change(&owner(), add(&bookkeeper, Role::Bookkeeper, History::Full))
            .await,
    ); // 1
    assert_accepted(
        &w.send(&bookkeeper, Some(1), save(w.business, "draft-1"))
            .await,
    ); // 2
    let kept = w.keys_for(&bookkeeper).await;
    assert_eq!(epochs(&kept), vec![0]);

    let removed = w.change(&owner(), remove(&bookkeeper)).await; // 3
    assert_accepted(&removed);
    // Removing a member starts epoch 1, and the removal is the first
    // acceptance under it.
    assert_eq!(
        parse_acceptance(&acceptance_event(&removed)).unwrap().epoch,
        1
    );
    assert_eq!(w.membership().await.epoch, 1);

    // Refused at once: proposals, the chain, and keys, even the ones once
    // received.
    let refused = w
        .send(&bookkeeper, Some(3), save(w.business, "draft-2"))
        .await;
    assert_rejected(&refused, &bookkeeper, ProtocolError::NotMember);
    assert!(w
        .ledger
        .acceptances(&w.business, &pk(&bookkeeper), None, 100)
        .await
        .unwrap()
        .is_none());
    assert!(w.keys_for(&bookkeeper).await.is_empty());
    let base = w.serve().await;
    let path = format!("/v1/businesses/{}/acceptances", w.business);
    assert_eq!(get(&base, &bookkeeper, &path).await.0, 404);
    let (status, body) = get(&base, &bookkeeper, "/v1/epoch-keys").await;
    assert_eq!(status, 200);
    assert_eq!(body, r#"{"deliveries":[]}"#);

    // After the rotation, the keys they kept open nothing new.
    assert_accepted(&w.send(&owner(), Some(3), save(w.business, "draft-3")).await); // 4
    let chain = w.chain().await;
    assert_eq!(readable(&chain, &kept), vec![0, 1, 2]);
    assert_eq!(
        readable(&chain, &w.keys_for(&owner()).await),
        vec![0, 1, 2, 3, 4]
    );
    w.finish().await;
}

#[tokio::test]
async fn an_owner_level_change_activates_after_48_hours() {
    let w = World::new().await;
    w.create().await;
    let joining = key(21);
    let proposed_at = w.clock.now_secs();
    let ack = w
        .change(&owner(), add(&joining, Role::Owner, History::Full))
        .await;
    assert_accepted(&ack);
    let (_, contents) = w.read(&owner()).await;
    assert_eq!(
        contents[1].outcome,
        Outcome::Pending {
            effective_at: proposed_at + OWNER_DELAY_SECS
        }
    );
    let state = w.membership().await;
    assert_eq!(state.pending.len(), 1);
    assert_eq!(state.actor(&pk(&joining)), Actor::Outsider);

    // Visible to every owner and the recovery key; to no one else.
    let base = w.serve().await;
    let path = format!("/v1/businesses/{}/pending", w.business);
    for keys in [owner(), backup()] {
        let (status, body) = get(&base, &keys, &path).await;
        assert_eq!(status, 200, "{body}");
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(value["pending"][0]["proposal"], pending_id(&ack).to_hex());
        assert_eq!(value["pending"][0]["change"]["op"], "add");
        assert_eq!(
            value["pending"][0]["effectiveAt"],
            proposed_at + OWNER_DELAY_SECS
        );
    }
    assert_eq!(get(&base, &joining, &path).await.0, 404);
    assert_eq!(get(&base, &outsider(), &path).await.0, 404);

    // One second early: nothing happens.
    w.clock.advance_secs(OWNER_DELAY_SECS - 1);
    assert_eq!(w.run_due().await, 0);
    assert_eq!(w.head().await, 1);
    let early = w.send(&joining, Some(1), save(w.business, "draft-1")).await;
    assert_rejected(&early, &joining, ProtocolError::NotMember);

    // Due: the scheduler appends the activation, once.
    w.clock.advance_secs(1);
    assert_eq!(w.run_due().await, 1);
    assert_eq!(w.run_due().await, 0);
    assert_eq!(w.head().await, 2);
    let state = w.membership().await;
    assert_eq!(state.actor(&pk(&joining)), Actor::Member(Role::Owner));
    assert!(state.pending.is_empty());
    let rows: i64 = sqlx::query_scalar("select count(*) from pending_changes")
        .fetch_one(&w.pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);

    // The activation repeats the pending proposal.
    let (events, contents) = w.read(&owner()).await;
    assert_eq!(contents[2].outcome, Outcome::Activated);
    assert_eq!(contents[2].proposal.id, contents[1].proposal.id);
    assert_eq!(contents[2].payload, contents[1].payload);
    verify_chain(w.business, pk(&service_keys()), None, &events).unwrap();

    // The new owner reads everything and acts.
    assert_eq!(epochs(&w.keys_for(&joining).await), vec![0]);
    assert_accepted(&w.send(&joining, Some(2), save(w.business, "draft-1")).await);
    w.finish().await;
}

#[tokio::test]
async fn another_owners_veto_cancels_a_pending_change() {
    let w = World::new().await;
    w.send(
        &owner(),
        None,
        genesis(
            w.business,
            Recovery::SecondOwner {
                pubkey: pk(&second_owner()),
            },
        ),
    )
    .await;
    let joining = key(22);
    let ack = w
        .change(&owner(), add(&joining, Role::Owner, History::Full))
        .await;
    let pending = pending_id(&ack);

    // The proposer can't veto their own change.
    let own = w.send(&owner(), Some(1), veto(pending)).await;
    assert_rejected(&own, &owner(), ProtocolError::OwnVeto);

    let vetoed = w.send(&second_owner(), Some(1), veto(pending)).await;
    assert_accepted(&vetoed);
    assert!(w.membership().await.pending.is_empty());
    // A second veto finds nothing to cancel.
    let again = w.send(&second_owner(), Some(2), veto(pending)).await;
    assert_rejected(&again, &second_owner(), ProtocolError::NoSuchPending);

    w.clock.advance_secs(OWNER_DELAY_SECS);
    assert_eq!(w.run_due().await, 0);
    assert_eq!(w.head().await, 2);
    assert_eq!(w.membership().await.actor(&pk(&joining)), Actor::Outsider);
    w.finish().await;
}

#[tokio::test]
async fn the_recovery_keys_veto_cancels_a_pending_change_and_a_late_veto_is_refused() {
    let w = World::new().await;
    w.create().await;
    let ack = w
        .change(
            &owner(),
            MembershipChange::ReplaceRecoveryKey {
                key: Some(pk(&key(23))),
            },
        )
        .await;
    assert_accepted(&w.send(&backup(), Some(1), veto(pending_id(&ack))).await);
    w.clock.advance_secs(OWNER_DELAY_SECS);
    assert_eq!(w.run_due().await, 0);
    assert_eq!(w.membership().await.recovery_key, Some(pk(&backup())));

    // A veto after the change fell due is too late, even if the scheduler
    // hasn't run: the service activates due changes first.
    let joining = key(24);
    let ack = w
        .change(&owner(), add(&joining, Role::Owner, History::Full))
        .await; // 3
    w.clock.advance_secs(OWNER_DELAY_SECS);
    let late = w.send(&backup(), Some(3), veto(pending_id(&ack))).await;
    assert_rejected(&late, &backup(), ProtocolError::NoSuchPending);
    assert_eq!(w.head().await, 4);
    assert_eq!(
        w.membership().await.actor(&pk(&joining)),
        Actor::Member(Role::Owner)
    );
    w.finish().await;
}

#[tokio::test]
async fn commands_that_would_leave_no_owner_or_no_recovery_key_are_rejected() {
    // One owner and a backup key.
    let w = World::new().await;
    w.create().await;
    let no_owner = [
        remove(&owner()),
        MembershipChange::ChangeRole {
            member: pk(&owner()),
            role: Role::Viewer,
        },
    ];
    for change in no_owner {
        assert_rejected(
            &w.change(&owner(), change).await,
            &owner(),
            ProtocolError::NoOwner,
        );
    }
    let drop_backup = MembershipChange::ReplaceRecoveryKey { key: None };
    assert_rejected(
        &w.change(&owner(), drop_backup).await,
        &owner(),
        ProtocolError::NoRecovery,
    );
    assert_eq!(w.head().await, 0);

    // Two owners and no backup: neither may leave.
    let two = World::new().await;
    two.send(
        &owner(),
        None,
        genesis(
            two.business,
            Recovery::SecondOwner {
                pubkey: pk(&second_owner()),
            },
        ),
    )
    .await;
    assert_rejected(
        &two.change(&owner(), remove(&second_owner())).await,
        &owner(),
        ProtocolError::NoRecovery,
    );
    two.finish().await;

    // Checked again when due. Two owners and a backup key: dropping the
    // backup and removing an owner are each fine alone, but not together.
    // The first activates; the second lapses and changes nothing.
    let other = key(25);
    w.change(&owner(), add(&other, Role::Owner, History::Full))
        .await;
    w.clock.advance_secs(OWNER_DELAY_SECS);
    assert_eq!(w.run_due().await, 1); // 2
    assert_accepted(&w.change(&owner(), drop_backup).await); // 3
    assert_accepted(&w.change(&other, remove(&owner())).await); // 4
    w.clock.advance_secs(OWNER_DELAY_SECS);
    assert_eq!(w.run_due().await, 2); // 5, 6
    let (_, contents) = w.read(&owner()).await;
    assert_eq!(contents[5].outcome, Outcome::Activated);
    assert_eq!(contents[6].outcome, Outcome::Lapsed);
    let state = w.membership().await;
    assert_eq!(state.recovery_key, None);
    assert_eq!(state.actor(&pk(&owner())), Actor::Member(Role::Owner));
    assert_eq!(state.actor(&pk(&other)), Actor::Member(Role::Owner));
    assert!(state.pending.is_empty());
    w.finish().await;
}

#[tokio::test]
async fn from_join_members_cant_read_before_joining_and_full_history_members_read_everything() {
    let w = World::new().await;
    w.create().await; // 0, epoch 0
    let late = key(26);
    let full = key(27);
    w.send(&owner(), Some(0), save(w.business, "draft-1")).await; // 1
    assert_accepted(
        &w.change(&owner(), add(&late, Role::Viewer, History::FromJoin))
            .await,
    ); // 2, epoch 1
    w.send(&owner(), Some(2), save(w.business, "draft-2")).await; // 3
    assert_accepted(
        &w.change(&owner(), add(&full, Role::Viewer, History::Full))
            .await,
    ); // 4
    w.send(&owner(), Some(4), save(w.business, "draft-3")).await; // 5

    let chain = w.chain().await;
    assert_eq!(chain.len(), 6);
    // Joining from-join-date started epoch 1: no key before it.
    let late_keys = w.keys_for(&late).await;
    assert_eq!(epochs(&late_keys), vec![1]);
    assert_eq!(readable(&chain, &late_keys), vec![2, 3, 4, 5]);
    // A full-history member joining later receives every past key.
    let full_keys = w.keys_for(&full).await;
    assert_eq!(epochs(&full_keys), vec![0, 1]);
    assert_eq!(readable(&chain, &full_keys), vec![0, 1, 2, 3, 4, 5]);
    let (_, theirs) = w.read(&full).await;
    let (_, owners) = w.read(&owner()).await;
    assert_eq!(theirs, owners);
    w.finish().await;
}

#[tokio::test]
async fn a_submitter_can_only_submit_receipt_drafts_and_receives_no_keys() {
    let w = World::new().await;
    w.create().await;
    let submitter = key(28);
    assert_accepted(
        &w.change(&owner(), add(&submitter, Role::Submitter, History::Full))
            .await,
    ); // 1
       // A submitter reads nothing, so adding one starts no epoch.
    assert_eq!(w.membership().await.epoch, 0);

    let submit = |id: &str| Command::SubmitReceiptDraft {
        draft: draft(w.business, id),
        documents: vec![],
    };
    assert_accepted(&w.send(&submitter, Some(1), submit("submitted-1")).await); // 2
    assert_eq!(w.snapshot().await.drafts.len(), 1);
    // Only new drafts.
    let again = w.send(&submitter, Some(2), submit("submitted-1")).await;
    assert_rejected(&again, &submitter, ProtocolError::SubmitterNewDraftsOnly);
    // Nothing else.
    let others = [
        save(w.business, "draft-1"),
        approve("submitted-1"),
        Command::UpdateSettings {
            change: ledger_core::SettingsChange::AddCategory {
                name: "Invented travel".into(),
                currency: ledger_core::Currency::Usd,
            },
        },
        Command::ChangeMembership {
            change: add(&key(29), Role::Viewer, History::Full),
        },
        veto(EventId::from_byte_array([0; 32])),
    ];
    for command in others {
        let ack = w.send(&submitter, Some(2), command).await;
        assert_rejected(&ack, &submitter, ProtocolError::Forbidden);
    }
    assert_eq!(w.head().await, 2);

    // No keys, no chain, no pending changes.
    assert!(w.keys_for(&submitter).await.is_empty());
    let delivered: i64 =
        sqlx::query_scalar("select count(*) from epoch_deliveries where member = $1")
            .bind(pk(&submitter).to_hex())
            .fetch_one(&w.pool)
            .await
            .unwrap();
    assert_eq!(delivered, 0);
    assert!(w
        .ledger
        .acceptances(&w.business, &pk(&submitter), None, 100)
        .await
        .unwrap()
        .is_none());
    assert!(w
        .ledger
        .pending(&w.business, &pk(&submitter))
        .await
        .unwrap()
        .is_none());
    w.finish().await;
}

#[tokio::test]
async fn a_stolen_owner_key_cant_veto_its_recovery() {
    // The only owner's key is stolen: whoever holds `owner()` now is the
    // thief. The owner recovers with the backup key.
    let w = World::new().await;
    w.create().await;
    w.send(&owner(), Some(0), save(w.business, "draft-1")).await; // 1
    let thief = owner();
    let replacement = key(30);
    let recover = MembershipChange::Recover {
        lost: pk(&thief),
        replacement: pk(&replacement),
    };
    // The recovery key signs only recovery and vetoes.
    let other = w
        .change(&backup(), add(&key(31), Role::Viewer, History::Full))
        .await;
    assert_rejected(&other, &backup(), ProtocolError::Forbidden);

    let pending = w.change(&backup(), recover).await; // 2
    assert_accepted(&pending);
    // The thief's veto is refused: recovery outranks the owner it replaces.
    let vetoed = w.send(&thief, Some(2), veto(pending_id(&pending))).await;
    assert_rejected(&vetoed, &thief, ProtocolError::RecoveredVeto);
    assert_eq!(w.membership().await.pending.len(), 1);

    w.clock.advance_secs(OWNER_DELAY_SECS);
    assert_eq!(w.run_due().await, 1); // 3, epoch 1
    let state = w.membership().await;
    assert_eq!(state.epoch, 1);
    assert_eq!(state.actor(&pk(&thief)), Actor::Outsider);
    assert_eq!(state.actor(&pk(&replacement)), Actor::Member(Role::Owner));

    // The thief is cut off; the replacement owner has full history.
    let lost = w.send(&thief, Some(3), save(w.business, "draft-2")).await;
    assert_rejected(&lost, &thief, ProtocolError::NotMember);
    assert!(w.keys_for(&thief).await.is_empty());
    let keys = w.keys_for(&replacement).await;
    assert_eq!(epochs(&keys), vec![0, 1]);
    assert_eq!(readable(&w.chain().await, &keys), vec![0, 1, 2, 3]);
    assert_accepted(
        &w.send(&replacement, Some(3), save(w.business, "draft-2"))
            .await,
    );
    w.finish().await;
}

#[tokio::test]
async fn another_owners_veto_cancels_a_malicious_recovery() {
    // Two owners and a backup key; the backup key is misused against the
    // first owner.
    let w = World::new().await;
    w.create().await;
    w.change(&owner(), add(&second_owner(), Role::Owner, History::Full))
        .await; // 1
    w.clock.advance_secs(OWNER_DELAY_SECS);
    assert_eq!(w.run_due().await, 1); // 2
    let attacker = key(38);
    let pending = w
        .change(
            &backup(),
            MembershipChange::Recover {
                lost: pk(&owner()),
                replacement: pk(&attacker),
            },
        )
        .await; // 3
    assert_accepted(&pending);
    let id = pending_id(&pending);

    // The targeted owner can't veto it, but the other owner can.
    let own = w.send(&owner(), Some(3), veto(id)).await;
    assert_rejected(&own, &owner(), ProtocolError::RecoveredVeto);
    assert_accepted(&w.send(&second_owner(), Some(3), veto(id)).await); // 4

    w.clock.advance_secs(OWNER_DELAY_SECS);
    assert_eq!(w.run_due().await, 0);
    let state = w.membership().await;
    assert!(state.pending.is_empty());
    assert_eq!(state.epoch, 0);
    assert_eq!(state.actor(&pk(&owner())), Actor::Member(Role::Owner));
    assert_eq!(state.actor(&pk(&attacker)), Actor::Outsider);
    w.finish().await;
}

#[tokio::test]
async fn replacing_the_service_key_hands_the_chain_to_the_new_key() {
    let w = World::new().await;
    w.create().await;
    let old = pk(&service_keys());
    let new = pk(&next_service_keys());

    // A key the service doesn't hold is refused: it could sign nothing.
    let unheld = w
        .change(
            &owner(),
            MembershipChange::ReplaceServiceKey { key: pk(&key(32)) },
        )
        .await;
    assert_eq!(rejection(&unheld, &owner()).reason.message, KEY_NOT_HELD);

    assert_accepted(
        &w.change(&owner(), MembershipChange::ReplaceServiceKey { key: new })
            .await,
    ); // 1
    w.clock.advance_secs(OWNER_DELAY_SECS);
    assert_eq!(w.run_due().await, 1); // 2

    // The old key signs the hand-over, which names the new key and starts
    // epoch 1.
    let chain = w.chain().await;
    assert_eq!(chain[2].pubkey, old);
    let handover = parse_acceptance(&chain[2]).unwrap();
    assert_eq!(handover.next_service, Some(new));
    assert_eq!(handover.epoch, 1);
    assert_eq!(w.membership().await.service, new);

    // Proposals now go to the new key.
    let to_old = w.send(&owner(), Some(2), save(w.business, "draft-1")).await;
    let reason = rejection(&to_old, &owner());
    assert_eq!(reason.reason.message, message(ProtocolError::Decrypt));
    let to_new = w.proposal_to(&new, &owner(), Some(2), save(w.business, "draft-1"));
    let ack = w.ledger.submit(&to_new).await.unwrap();
    assert_accepted(&ack);
    assert_eq!(acceptance_event(&ack).pubkey, new);

    // The chain verifies across the hand-over, from the key genesis
    // declared.
    let chain = w.chain().await;
    let head = verify_chain(w.business, old, None, &chain)
        .unwrap()
        .unwrap();
    assert_eq!(head.service, new);

    // Older epoch secrets were resealed to the new key: a full-history
    // member added now receives them all.
    let viewer = key(33);
    let change = Command::ChangeMembership {
        change: add(&viewer, Role::Viewer, History::Full),
    };
    let ack = w
        .ledger
        .submit(&w.proposal_to(&new, &owner(), Some(3), change))
        .await
        .unwrap();
    assert_accepted(&ack);
    let keys = w.keys_for(&viewer).await;
    assert_eq!(epochs(&keys), vec![0, 1]);
    assert_eq!(readable(&w.chain().await, &keys), vec![0, 1, 2, 3, 4]);
    w.finish().await;
}

#[tokio::test]
async fn a_chain_with_membership_changes_rebuilds_audits_and_rederives_the_membership() {
    let w = World::new().await;
    w.create().await; // 0
    let bookkeeper = key(34);
    let late = key(35);
    let joining = key(36);
    w.change(&owner(), add(&bookkeeper, Role::Bookkeeper, History::Full))
        .await; // 1
    w.send(&bookkeeper, Some(1), save(w.business, "draft-1"))
        .await; // 2
    w.change(&owner(), add(&late, Role::Viewer, History::FromJoin))
        .await; // 3, epoch 1
    let vetoed = w
        .change(&owner(), add(&key(37), Role::Owner, History::Full))
        .await; // 4
    w.send(&backup(), Some(4), veto(pending_id(&vetoed))).await; // 5
    w.change(&owner(), add(&joining, Role::Owner, History::Full))
        .await; // 6
    w.clock.advance_secs(OWNER_DELAY_SECS);
    w.send(&bookkeeper, Some(6), approve("draft-1")).await; // 7 activation, 8
    w.change(&owner(), remove(&bookkeeper)).await; // 9, epoch 2
    w.send(&joining, Some(9), save(w.business, "draft-2")).await; // 10
    assert_eq!(w.head().await, 10);

    let (events, contents) = w.read(&owner()).await;
    verify_chain(w.business, pk(&service_keys()), None, &events).unwrap();
    assert_eq!(rebuild(&contents).unwrap(), w.snapshot().await);
    let mut books = audit(None, &contents[0]).unwrap();
    for content in &contents[1..] {
        books = audit(Some(&books), content).unwrap();
    }
    assert_eq!(books, w.snapshot().await);

    // A reader re-derives the membership and checks every decision.
    let mut state = MembershipState::from_genesis(&contents[0]).unwrap();
    for (event, content) in events.iter().zip(&contents).skip(1) {
        let header = parse_acceptance(event).unwrap();
        state = state
            .follow(&header, content, event.created_at.as_secs())
            .unwrap();
    }
    assert_eq!(state, w.membership().await);
    assert_eq!(state.epoch, 2);

    // The members table matches.
    let members: Vec<(String, String)> =
        sqlx::query_as("select pubkey, role from members order by pubkey")
            .fetch_all(&w.pool)
            .await
            .unwrap();
    let mut expected: Vec<(String, String)> = state
        .members
        .iter()
        .map(|(k, m)| {
            (
                k.to_hex(),
                serde_json::to_value(m.role)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string(),
            )
        })
        .collect();
    expected.sort();
    assert_eq!(members, expected);
    w.finish().await;
}
