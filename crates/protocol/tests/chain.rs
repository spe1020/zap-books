//! Chain verification: any altered, missing or reordered acceptance is
//! detected without decrypting anything.

mod common;

use common::*;
use nostr::prelude::{Event, EventId, Keys, PublicKey};
use protocol::{
    build_acceptance, build_proposal, open_proposal, verify_chain, AcceptanceContent,
    AcceptanceHeader, ChainHead, Command, EnvRecord, EpochKey, MembershipChange, Outcome,
    ProposalPayload, ProtocolError, PROTOCOL_VERSION,
};

fn proposal(seq: u64) -> Event {
    let payload = ProposalPayload {
        v: PROTOCOL_VERSION,
        salt: salt(),
        business: business_id(),
        base_seq: Some(seq),
        command: Command::Approve {
            expense_id: format!("draft-{seq}"),
        },
    };
    build_proposal(&bookkeeper(), &pk(&service()), &payload, at(100 + seq)).unwrap()
}

fn accept(
    signer: &Keys,
    proposal: &Event,
    seq: u64,
    prev: Option<EventId>,
    next_service: Option<PublicKey>,
) -> Event {
    accept_in(0, signer, proposal, seq, prev, next_service)
}

fn accept_in(
    epoch: u64,
    signer: &Keys,
    proposal: &Event,
    seq: u64,
    prev: Option<EventId>,
    next_service: Option<PublicKey>,
) -> Event {
    let (header, payload) = open_proposal(proposal, &service()).unwrap();
    let content = AcceptanceContent {
        v: PROTOCOL_VERSION,
        proposal: proposal.clone(),
        payload,
        env: EnvRecord {
            ids: vec![],
            now: "2026-01-02T03:04:05.000Z".into(),
        },
        changes: vec![],
        outcome: Outcome::Applied,
    };
    let acceptance = AcceptanceHeader {
        business: header.business,
        seq,
        prev,
        proposal: header.id,
        epoch,
        next_service,
    };
    let epoch = EpochKey::generate(epoch);
    build_acceptance(
        signer,
        &epoch.public_key(),
        &acceptance,
        &content,
        at(200 + seq),
    )
    .unwrap()
}

/// The acceptance activating a replacement of the service key with `next`,
/// signed by the current key: the only one that may carry a `service` tag.
fn handover(next: PublicKey, seq: u64, prev: EventId) -> Event {
    let payload = ProposalPayload {
        v: PROTOCOL_VERSION,
        salt: salt(),
        business: business_id(),
        base_seq: Some(seq - 1),
        command: Command::ChangeMembership {
            change: MembershipChange::ReplaceServiceKey { key: next },
        },
    };
    let proposal = build_proposal(&owner(), &pk(&service()), &payload, at(100 + seq)).unwrap();
    let (header, payload) = open_proposal(&proposal, &service()).unwrap();
    let content = AcceptanceContent {
        v: PROTOCOL_VERSION,
        proposal,
        payload,
        env: EnvRecord {
            ids: vec![],
            now: "2026-01-02T03:04:05.000Z".into(),
        },
        changes: vec![],
        outcome: Outcome::Activated,
    };
    let acceptance = AcceptanceHeader {
        business: header.business,
        seq,
        prev: Some(prev),
        proposal: header.id,
        epoch: 0,
        next_service: Some(next),
    };
    let epoch = EpochKey::generate(0);
    build_acceptance(
        &service(),
        &epoch.public_key(),
        &acceptance,
        &content,
        at(200 + seq),
    )
    .unwrap()
}

/// A valid chain of `n` acceptances: genesis, then approvals.
fn chain(n: u64) -> Vec<Event> {
    let mut out: Vec<Event> = Vec::new();
    for seq in 0..n {
        let p = if seq == 0 {
            genesis_event()
        } else {
            proposal(seq - 1)
        };
        let prev = out.last().map(|e| e.id);
        out.push(accept(&service(), &p, seq, prev, None));
    }
    out
}

#[test]
fn a_valid_chain_verifies_and_can_be_continued() {
    let events = chain(5);
    let head = verify_chain(business_id(), pk(&service()), None, &events)
        .unwrap()
        .unwrap();
    assert_eq!(head.seq, 4);
    assert_eq!(head.id, events[4].id);

    let first = verify_chain(business_id(), pk(&service()), None, &events[..2])
        .unwrap()
        .unwrap();
    let rest = verify_chain(business_id(), pk(&service()), Some(first), &events[2..])
        .unwrap()
        .unwrap();
    assert_eq!(rest, head);
}

