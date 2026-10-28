//! The pieces together, as the service and a reader would use them:
//! genesis, membership changes that rotate epochs, key deliveries, and a
//! reader re-deriving membership from the chain it can decrypt.

mod common;

use std::collections::BTreeMap;

use common::*;
use nostr::prelude::{Event, Keys};
use protocol::{
    build_acceptance, build_delivery, build_proposal, open_acceptance, open_delivery,
    open_proposal, verify_chain, AcceptanceContent, AcceptanceHeader, Command, EnvRecord, EpochKey,
    History, MembershipChange, MembershipState, Outcome, ProposalPayload, Role, OWNER_DELAY_SECS,
    PROTOCOL_VERSION,
};

const T0: u64 = 1_800_000_000;

/// A stand-in for the service: membership, epoch keys and the chain.
struct Books {
    state: MembershipState,
    epochs: Vec<EpochKey>,
    chain: Vec<Event>,
}

impl Books {
    fn create() -> Books {
        let genesis = genesis_event();
        let (_, payload) = open_proposal(&genesis, &service()).unwrap();
        let mut books = Books {
            state: MembershipState::genesis(pk(&owner()), pk(&service()), backup_recovery())
                .unwrap(),
            epochs: vec![EpochKey::generate(0)],
            chain: vec![],
        };
        books.append(genesis, payload, Outcome::Applied, T0);
        books
    }

    fn append(&mut self, proposal: Event, payload: ProposalPayload, outcome: Outcome, now: u64) {
        let epoch = self.epochs.last().unwrap();
        let header = AcceptanceHeader {
            business: business_id(),
            seq: self.chain.len() as u64,
            prev: self.chain.last().map(|e| e.id),
            proposal: proposal.id,
            epoch: epoch.epoch,
            next_service: None,
        };
        let content = AcceptanceContent {
            v: PROTOCOL_VERSION,
            proposal,
            payload,
            env: EnvRecord {
                ids: vec![],
                now: "2027-01-15T08:00:00.000Z".into(),
            },
            changes: vec![],
            outcome,
        };
        let event =
            build_acceptance(&service(), &epoch.public_key(), &header, &content, at(now)).unwrap();
        self.chain.push(event);
    }

    /// Proposes and accepts a membership change, as the service would.
    fn change(&mut self, signer: &Keys, change: MembershipChange, now: u64) -> Event {
        let payload = ProposalPayload {
            v: PROTOCOL_VERSION,
            business: business_id(),
            base_seq: Some(self.chain.len() as u64 - 1),
            command: Command::ChangeMembership { change },
        };
        let proposal = build_proposal(signer, &pk(&service()), &payload, at(now)).unwrap();
        let (header, payload) = open_proposal(&proposal, &service()).unwrap();
        let update = self
            .state
            .propose(header.id, &header.author, &change, now)
            .unwrap();
        if update.new_epoch {
            self.epochs.push(EpochKey::generate(update.state.epoch));
        }
        self.state = update.state;
        self.append(proposal.clone(), payload, update.outcome, now);
        proposal
    }

    /// Activates a due pending change.
    fn activate(&mut self, proposal: &Event, now: u64) {
        let (_, payload) = open_proposal(proposal, &service()).unwrap();
        let (state, new_epoch) = self.state.activate(&proposal.id, now).unwrap();
        if new_epoch {
            self.epochs.push(EpochKey::generate(state.epoch));
        }
        self.state = state;
        self.append(proposal.clone(), payload, Outcome::Activated, now);
    }

    /// The deliveries a member is entitled to, opened by that member.
    fn keys_for(&self, member: &Keys) -> BTreeMap<u64, EpochKey> {
        let Some(range) = self.state.readable_epochs(&pk(member)) else {
            return BTreeMap::new();
        };
        range
            .map(|n| {
                let event = build_delivery(
                    &service(),
                    &pk(member),
                    business_id(),
                    &self.epochs[n as usize],
                    at(T0),
                )
                .unwrap();
                let delivery = open_delivery(&event, member, &pk(&service())).unwrap();
                (n, delivery.key)
            })
            .collect()
    }
}

/// Which seqs `keys` can decrypt.
fn readable(chain: &[Event], keys: &BTreeMap<u64, EpochKey>) -> Vec<u64> {
    chain
        .iter()
        .filter_map(|e| {
            let header = protocol::parse_acceptance(e).unwrap();
            let key = keys.get(&header.epoch)?;
            open_acceptance(e, key).ok().map(|(h, _)| h.seq)
        })
        .collect()
}

