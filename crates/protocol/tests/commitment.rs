//! The payload commitment: a proposal's signed `payload` tag is the SHA-256
//! of its exact plaintext payload, salt included. A reader holding only the
//! epoch key checks that the command in an acceptance is the one the member
//! signed, without being able to decrypt the proposal itself.

mod common;

use common::*;
use nostr::event::FinalizeEvent;
use nostr::prelude::{Event, EventBuilder, Tag};
use protocol::{
    build_acceptance, build_proposal, decrypt_from, open_acceptance, open_proposal, parse_proposal,
    AcceptanceContent, AcceptanceHeader, Command, CommittedPayload, EnvRecord, EpochKey, Outcome,
    ProposalPayload, ProtocolError, PROTOCOL_VERSION,
};

fn approve(expense: &str) -> Command {
    Command::Approve {
        expense_id: expense.into(),
    }
}

fn proposal() -> Event {
    let payload = ProposalPayload::new(business_id(), Some(3), approve("draft-1"));
    build_proposal(&bookkeeper(), &pk(&service()), &payload, at(10)).unwrap()
}

/// An acceptance of `proposal` carrying `payload` as its command, whatever
/// the proposal actually said.
fn accept(proposal: &Event, payload: CommittedPayload, epoch: &EpochKey) -> Event {
    let header = AcceptanceHeader {
        business: business_id(),
        seq: 4,
        prev: Some(nostr::prelude::EventId::from_slice(&[7; 32]).unwrap()),
        proposal: proposal.id,
        epoch: epoch.epoch,
        next_service: None,
    };
    let content = AcceptanceContent {
        v: PROTOCOL_VERSION,
        proposal: proposal.clone(),
        payload,
        env: EnvRecord {
            ids: vec![],
            now: "2023-03-01T09:00:00.000Z".into(),
        },
        changes: vec![],
        outcome: Outcome::Applied,
    };
    build_acceptance(&service(), &epoch.public_key(), &header, &content, at(11)).unwrap()
}

#[test]
fn the_command_the_member_signed_verifies() {
    let epoch = EpochKey::generate(0);
    let proposal = proposal();
    let (_, payload) = open_proposal(&proposal, &service()).unwrap();
    let event = accept(&proposal, payload, &epoch);

    let (_, content) = open_acceptance(&event, &epoch, &pk(&service())).unwrap();
    assert_eq!(content.payload.command, approve("draft-1"));
    assert_eq!(
        content.payload.hash(),
        parse_proposal(&content.proposal).unwrap().payload_hash
    );

    // The reader verified it without being able to read the proposal: its
    // content is encrypted to the service, and the reader holds only the
    // epoch key.
    let reader = viewer();
    assert!(decrypt_from(reader.secret_key(), &pk(&bookkeeper()), &proposal.content).is_err());
    assert!(decrypt_from(epoch.secret_key(), &pk(&bookkeeper()), &proposal.content).is_err());
}

#[test]
fn a_substituted_command_fails() {
    let epoch = EpochKey::generate(0);
    let proposal = proposal();
    let (_, genuine) = open_proposal(&proposal, &service()).unwrap();
    // The service records another expense's approval under this proposal,
    // keeping the salt, business and baseSeq, so every tag still agrees.
    let substituted = CommittedPayload::new(ProposalPayload {
        command: approve("draft-2"),
        ..genuine.payload().clone()
    })
    .unwrap();
    let event = accept(&proposal, substituted, &epoch);
    assert_eq!(
        open_acceptance(&event, &epoch, &pk(&service())).unwrap_err(),
        ProtocolError::PayloadMismatch
    );
}

#[test]
fn a_changed_salt_fails() {
    let epoch = EpochKey::generate(0);
    let proposal = proposal();
    let (_, genuine) = open_proposal(&proposal, &service()).unwrap();
    let resalted = CommittedPayload::new(ProposalPayload {
        salt: "00".repeat(32),
        ..genuine.payload().clone()
    })
    .unwrap();
    let event = accept(&proposal, resalted, &epoch);
    assert_eq!(
        open_acceptance(&event, &epoch, &pk(&service())).unwrap_err(),
        ProtocolError::PayloadMismatch
    );
}