#[test]
fn tampered_content_is_detected() {
    let mut events = chain(4);
    let mut json: serde_json::Value = serde_json::from_str(&events[2].as_json()).unwrap();
    let content = json["content"].as_str().unwrap().to_string();
    // Change the last character, whatever it is.
    let last = if content.ends_with('A') { "B" } else { "A" };
    json["content"] = format!("{}{last}", &content[..content.len() - 1]).into();
    events[2] = Event::from_json(json.to_string()).unwrap();
    assert_eq!(
        verify_chain(business_id(), pk(&service()), None, &events).unwrap_err(),
        ProtocolError::BadSignature
    );
}

#[test]
fn a_deleted_acceptance_is_detected() {
    let mut events = chain(4);
    events.remove(2);
    assert_eq!(
        verify_chain(business_id(), pk(&service()), None, &events).unwrap_err(),
        ProtocolError::ChainSeq {
            expected: 2,
            found: 3
        }
    );
    // Removing the last one leaves a valid, shorter chain: the reader
    // compares the head with the service's reported head.
    let mut short = chain(4);
    short.pop();
    assert!(verify_chain(business_id(), pk(&service()), None, &short).is_ok());
}

#[test]
fn reordered_acceptances_are_detected() {
    let mut events = chain(4);
    events.swap(1, 2);
    assert!(matches!(
        verify_chain(business_id(), pk(&service()), None, &events).unwrap_err(),
        ProtocolError::ChainSeq { .. }
    ));
}

#[test]
fn a_replaced_acceptance_breaks_the_link() {
    // The service (or anyone with its key) re-signs seq 2 for another
    // proposal: seq 3 still names the original as `prev`.
    let mut events = chain(4);
    events[2] = accept(&service(), &proposal(9), 2, Some(events[1].id), None);
    assert_eq!(
        verify_chain(business_id(), pk(&service()), None, &events).unwrap_err(),
        ProtocolError::ChainPrev(3)
    );
}

#[test]
fn an_acceptance_signed_by_another_key_is_detected() {
    let mut events = chain(3);
    events[2] = accept(&outsider(), &proposal(1), 2, Some(events[1].id), None);
    assert_eq!(
        verify_chain(business_id(), pk(&service()), None, &events).unwrap_err(),
        ProtocolError::ChainSigner(2)
    );
}

#[test]
fn another_business_chain_is_refused() {
    let events = chain(2);
    let other = protocol::BusinessId::from_bytes([0xcd; 32]);
    assert_eq!(
        verify_chain(other, pk(&service()), None, &events).unwrap_err(),
        ProtocolError::ChainBusiness(0)
    );
}

#[test]
fn a_service_key_replacement_hands_over_the_chain() {
    let new_service = key(9);
    let mut events = chain(2);
    events.push(handover(pk(&new_service), 2, events[1].id));
    events.push(accept(
        &new_service,
        &proposal(2),
        3,
        Some(events[2].id),
        None,
    ));
    let head = verify_chain(business_id(), pk(&service()), None, &events)
        .unwrap()
        .unwrap();
    assert_eq!(head.service, pk(&new_service));

    // After the handover the old key's signature no longer counts.
    events[3] = accept(&service(), &proposal(2), 3, Some(events[2].id), None);
    assert_eq!(
        verify_chain(business_id(), pk(&service()), None, &events).unwrap_err(),
        ProtocolError::ChainSigner(3)
    );
}

#[test]
fn genesis_is_epoch_0_and_each_acceptance_starts_at_most_one_epoch() {
    // PROTO-2: one acceptance can start at most one epoch.
    let mut events = chain(2);
    events.push(accept_in(
        1,
        &service(),
        &proposal(2),
        2,
        Some(events[1].id),
        None,
    ));
    verify_chain(business_id(), pk(&service()), None, &events).unwrap();

    events[2] = accept_in(2, &service(), &proposal(2), 2, Some(events[1].id), None);
    assert_eq!(
        verify_chain(business_id(), pk(&service()), None, &events).unwrap_err(),
        ProtocolError::ChainEpoch(2)
    );

    let late = vec![accept_in(1, &service(), &genesis_event(), 0, None, None)];
    assert_eq!(
        verify_chain(business_id(), pk(&service()), None, &late).unwrap_err(),
        ProtocolError::ChainEpoch(0)
    );
}

#[test]
fn a_head_at_the_last_seq_is_an_error_not_a_wrap() {
    // PROTO-5: the next seq after u64::MAX is refused, not computed.
    let events = chain(1);
    let head = ChainHead {
        seq: u64::MAX,
        id: events[0].id,
        epoch: 0,
        service: pk(&service()),
    };
    assert_eq!(
        verify_chain(business_id(), pk(&service()), Some(head), &events).unwrap_err(),
        ProtocolError::Overflow("seq")
    );
}
