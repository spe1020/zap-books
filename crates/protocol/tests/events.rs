//! Proposals, acceptances, rejections and deliveries: building, opening, and
//! refusing anything altered or addressed to someone else.

mod common;

use common::*;
use nostr::event::FinalizeEvent;
use nostr::prelude::{Event, EventBuilder, Kind, Tag};
use protocol::{
    build_acceptance, build_delivery, build_proposal, build_rejection, encrypt_to, kinds,
    open_acceptance, open_delivery, open_proposal, open_rejection, parse_proposal,
    AcceptanceContent, AcceptanceHeader, Command, CommandType, CommittedPayload, EnvRecord,
    EpochKey, MembershipChange, Outcome, ProposalPayload, ProtocolError, Reason, RejectionCode,
    PROTOCOL_VERSION,
};

fn approve_payload(base_seq: u64) -> ProposalPayload {
    ProposalPayload {
        v: PROTOCOL_VERSION,
        salt: salt(),
        business: business_id(),
        base_seq: Some(base_seq),
        command: Command::Approve {
            expense_id: "draft-1".into(),
        },
    }
}

/// Re-signs `event` with its tags replaced, keeping the content and, unless
/// `tags` names one, its `payload` tag.
fn with_tags(keys: &nostr::prelude::Keys, event: &Event, tags: Vec<Vec<&str>>) -> Event {
    let mut tags: Vec<Tag> = tags.into_iter().map(|t| Tag::parse(t).unwrap()).collect();
    if !tags.iter().any(|t| t.kind() == "payload") {
        tags.extend(event.tags.iter().filter(|t| t.kind() == "payload").cloned());
    }
    EventBuilder::new(event.kind, event.content.clone())
        .tags(tags)
        .custom_created_at(event.created_at)
        .finalize(keys)
        .unwrap()
}

#[test]
fn a_proposal_round_trips_and_its_tags_route_it() {
    let event = build_proposal(&bookkeeper(), &pk(&service()), &approve_payload(4), at(5)).unwrap();
    assert_eq!(u16::from(event.kind), kinds::PROPOSAL);
    let header = parse_proposal(&event).unwrap();
    assert_eq!(header.command, CommandType::Approve);
    assert_eq!(header.base_seq, Some(4));
    assert_eq!(header.business, business_id());
    assert_eq!(header.author, pk(&bookkeeper()));
    // The business tag is single-letter, so relays index it (NIP-01 `#b`).
    let business = event.tags.iter().find(|t| t.kind() == "b").unwrap();
    assert!(business.single_letter_tag().is_some());
    assert_eq!(business.content(), Some(business_id().to_string().as_str()));
    let (_, payload) = open_proposal(&event, &service()).unwrap();
    assert_eq!(*payload.payload(), approve_payload(4));
}

#[test]
fn the_proposal_id_is_stable_across_resends() {
    let event = build_proposal(&bookkeeper(), &pk(&service()), &approve_payload(4), at(5)).unwrap();
    let resent = Event::from_json(event.as_json()).unwrap();
    assert_eq!(parse_proposal(&resent).unwrap().id, event.id);
}

#[test]
fn only_the_service_can_read_a_proposal() {
    let event = build_proposal(&bookkeeper(), &pk(&service()), &approve_payload(4), at(5)).unwrap();
    assert_eq!(
        open_proposal(&event, &outsider()).unwrap_err(),
        ProtocolError::Decrypt
    );
}

#[test]
fn an_altered_proposal_fails_its_signature() {
    let event = build_proposal(&bookkeeper(), &pk(&service()), &approve_payload(4), at(5)).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&event.as_json()).unwrap();
    json["tags"][2][1] = "0".into();
    let altered = Event::from_json(json.to_string()).unwrap();
    assert_eq!(
        parse_proposal(&altered).unwrap_err(),
        ProtocolError::BadSignature
    );
}