#[test]
fn epochs_follow_membership_and_readers_rebuild_it() {
    let mut books = Books::create(); // seq 0, epoch 0
    let late = key(70);
    let full = key(71);
    books.change(
        &owner(),
        add(&bookkeeper(), Role::Bookkeeper, History::Full),
        T0,
    ); // 1
    books.change(&owner(), add(&late, Role::Viewer, History::FromJoin), T0); // 2, epoch 1
    books.change(
        &owner(),
        add(&submitter(), Role::Submitter, History::Full),
        T0,
    ); // 3
    books.change(
        &owner(),
        MembershipChange::Remove {
            member: pk(&bookkeeper()),
        },
        T0,
    ); // 4, epoch 2
    let pending = books.change(&owner(), add(&key(72), Role::Owner, History::Full), T0); // 5
    books.activate(&pending, T0 + OWNER_DELAY_SECS); // 6
    books.change(
        &owner(),
        add(&full, Role::Viewer, History::Full),
        T0 + OWNER_DELAY_SECS,
    ); // 7
    assert_eq!(books.state.epoch, 2);

    verify_chain(business_id(), pk(&service()), None, &books.chain).unwrap();
    let all: Vec<u64> = (0..books.chain.len() as u64).collect();

    // Full history reads everything; from-join-date reads from its join.
    assert_eq!(readable(&books.chain, &books.keys_for(&owner())), all);
    assert_eq!(readable(&books.chain, &books.keys_for(&full)), all);
    assert_eq!(
        readable(&books.chain, &books.keys_for(&late)),
        vec![2, 3, 4, 5, 6, 7]
    );
    // A submitter receives nothing and reads nothing.
    assert!(books.keys_for(&submitter()).is_empty());
    // The removed bookkeeper kept epoch 0's key, which opens nothing after
    // the removal.
    let old: BTreeMap<u64, EpochKey> = [(0, books.epochs[0].clone())].into();
    assert_eq!(readable(&books.chain, &old), vec![0, 1]);
    assert!(books.keys_for(&bookkeeper()).is_empty());

    // A full-history reader re-derives the membership from the chain.
    let keys = books.keys_for(&full);
    let mut rebuilt: Option<MembershipState> = None;
    for event in &books.chain {
        let header = protocol::parse_acceptance(event).unwrap();
        let (_, content) = open_acceptance(event, &keys[&header.epoch]).unwrap();
        let at = event.created_at.as_secs();
        rebuilt = Some(match rebuilt {
            None => MembershipState::from_genesis(&content).unwrap(),
            Some(state) => {
                let next = state.follow(&content, at).unwrap();
                // The acceptance that starts an epoch is the first under it.
                assert_eq!(next.epoch, header.epoch);
                next
            }
        });
    }
    assert_eq!(rebuilt.unwrap(), books.state);
}

#[test]
fn a_reader_refuses_a_change_the_service_should_have_rejected() {
    let books = Books::create();
    // The service accepts an outsider's membership change: a reader replaying
    // the chain catches it.
    let payload = ProposalPayload {
        v: PROTOCOL_VERSION,
        business: business_id(),
        base_seq: Some(0),
        command: Command::ChangeMembership {
            change: add(&key(80), Role::Bookkeeper, History::Full),
        },
    };
    let proposal = build_proposal(&outsider(), &pk(&service()), &payload, at(T0)).unwrap();
    let content = AcceptanceContent {
        v: PROTOCOL_VERSION,
        proposal,
        payload,
        env: EnvRecord {
            ids: vec![],
            now: "2027-01-15T08:00:00.000Z".into(),
        },
        changes: vec![],
        outcome: Outcome::Applied,
    };
    assert!(books.state.follow(&content, T0).is_err());

    // Or an owner-level change applied without its wait.
    let mut content = content;
    content.proposal = build_proposal(&owner(), &pk(&service()), &content.payload, at(T0)).unwrap();
    content.payload.command = Command::ChangeMembership {
        change: add(&key(80), Role::Owner, History::Full),
    };
    assert!(books.state.follow(&content, T0).is_err());
}

#[test]
fn a_pending_change_that_no_longer_applies_lapses() {
    let mut books = Books::create();
    let target = key(90);
    let pending = books.change(&owner(), add(&target, Role::Owner, History::Full), T0);
    // Meanwhile the same person joins as a viewer.
    books.change(&owner(), add(&target, Role::Viewer, History::Full), T0 + 1);
    let due = T0 + OWNER_DELAY_SECS;
    assert!(books.state.activate(&pending.id, due).is_err());
    let lapsed = books.state.lapse(&pending.id, due).unwrap();
    assert!(lapsed.pending.is_empty());
    assert_eq!(
        lapsed.actor(&pk(&target)),
        protocol::Actor::Member(Role::Viewer)
    );
    // Not before it is due.
    assert!(books.state.lapse(&pending.id, due - 1).is_err());
}

fn add(keys: &Keys, role: Role, history: History) -> MembershipChange {
    MembershipChange::Add {
        member: pk(keys),
        role,
        history,
    }
}
