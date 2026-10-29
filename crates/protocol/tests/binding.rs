//! An acceptance is bound to one signer and one business. A reader refuses
//! one signed by anyone but the service, and one that embeds a member's
//! genuine proposal for another business, at every stage: opening it,
//! following the membership, and auditing the books.

mod common;

use common::*;
use ledger_core::{BusinessData, SettingsChange};
use nostr::event::FinalizeEvent;
use nostr::prelude::{Event, EventBuilder, Keys, Kind, Tag};
use protocol::{
    apply_command, audit, build_acceptance, build_proposal, diff, encrypt_to, genesis_business,
    kinds, open_acceptance, open_proposal, AcceptanceContent, AcceptanceHeader, BusinessId,
    Command, EnvRecord, EpochKey, MembershipState, Outcome, ProposalPayload, ProtocolError,
    RecordingEnv, Refusal, PROTOCOL_VERSION,
};

const NOW: &str = "2023-03-01T09:00:00.000Z";

fn uuids() -> impl FnMut() -> String {
    let mut n = 0u64;
    move || {
        n += 1;
        format!("00000000-0000-4000-8000-{n:012}")
    }
}

/// The genesis command with the seed's records carrying our business id.
fn genesis_command() -> Command {
    let mut payload = genesis_payload(backup_recovery());
    if let Command::CreateBusiness {
        profile,
        accounts,
        categories,
        ..
    } = &mut payload.command
    {
        let id = business_id().to_string();
        profile.id = id.clone();
        profile.business_id = id.clone();
        for a in accounts.iter_mut() {
            a.business_id = id.clone();
        }
        for c in categories.iter_mut() {
            c.business_id = id.clone();
        }
    }
    payload.command
}

/// Signs an acceptance with `signer`, carrying `content` encrypted from
/// `signer` to `epoch`, under the tags a real acceptance has: what a
/// dishonest service, or anyone holding the epoch public key, can produce
/// where `build_acceptance` refuses.
fn forge(
    signer: &Keys,
    epoch: &EpochKey,
    header: &AcceptanceHeader,
    content: &AcceptanceContent,
) -> Event {
    let mut tags = vec![
        Tag::parse(["b", &header.business.to_string()]).unwrap(),
        Tag::parse(["seq", &header.seq.to_string()]).unwrap(),
    ];
    if let Some(prev) = header.prev {
        tags.push(Tag::parse(["prev", &prev.to_hex()]).unwrap());
    }
    tags.push(Tag::event(header.proposal));
    tags.push(Tag::parse(["epoch", &header.epoch.to_string()]).unwrap());
    let text = serde_json::to_string(content).unwrap();
    let sealed = encrypt_to(signer.secret_key(), &epoch.public_key(), &text).unwrap();
    EventBuilder::new(Kind::from(kinds::ACCEPTANCE), sealed)
        .tags(tags)
        .custom_created_at(at(10))
        .finalize(signer)
        .unwrap()
}

fn content(
    proposal: Event,
    env: EnvRecord,
    changes: Vec<protocol::RecordChange>,
) -> AcceptanceContent {
    let (_, payload) = open_proposal(&proposal, &service()).unwrap();
    AcceptanceContent {
        v: PROTOCOL_VERSION,
        proposal,
        payload,
        env,
        changes,
        outcome: Outcome::Applied,
    }
}

fn no_env() -> EnvRecord {
    EnvRecord {
        ids: vec![],
        now: NOW.into(),
    }
}

/// The genesis acceptance of business A, built by the service.
fn genesis_acceptance(epoch: &EpochKey) -> Event {
    // One genesis event: each build encrypts with a fresh nonce, so two
    // calls give two ids.
    let genesis = genesis_event();
    let header = AcceptanceHeader {
        business: business_id(),
        seq: 0,
        prev: None,
        proposal: genesis.id,
        epoch: 0,
        next_service: None,
    };
    let content = content(genesis, no_env(), vec![]);
    build_acceptance(&service(), &epoch.public_key(), &header, &content, at(5)).unwrap()
}