#[test]
fn tags_must_match_the_encrypted_command() {
    let keys = bookkeeper();
    let event = build_proposal(&keys, &pk(&service()), &approve_payload(4), at(5)).unwrap();
    let business = business_id().to_string();

    let wrong_seq = with_tags(
        &keys,
        &event,
        vec![
            vec!["b", &business],
            vec!["cmd", "Approve"],
            vec!["base_seq", "3"],
        ],
    );
    assert_eq!(
        open_proposal(&wrong_seq, &service()).unwrap_err(),
        ProtocolError::TagMismatch("base_seq")
    );

    let wrong_cmd = with_tags(
        &keys,
        &event,
        vec![
            vec!["b", &business],
            vec!["cmd", "SaveRule"],
            vec!["base_seq", "4"],
        ],
    );
    assert_eq!(
        open_proposal(&wrong_cmd, &service()).unwrap_err(),
        ProtocolError::TagMismatch("cmd")
    );

    let other = "cd".repeat(32);
    let wrong_business = with_tags(
        &keys,
        &event,
        vec![
            vec!["b", &other],
            vec!["cmd", "Approve"],
            vec!["base_seq", "4"],
        ],
    );
    assert_eq!(
        open_proposal(&wrong_business, &service()).unwrap_err(),
        ProtocolError::TagMismatch("b")
    );
}

#[test]
fn malformed_tags_are_refused() {
    let keys = bookkeeper();
    let event = build_proposal(&keys, &pk(&service()), &approve_payload(4), at(5)).unwrap();
    let business = business_id().to_string();
    let cases: Vec<(Vec<Vec<&str>>, ProtocolError)> = vec![
        (
            vec![vec!["cmd", "Approve"], vec!["base_seq", "4"]],
            ProtocolError::MissingTag("b"),
        ),
        (
            vec![vec!["b", &business], vec!["cmd", "Approve"]],
            ProtocolError::MissingTag("base_seq"),
        ),
        (
            vec![
                vec!["b", &business],
                vec!["cmd", "Approve"],
                vec!["base_seq", "04"],
            ],
            ProtocolError::BadTag("base_seq"),
        ),
        (
            vec![
                vec!["b", &business],
                vec!["cmd", "Approve"],
                vec!["base_seq", "4"],
                vec!["base_seq", "4"],
            ],
            ProtocolError::BadTag("base_seq"),
        ),
        (
            vec![
                vec!["b", &business],
                vec!["cmd", "Delete"],
                vec!["base_seq", "4"],
            ],
            ProtocolError::BadTag("cmd"),
        ),
        (
            vec![
                vec!["b", "AB"],
                vec!["cmd", "Approve"],
                vec!["base_seq", "4"],
            ],
            ProtocolError::BadTag("b"),
        ),
    ];
    for (tags, expected) in cases {
        let event = with_tags(&keys, &event, tags);
        assert_eq!(parse_proposal(&event).unwrap_err(), expected);
    }
}

#[test]
fn the_wrong_kind_is_refused() {
    let event = EventBuilder::new(Kind::TextNote, "hello")
        .finalize(&bookkeeper())
        .unwrap();
    assert_eq!(
        parse_proposal(&event).unwrap_err(),
        ProtocolError::WrongKind(1)
    );
}