#[test]
fn the_exact_text_is_committed_not_its_meaning() {
    // The same payload with its keys reordered is a different text, so it
    // does not match: acceptances must carry the proposer's text unchanged.
    let epoch = EpochKey::generate(0);
    let proposal = proposal();
    let (_, genuine) = open_proposal(&proposal, &service()).unwrap();
    let value: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(genuine.text()).unwrap();
    let reordered: serde_json::Map<_, _> = value.into_iter().rev().collect();
    let text = serde_json::to_string(&reordered).unwrap();
    assert_ne!(text, genuine.text());
    let same_meaning = CommittedPayload::from_text(text).unwrap();
    assert_eq!(same_meaning.payload(), genuine.payload());
    let event = accept(&proposal, same_meaning, &epoch);
    assert_eq!(
        open_acceptance(&event, &epoch, &pk(&service())).unwrap_err(),
        ProtocolError::PayloadMismatch
    );
}

#[test]
fn the_service_checks_the_commitment_too() {
    // A proposal whose payload tag doesn't match its encrypted content.
    let genuine = proposal();
    let tags: Vec<Tag> = genuine
        .tags
        .iter()
        .map(|t| {
            if t.kind() == "payload" {
                Tag::parse(["payload", "ab".repeat(32).as_str()]).unwrap()
            } else {
                t.clone()
            }
        })
        .collect();
    let forged = EventBuilder::new(genuine.kind, genuine.content.clone())
        .tags(tags)
        .custom_created_at(genuine.created_at)
        .finalize(&bookkeeper())
        .unwrap();
    assert_eq!(
        open_proposal(&forged, &service()).unwrap_err(),
        ProtocolError::PayloadMismatch
    );
}

#[test]
fn every_payload_gets_its_own_salt() {
    let a = ProposalPayload::new(business_id(), Some(3), approve("draft-1"));
    let b = ProposalPayload::new(business_id(), Some(3), approve("draft-1"));
    assert_ne!(a.salt, b.salt);
    assert_eq!(a.salt.len(), 64);
    // So two identical commands have unrelated public hashes.
    let ha = CommittedPayload::new(a).unwrap().hash();
    let hb = CommittedPayload::new(b).unwrap().hash();
    assert_ne!(ha, hb);
}

#[test]
fn a_missing_or_malformed_salt_is_refused() {
    let mut payload = ProposalPayload::new(business_id(), Some(3), approve("draft-1"));
    payload.salt = "short".into();
    assert!(matches!(
        build_proposal(&bookkeeper(), &pk(&service()), &payload, at(1)).unwrap_err(),
        ProtocolError::BadContent(_)
    ));
    // The same payload with its salt removed no longer parses.
    payload.salt = "5a".repeat(32);
    let mut value = serde_json::to_value(&payload).unwrap();
    assert!(CommittedPayload::from_text(value.to_string()).is_ok());
    value.as_object_mut().unwrap().remove("salt");
    assert_eq!(
        CommittedPayload::from_text(value.to_string()).unwrap_err(),
        ProtocolError::BadContent("it is not valid JSON for this protocol version")
    );
}

#[test]
fn unreadable_content_never_echoes_its_text() {
    // PROTO-4: serde quotes the offending token; the error must not.
    let mut value = serde_json::to_value(ProposalPayload::new(
        business_id(),
        Some(3),
        approve("draft-1"),
    ))
    .unwrap();
    let cases = [
        serde_json::json!({"type": "Invented Vendor Text"}),
        serde_json::json!({"type": "Approve", "expenseId": 987654321}),
    ];
    for command in cases {
        value["command"] = command;
        let text = value.to_string();
        let err = CommittedPayload::from_text(text).unwrap_err();
        let reason = protocol::Reason::from_error(&err);
        for shown in [err.to_string(), format!("{err:?}"), reason.message] {
            for fragment in ["Invented", "987654321", "Vendor"] {
                assert!(!shown.contains(fragment), "{shown}");
            }
        }
    }
}