#[test]
fn an_acceptance_must_be_signed_by_the_service() {
    let epoch0 = EpochKey::generate(0);
    let genesis = genesis_event();
    let header = AcceptanceHeader {
        business: business_id(),
        seq: 0,
        prev: None,
        proposal: genesis.id,
        epoch: 0,
        next_service: None,
    };
    let content = content(genesis, no_env(), vec![]);

    // The forge produces what the service produces.
    let real = forge(&service(), &epoch0, &header, &content);
    assert!(open_acceptance(&real, &epoch0, &pk(&service())).is_ok());

    // A viewer holds the epoch key, so they can encrypt to it and sign an
    // acceptance of their own. The chain's expected signer refuses it.
    let forged = forge(&viewer(), &epoch0, &header, &content);
    assert_eq!(
        open_acceptance(&forged, &epoch0, &pk(&service())).unwrap_err(),
        ProtocolError::WrongService
    );
    // Only the signer differs: told to expect the viewer, it opens.
    assert!(open_acceptance(&forged, &epoch0, &pk(&viewer())).is_ok());
    // The real one, checked against the wrong key, is refused too.
    assert_eq!(
        open_acceptance(&real, &epoch0, &pk(&viewer())).unwrap_err(),
        ProtocolError::WrongService
    );
}

/// Business A's books and state, and the owner's genuine, correctly
/// signed proposal for business B, recorded as if it were A's: the
/// command applies cleanly to A's books, so nothing but the business
/// binding tells the two apart. Returns A's books, A's state, the forged
/// acceptance's header, its content, and the same content made for A.
struct CrossBusiness {
    books: BusinessData,
    state: MembershipState,
    epoch: EpochKey,
    header: AcceptanceHeader,
    other: AcceptanceContent,
    own: AcceptanceContent,
}

fn cross_business() -> CrossBusiness {
    let epoch = EpochKey::generate(0);
    let genesis = genesis_acceptance(&epoch);
    let books = genesis_business(business_id(), &genesis_command()).unwrap();
    let state = MembershipState::genesis(
        business_id(),
        pk(&owner()),
        pk(&service()),
        backup_recovery(),
    )
    .unwrap();
    let command = Command::UpdateSettings {
        change: SettingsChange::Profile {
            name: "Renamed Studio".into(),
            industry: "Design".into(),
        },
    };
    // What the service records when it applies the command to A's books.
    let mut env = RecordingEnv::new(uuids(), NOW);
    let after = apply_command(&books, &command, &mut env).unwrap();
    let changes = diff(&books, &after).unwrap();
    let record = env.into_record();
    let proposal = |business| {
        let payload = ProposalPayload {
            v: PROTOCOL_VERSION,
            salt: salt(),
            business,
            base_seq: Some(0),
            command: command.clone(),
        };
        build_proposal(&owner(), &pk(&service()), &payload, at(10)).unwrap()
    };
    let other = content(
        proposal(BusinessId::from_bytes([0xcd; 32])),
        record.clone(),
        changes.clone(),
    );
    let own = content(proposal(business_id()), record, changes);
    let header = AcceptanceHeader {
        business: business_id(),
        seq: 1,
        prev: Some(genesis.id),
        proposal: other.proposal.id,
        epoch: 0,
        next_service: None,
    };
    CrossBusiness {
        books,
        state,
        epoch,
        header,
        other,
        own,
    }
}

#[test]
fn open_acceptance_refuses_a_proposal_for_another_business() {
    let c = cross_business();
    // The service can't build it; a dishonest one can sign it anyway.
    assert_eq!(
        build_acceptance(
            &service(),
            &c.epoch.public_key(),
            &c.header,
            &c.other,
            at(10)
        )
        .unwrap_err(),
        ProtocolError::TagMismatch("e")
    );
    let forged = forge(&service(), &c.epoch, &c.header, &c.other);
    assert_eq!(
        open_acceptance(&forged, &c.epoch, &pk(&service())).unwrap_err(),
        ProtocolError::TagMismatch("b")
    );
    // The same command, proposed for this business, opens.
    let header = AcceptanceHeader {
        proposal: c.own.proposal.id,
        ..c.header
    };
    let real =
        build_acceptance(&service(), &c.epoch.public_key(), &header, &c.own, at(10)).unwrap();
    assert!(open_acceptance(&real, &c.epoch, &pk(&service())).is_ok());
}

#[test]
fn follow_refuses_an_acceptance_for_another_business() {
    let c = cross_business();
    assert_eq!(
        c.state.follow(&c.other, 10).unwrap_err(),
        ProtocolError::WrongBusiness
    );
    // The owner may change settings, so the same content for this business
    // follows (and leaves the membership as it was).
    assert_eq!(c.state.follow(&c.own, 10).unwrap(), c.state);
}

#[test]
fn audit_refuses_an_acceptance_for_another_business() {
    let c = cross_business();
    assert_eq!(
        audit(Some(&c.books), &c.other).unwrap_err(),
        Refusal::Protocol(ProtocolError::WrongBusiness)
    );
    // The recorded changes are exactly what the command does to A's books,
    // so only the business binding separates the two.
    let after = audit(Some(&c.books), &c.own).unwrap();
    assert_eq!(after.profile.name, "Renamed Studio");
}