#[test]
fn genesis_declares_the_service_key() {
    let event = genesis_event();
    let header = parse_proposal(&event).unwrap();
    assert_eq!(header.service, Some(pk(&service())));
    assert_eq!(header.base_seq, None);
    assert!(open_proposal(&event, &service()).is_ok());

    // Sent to a different service, it is refused before decrypting.
    assert_eq!(
        open_proposal(&event, &outsider()).unwrap_err(),
        ProtocolError::WrongService
    );

    // A genesis whose tag names another key than its content does.
    let mut payload = genesis_payload(backup_recovery());
    if let Command::CreateBusiness { service, .. } = &mut payload.command {
        *service = pk(&outsider());
    }
    let committed = CommittedPayload::new(payload).unwrap();
    let content = encrypt_to(owner().secret_key(), &pk(&service()), committed.text()).unwrap();
    let business = business_id().to_string();
    let service_hex = pk(&service()).to_hex();
    let hash = committed.hash();
    let forged = EventBuilder::new(Kind::from(kinds::PROPOSAL), content)
        .tags([
            Tag::parse(["b", business.as_str()]).unwrap(),
            Tag::parse(["cmd", "CreateBusiness"]).unwrap(),
            Tag::parse(["service", service_hex.as_str()]).unwrap(),
            Tag::parse(["payload", hash.as_str()]).unwrap(),
        ])
        .finalize(&owner())
        .unwrap();
    assert_eq!(
        open_proposal(&forged, &service()).unwrap_err(),
        ProtocolError::TagMismatch("service")
    );
}

fn acceptance_for(
    proposal: &Event,
    seq: u64,
    prev: Option<nostr::prelude::EventId>,
    epoch: &EpochKey,
) -> Event {
    let (header, payload) = open_proposal(proposal, &service()).unwrap();
    let content = AcceptanceContent {
        v: PROTOCOL_VERSION,
        proposal: proposal.clone(),
        payload,
        env: EnvRecord {
            ids: vec!["gen-1".into()],
            now: "2026-01-02T03:04:05.000Z".into(),
        },
        changes: Vec::new(),
        outcome: Outcome::Applied,
    };
    let acceptance = AcceptanceHeader {
        business: header.business,
        seq,
        prev,
        proposal: header.id,
        epoch: epoch.epoch,
        next_service: None,
    };
    build_acceptance(
        &service(),
        &epoch.public_key(),
        &acceptance,
        &content,
        at(10),
    )
    .unwrap()
}

#[test]
fn an_acceptance_opens_only_with_its_epoch_key() {
    let epoch0 = EpochKey::generate(0);
    let genesis = genesis_event();
    let event = acceptance_for(&genesis, 0, None, &epoch0);
    let (header, content) = open_acceptance(&event, &epoch0, &pk(&service())).unwrap();
    assert_eq!(header.seq, 0);
    assert_eq!(content.proposal.id, genesis.id);

    // Another key labelled epoch 0, and the right key under another number.
    let other = EpochKey::generate(0);
    assert_eq!(
        open_acceptance(&event, &other, &pk(&service())).unwrap_err(),
        ProtocolError::Decrypt
    );
    let renumbered = EpochKey::from_secret(1, epoch0.secret_key().clone());
    assert_eq!(
        open_acceptance(&event, &renumbered, &pk(&service())).unwrap_err(),
        ProtocolError::Decrypt
    );
}

#[test]
fn an_acceptance_must_name_its_proposal() {
    let epoch0 = EpochKey::generate(0);
    let (_, payload) = open_proposal(&genesis_event(), &service()).unwrap();
    let content = AcceptanceContent {
        v: PROTOCOL_VERSION,
        proposal: genesis_event(),
        payload,
        env: EnvRecord {
            ids: vec![],
            now: "2026-01-02T03:04:05.000Z".into(),
        },
        changes: vec![],
        outcome: Outcome::Applied,
    };
    let header = AcceptanceHeader {
        business: business_id(),
        seq: 0,
        prev: None,
        proposal: nostr::prelude::EventId::from_slice(&[0; 32]).unwrap(),
        epoch: 0,
        next_service: None,
    };
    assert!(build_acceptance(&service(), &epoch0.public_key(), &header, &content, at(10)).is_err());
}

/// Re-signs an acceptance with the service key, its `service` tag removed
/// and `next` (if any) put in its place: what whoever holds the service key
/// could forge.
fn with_handover(event: &Event, next: Option<&nostr::prelude::PublicKey>) -> Event {
    let mut tags: Vec<Tag> = event
        .tags
        .iter()
        .filter(|t| t.kind() != "service")
        .cloned()
        .collect();
    if let Some(next) = next {
        tags.push(Tag::parse(["service", &next.to_hex()]).unwrap());
    }
    EventBuilder::new(event.kind, event.content.clone())
        .tags(tags)
        .custom_created_at(event.created_at)
        .finalize(&service())
        .unwrap()
}

