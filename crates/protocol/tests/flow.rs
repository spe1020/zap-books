//! The pieces together, as the service and a reader would use them:
//! genesis, membership changes that rotate epochs, key deliveries, and a
//! reader re-deriving membership from the chain it can decrypt.

mod common;

use std::collections::BTreeMap;

use common::*;
use nostr::prelude::{Event, Keys};
use protocol::{
    build_acceptance, build_delivery, build_proposal, open_acceptance, open_delivery,
    open_proposal, verify_chain, AcceptanceContent, AcceptanceHeader, Command, CommittedPayload,
    EnvRecord, EpochKey, History, MembershipChange, MembershipState, Outcome, ProposalPayload,
    ProtocolError, Role, OWNER_DELAY_SECS, PROTOCOL_VERSION,
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
            state: MembershipState::genesis(
                business_id(),
                pk(&owner()),
                pk(&service()),
                backup_recovery(),
            )
            .unwrap(),
            epochs: vec![EpochKey::generate(0)],
            chain: vec![],
        };
        books.append(genesis, payload, Outcome::Applied, T0);
        books
    }

    fn append(&mut self, proposal: Event, payload: CommittedPayload, outcome: Outcome, now: u64) {
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
        self.change_sealed(signer, change, now, true)
    }

    /// As [`Books::change`]; with `rotate: false`, a change that starts an
    /// epoch is sealed under the old one, as a service that fails to rotate
    /// would.
    fn change_sealed(
        &mut self,
        signer: &Keys,
        change: MembershipChange,
        now: u64,
        rotate: bool,
    ) -> Event {
        let payload = ProposalPayload {
            v: PROTOCOL_VERSION,
            salt: salt(),
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
        if update.new_epoch && rotate {
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
            open_acceptance(e, key, &pk(&service()))
                .ok()
                .map(|(h, _)| h.seq)
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
        let (_, content) = open_acceptance(event, &keys[&header.epoch], &pk(&service())).unwrap();
        let at = event.created_at.as_secs();
        rebuilt = Some(match rebuilt {
            None => MembershipState::from_genesis(&content).unwrap(),
            Some(state) => {
                // follow checks the acceptance that starts an epoch is the
                // first under it.
                state.follow(&header, &content, at).unwrap()
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
        salt: salt(),
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
        payload: CommittedPayload::new(payload.clone()).unwrap(),
        env: EnvRecord {
            ids: vec![],
            now: "2027-01-15T08:00:00.000Z".into(),
        },
        changes: vec![],
        outcome: Outcome::Applied,
    };
    let header = next_header(&books, &content);
    assert_eq!(
        books.state.follow(&header, &content, T0).unwrap_err(),
        ProtocolError::NotMember
    );

    // Or an owner-level change applied without its wait.
    let owner_level = ProposalPayload {
        command: Command::ChangeMembership {
            change: add(&key(80), Role::Owner, History::Full),
        },
        ..payload
    };
    let content = AcceptanceContent {
        proposal: build_proposal(&owner(), &pk(&service()), &owner_level, at(T0)).unwrap(),
        payload: CommittedPayload::new(owner_level).unwrap(),
        ..content
    };
    let header = next_header(&books, &content);
    assert_eq!(
        books.state.follow(&header, &content, T0).unwrap_err(),
        ProtocolError::Forbidden
    );
}

/// The header of the next acceptance in `books`, in the current epoch.
fn next_header(books: &Books, content: &AcceptanceContent) -> AcceptanceHeader {
    AcceptanceHeader {
        business: business_id(),
        seq: books.chain.len() as u64,
        prev: books.chain.last().map(|e| e.id),
        proposal: content.proposal.id,
        epoch: books.state.epoch,
        next_service: None,
    }
}

/// Opens every acceptance with `keys` and re-derives the membership,
/// stopping at the first acceptance `follow` refuses.
fn follow_chain(
    chain: &[Event],
    keys: &BTreeMap<u64, EpochKey>,
) -> Result<MembershipState, (u64, ProtocolError)> {
    let mut state: Option<MembershipState> = None;
    for event in chain {
        let header = protocol::parse_acceptance(event).unwrap();
        let (header, content) =
            open_acceptance(event, &keys[&header.epoch], &pk(&service())).unwrap();
        state = Some(match state {
            None => MembershipState::from_genesis(&content).unwrap(),
            Some(state) => state
                .follow(&header, &content, event.created_at.as_secs())
                .map_err(|e| (header.seq, e))?,
        });
    }
    Ok(state.unwrap())
}

#[test]
fn a_service_that_stops_rotating_after_a_removal_is_caught() {
    // PROTO-2: the chain itself is well formed, so only follow can tell.
    let mut books = Books::create();
    books.change(
        &owner(),
        add(&bookkeeper(), Role::Bookkeeper, History::Full),
        T0,
    ); // 1
    books.change_sealed(
        &owner(),
        MembershipChange::Remove {
            member: pk(&bookkeeper()),
        },
        T0,
        false,
    ); // 2: starts epoch 1, but sealed under epoch 0
    books.change(&owner(), add(&key(73), Role::Viewer, History::Full), T0); // 3
    assert_eq!(books.state.epoch, 1);
    assert_eq!(books.epochs.len(), 1);

    verify_chain(business_id(), pk(&service()), None, &books.chain).unwrap();
    // The removed bookkeeper's old key still opens everything.
    let old: BTreeMap<u64, EpochKey> = [(0, books.epochs[0].clone())].into();
    assert_eq!(readable(&books.chain, &old), vec![0, 1, 2, 3]);
    assert_eq!(
        follow_chain(&books.chain, &old).unwrap_err(),
        (2, ProtocolError::WrongEpoch)
    );

    // Rotating as the rules say follows cleanly.
    let mut honest = Books::create();
    honest.change(
        &owner(),
        add(&bookkeeper(), Role::Bookkeeper, History::Full),
        T0,
    );
    honest.change(
        &owner(),
        MembershipChange::Remove {
            member: pk(&bookkeeper()),
        },
        T0,
    );
    let keys = honest.keys_for(&owner());
    assert_eq!(follow_chain(&honest.chain, &keys).unwrap(), honest.state);
}

#[test]
fn an_epoch_tag_ahead_of_the_rules_is_caught_too() {
    // PROTO-2, the other direction: an ordinary change sealed under a new
    // epoch the rules never started.
    let mut books = Books::create();
    books.epochs.push(EpochKey::generate(1));
    books.change(
        &owner(),
        add(&bookkeeper(), Role::Bookkeeper, History::Full),
        T0,
    );
    let keys: BTreeMap<u64, EpochKey> = books.epochs.iter().map(|k| (k.epoch, k.clone())).collect();
    verify_chain(business_id(), pk(&service()), None, &books.chain).unwrap();
    assert_eq!(
        follow_chain(&books.chain, &keys).unwrap_err(),
        (1, ProtocolError::WrongEpoch)
    );
}

#[test]
fn a_pending_change_waits_exactly_48_hours() {
    // PROTO-8: neither shorter nor longer than the service sets.
    let books = Books::create();
    let payload = ProposalPayload {
        v: PROTOCOL_VERSION,
        salt: salt(),
        business: business_id(),
        base_seq: Some(0),
        command: Command::ChangeMembership {
            change: add(&key(81), Role::Owner, History::Full),
        },
    };
    let content = |effective_at| AcceptanceContent {
        v: PROTOCOL_VERSION,
        proposal: build_proposal(&owner(), &pk(&service()), &payload, at(T0)).unwrap(),
        payload: CommittedPayload::new(payload.clone()).unwrap(),
        env: EnvRecord {
            ids: vec![],
            now: "2027-01-15T08:00:00.000Z".into(),
        },
        changes: vec![],
        outcome: Outcome::Pending { effective_at },
    };
    let exact = content(T0 + OWNER_DELAY_SECS);
    let header = next_header(&books, &exact);
    let state = books.state.follow(&header, &exact, T0).unwrap();
    assert_eq!(state.due(T0 + OWNER_DELAY_SECS).len(), 1);
    for effective_at in [
        T0 + OWNER_DELAY_SECS - 1,
        T0 + OWNER_DELAY_SECS + 1,
        u64::MAX,
    ] {
        assert_eq!(
            books
                .state
                .follow(&header, &content(effective_at), T0)
                .unwrap_err(),
            ProtocolError::Forbidden,
            "effectiveAt {effective_at}"
        );
    }
}

#[test]
fn an_epoch_at_its_maximum_is_an_error_not_a_wrap() {
    // PROTO-5.
    let mut state = MembershipState::genesis(
        business_id(),
        pk(&owner()),
        pk(&service()),
        backup_recovery(),
    )
    .unwrap();
    state.epoch = u64::MAX;
    let joined = state
        .propose(
            genesis_event().id,
            &pk(&owner()),
            &add(&bookkeeper(), Role::Bookkeeper, History::Full),
            T0,
        )
        .unwrap()
        .state;
    for change in [
        MembershipChange::Remove {
            member: pk(&bookkeeper()),
        },
        add(&viewer(), Role::Viewer, History::FromJoin),
    ] {
        assert_eq!(
            joined
                .propose(genesis_event().id, &pk(&owner()), &change, T0)
                .unwrap_err(),
            ProtocolError::Overflow("epoch")
        );
    }
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

impl Books {
    /// Proposes and accepts any other command as the service would, without
    /// running it on the books: `follow` checks only who may propose it.
    fn accept(&mut self, signer: &Keys, command: Command, now: u64) {
        let payload = ProposalPayload {
            v: PROTOCOL_VERSION,
            salt: salt(),
            business: business_id(),
            base_seq: Some(self.chain.len() as u64 - 1),
            command,
        };
        let proposal = build_proposal(signer, &pk(&service()), &payload, at(now)).unwrap();
        let (header, payload) = open_proposal(&proposal, &service()).unwrap();
        self.state
            .authorize(&header.author, &payload.command)
            .unwrap();
        self.state = self.state.accepted(&header.author, &payload.command);
        self.append(proposal, payload, Outcome::Applied, now);
    }
}

/// An acceptance of `command`, signed by `signer`, that the service should
/// have rejected, as the next link of `books`.
fn forged(books: &Books, signer: &Keys, command: Command) -> (AcceptanceHeader, AcceptanceContent) {
    let payload = ProposalPayload {
        v: PROTOCOL_VERSION,
        salt: salt(),
        business: business_id(),
        base_seq: Some(books.chain.len() as u64 - 1),
        command,
    };
    let content = AcceptanceContent {
        v: PROTOCOL_VERSION,
        proposal: build_proposal(signer, &pk(&service()), &payload, at(T0)).unwrap(),
        payload: CommittedPayload::new(payload).unwrap(),
        env: EnvRecord {
            ids: vec![],
            now: "2027-01-15T08:00:00.000Z".into(),
        },
        changes: vec![],
        outcome: Outcome::Applied,
    };
    (next_header(books, &content), content)
}

#[test]
fn a_reader_rebuilds_which_drafts_an_assistant_may_edit() {
    let mut books = Books::create();
    books.change(
        &owner(),
        add(&assistant(), Role::Assistant, History::Full),
        T0,
    );
    let (seed, _) = seed_business();
    let mut own = seed.drafts[0].clone();
    own.id = "assistant-draft".into();
    books.accept(
        &assistant(),
        Command::SubmitReceiptDraft {
            draft: own.clone(),
            documents: vec![],
        },
        T0,
    );
    let save = |draft: &ledger_core::ExpenseDraft| Command::SaveDraft {
        draft: draft.clone(),
        documents: vec![],
        extraction: None,
    };
    books.accept(&assistant(), save(&own), T0);

    let rebuilt = follow_chain(&books.chain, &books.keys_for(&owner())).unwrap();
    assert_eq!(rebuilt, books.state);
    assert_eq!(
        rebuilt.assistant_drafts.get("assistant-draft"),
        Some(&pk(&assistant()))
    );

    // A service that let the assistant edit a person's draft is caught.
    let (header, content) = forged(&books, &assistant(), save(&seed.drafts[0]));
    assert_eq!(
        rebuilt.follow(&header, &content, T0).unwrap_err(),
        ProtocolError::AssistantOwnDraftsOnly
    );
    // So is one that let it approve its own draft.
    let (header, content) = forged(
        &books,
        &assistant(),
        Command::Approve {
            expense_id: own.id.clone(),
        },
    );
    assert_eq!(
        rebuilt.follow(&header, &content, T0).unwrap_err(),
        ProtocolError::Forbidden
    );
}

#[test]
fn a_reader_drops_an_assistants_drafts_when_a_person_saves_one_or_it_leaves() {
    let mut books = Books::create(); // 0
    books.change(
        &owner(),
        add(&assistant(), Role::Assistant, History::Full),
        T0,
    ); // 1
    let (seed, _) = seed_business();
    let draft = |id: &str| {
        let mut d = seed.drafts[0].clone();
        d.id = id.into();
        d
    };
    let submit = |d: ledger_core::ExpenseDraft| Command::SubmitReceiptDraft {
        draft: d,
        documents: vec![],
    };
    let save = |d: ledger_core::ExpenseDraft| Command::SaveDraft {
        draft: d,
        documents: vec![],
        extraction: None,
    };
    books.accept(&assistant(), submit(draft("taken-over")), T0); // 2
    books.accept(&assistant(), submit(draft("kept")), T0); // 3
                                                           // The owner saves one of them: it is no longer the assistant's.
    books.accept(&owner(), save(draft("taken-over")), T0); // 4

    let rebuilt = follow_chain(&books.chain, &books.keys_for(&owner())).unwrap();
    assert_eq!(rebuilt, books.state);
    assert_eq!(
        rebuilt.assistant_drafts.keys().collect::<Vec<_>>(),
        vec!["kept"]
    );
    // A service that let the assistant edit it again is caught.
    let (header, content) = forged(&books, &assistant(), save(draft("taken-over")));
    assert_eq!(
        rebuilt.follow(&header, &content, T0).unwrap_err(),
        ProtocolError::AssistantOwnDraftsOnly
    );

    // Removed and added back, it owns nothing.
    books.change(
        &owner(),
        MembershipChange::Remove {
            member: pk(&assistant()),
        },
        T0,
    ); // 5, epoch 1
    books.change(
        &owner(),
        add(&assistant(), Role::Assistant, History::Full),
        T0,
    ); // 6
    let rebuilt = follow_chain(&books.chain, &books.keys_for(&owner())).unwrap();
    assert_eq!(rebuilt, books.state);
    assert!(rebuilt.assistant_drafts.is_empty());
    let (header, content) = forged(&books, &assistant(), save(draft("kept")));
    assert_eq!(
        rebuilt.follow(&header, &content, T0).unwrap_err(),
        ProtocolError::AssistantOwnDraftsOnly
    );

    // So too after a role change away and back.
    books.accept(&assistant(), submit(draft("again")), T0); // 7
    for role in [Role::Viewer, Role::Assistant] {
        books.change(
            &owner(),
            MembershipChange::ChangeRole {
                member: pk(&assistant()),
                role,
            },
            T0,
        ); // 8, 9
    }
    let rebuilt = follow_chain(&books.chain, &books.keys_for(&owner())).unwrap();
    assert_eq!(rebuilt, books.state);
    assert!(rebuilt.assistant_drafts.is_empty());
    let (header, content) = forged(&books, &assistant(), save(draft("again")));
    assert_eq!(
        rebuilt.follow(&header, &content, T0).unwrap_err(),
        ProtocolError::AssistantOwnDraftsOnly
    );
}