#[test]
fn a_service_handover_tag_must_match_an_activated_key_replacement() {
    let epoch0 = EpochKey::generate(0);
    let attacker = pk(&key(90));
    let next = pk(&key(91));

    // A hand-over forged onto an ordinary acceptance. It is validly signed
    // by the service key, so the chain alone follows it to the attacker's
    // key, but opening it (and so auditing it) fails.
    let genesis = acceptance_for(&genesis_event(), 0, None, &epoch0);
    let approve = build_proposal(&owner(), &pk(&service()), &approve_payload(0), at(10)).unwrap();
    let ordinary = acceptance_for(&approve, 1, Some(genesis.id), &epoch0);
    assert!(open_acceptance(&ordinary, &epoch0, &pk(&service())).is_ok());
    let forged = with_handover(&ordinary, Some(&attacker));
    let head = protocol::verify_chain(
        business_id(),
        pk(&service()),
        None,
        &[genesis, forged.clone()],
    )
    .unwrap()
    .unwrap();
    assert_eq!(head.service, attacker);
    assert_eq!(
        open_acceptance(&forged, &epoch0, &pk(&service())).unwrap_err(),
        ProtocolError::TagMismatch("service")
    );

    // The real hand-over: the activation of a ReplaceServiceKey, naming the
    // same key.
    let payload = ProposalPayload {
        v: PROTOCOL_VERSION,
        salt: salt(),
        business: business_id(),
        base_seq: Some(1),
        command: Command::ChangeMembership {
            change: MembershipChange::ReplaceServiceKey { key: next },
        },
    };
    let proposal = build_proposal(&owner(), &pk(&service()), &payload, at(10)).unwrap();
    let (_, committed) = open_proposal(&proposal, &service()).unwrap();
    let content = |outcome| AcceptanceContent {
        v: PROTOCOL_VERSION,
        proposal: proposal.clone(),
        payload: committed.clone(),
        env: EnvRecord {
            ids: vec![],
            now: "2026-01-02T03:04:05.000Z".into(),
        },
        changes: vec![],
        outcome,
    };
    let header = |next_service| AcceptanceHeader {
        business: business_id(),
        seq: 2,
        prev: Some(ordinary.id),
        proposal: proposal.id,
        epoch: 0,
        next_service,
    };
    let build = |next_service, outcome| {
        build_acceptance(
            &service(),
            &epoch0.public_key(),
            &header(next_service),
            &content(outcome),
            at(20),
        )
    };
    let handover = build(Some(next), Outcome::Activated).unwrap();
    assert_eq!(
        open_acceptance(&handover, &epoch0, &pk(&service()))
            .unwrap()
            .0
            .next_service,
        Some(next)
    );

    // The service refuses to build, and a reader to open: a tag naming
    // another key, a tag on the pending acceptance, and an activation
    // without its tag.
    let mismatch = Err(ProtocolError::TagMismatch("service"));
    let pending = Outcome::Pending { effective_at: 100 };
    assert_eq!(build(Some(attacker), Outcome::Activated), mismatch);
    assert_eq!(build(Some(next), pending), mismatch);
    assert_eq!(build(None, Outcome::Activated), mismatch);
    for forged in [
        with_handover(&handover, Some(&attacker)),
        with_handover(&build(None, pending).unwrap(), Some(&next)),
        with_handover(&handover, None),
    ] {
        assert_eq!(
            open_acceptance(&forged, &epoch0, &pk(&service())).unwrap_err(),
            ProtocolError::TagMismatch("service")
        );
    }
}

#[test]
fn a_rejection_reaches_only_its_proposer() {
    let proposal = build_proposal(&viewer(), &pk(&service()), &approve_payload(4), at(5)).unwrap();
    let header = parse_proposal(&proposal).unwrap();
    let event = build_rejection(
        &service(),
        &header,
        &Reason::rejected("Your role can't make this change."),
        at(6),
    )
    .unwrap();
    assert_eq!(u16::from(event.kind), kinds::REJECTION);
    let rejection = open_rejection(&event, &viewer(), &pk(&service())).unwrap();
    assert_eq!(rejection.proposal, proposal.id);
    assert_eq!(rejection.reason.code, RejectionCode::Rejected);
    assert_eq!(
        rejection.reason.message,
        "Your role can't make this change."
    );
    assert_eq!(
        (rejection.reason.size, rejection.reason.limit),
        (None, None)
    );
    assert!(open_rejection(&event, &bookkeeper(), &pk(&service())).is_err());
    assert_eq!(
        open_rejection(&event, &viewer(), &pk(&outsider())).unwrap_err(),
        ProtocolError::WrongService
    );
}

#[test]
fn an_epoch_key_delivery_opens_only_for_its_member() {
    let epoch = EpochKey::generate(3);
    let event = build_delivery(&service(), &pk(&viewer()), business_id(), &epoch, at(7)).unwrap();
    assert_eq!(u16::from(event.kind), kinds::EPOCH_KEY_DELIVERY);
    let delivery = open_delivery(&event, &viewer(), &pk(&service())).unwrap();
    assert_eq!(delivery.business, business_id());
    assert_eq!(delivery.key.epoch, 3);
    assert_eq!(delivery.key.public_key(), epoch.public_key());
    assert!(open_delivery(&event, &bookkeeper(), &pk(&service())).is_err());
    // The secret never appears in the event.
    assert!(!event
        .as_json()
        .contains(&epoch.secret_key().to_secret_hex()));
}

/// Re-signs `event` with the value of its `name` tag uppercased.
fn uppercased(keys: &nostr::prelude::Keys, event: &Event, name: &str) -> Event {
    let tags: Vec<Vec<String>> = event
        .tags
        .iter()
        .map(|t| {
            let mut t = t.as_slice().to_vec();
            if t[0] == name {
                t[1] = t[1].to_uppercase();
            }
            t
        })
        .collect();
    let tags = tags
        .iter()
        .map(|t| t.iter().map(String::as_str).collect())
        .collect();
    with_tags(keys, event, tags)
}

#[test]
fn key_and_event_id_tags_have_one_spelling() {
    // PROTO-6: lowercase hex only, as the business id already is.
    let genesis = genesis_event();
    assert_eq!(
        parse_proposal(&uppercased(&owner(), &genesis, "service")).unwrap_err(),
        ProtocolError::BadTag("service")
    );
    // Re-signed unchanged, it still parses: only the spelling is refused.
    assert!(parse_proposal(&uppercased(&owner(), &genesis, "none")).is_ok());

    let epoch = EpochKey::generate(0);
    let first = acceptance_for(&genesis, 0, None, &epoch);
    let next = build_proposal(&bookkeeper(), &pk(&service()), &approve_payload(0), at(5)).unwrap();
    let second = acceptance_for(&next, 1, Some(first.id), &epoch);
    assert!(protocol::parse_acceptance(&uppercased(&service(), &second, "none")).is_ok());
    for name in ["prev", "e"] {
        assert_eq!(
            protocol::parse_acceptance(&uppercased(&service(), &second, name)).unwrap_err(),
            ProtocolError::BadTag(name)
        );
    }

    let (header, _) = open_proposal(&next, &service()).unwrap();
    let rejection = build_rejection(
        &service(),
        &header,
        &Reason::rejected("Invented reason."),
        at(6),
    )
    .unwrap();
    let forged = uppercased(&service(), &rejection, "p");
    assert_eq!(
        open_rejection(&forged, &bookkeeper(), &pk(&service())).unwrap_err(),
        ProtocolError::BadTag("p")
    );
}
