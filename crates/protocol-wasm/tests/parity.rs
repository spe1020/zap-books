//! WASM parity, the native half: every case in
//! `fixtures/protocol/wasm-parity.json` must give exactly its recorded
//! answer here. `src/wasm/parity.test.ts` runs the same cases through the
//! WebAssembly build, so the two builds agree byte for byte.
//!
//! The fixture is a small business's chain (invented values and test keys)
//! with its acceptances, deliveries and a signed proposal, plus tampered
//! copies. Signatures and NIP-44 nonces are random, so regenerating rewrites
//! the file; the answers stay the same in meaning. Regenerate with
//! `PROTOCOL_WASM_FIXTURES=write cargo test -p protocol-wasm --test parity`.

use std::path::PathBuf;

use ledger_core::{BusinessData, Env};
use nostr::event::FinalizeEvent;
use nostr::prelude::{Event, EventBuilder, Keys, Kind, PublicKey, SecretKey, Tag, Timestamp};
use protocol::{
    apply_command, build_acceptance, build_delivery, build_proposal, build_rejection, decrypt_from,
    diff, encrypt_to, genesis_business, genesis_changes, open_proposal, AcceptanceContent,
    AcceptanceHeader, BusinessId, Command, CommittedPayload, EnvRecord, EpochKey, History, Member,
    MembershipChange, MembershipState, Outcome, ProposalPayload, RecordingEnv, Recovery, Role,
    PROTOCOL_VERSION,
};
use protocol_wasm::api;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/protocol/wasm-parity.json")
}

#[derive(Serialize, Deserialize)]
struct Fixture {
    #[serde(rename = "_note")]
    note: String,
    cases: Vec<Case>,
}

/// One call: the export's name, its request, and the exact answer text or
/// error message.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    name: String,
    function: String,
    request: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    answer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn call(function: &str, request: &str) -> api::Answer {
    match function {
        "buildProposal" => api::build_proposal(request),
        "checkSignedProposal" => api::check_signed_proposal(request),
        "verifyChain" => api::verify_chain(request),
        "openAcceptance" => api::open_acceptance(request),
        "followMembership" => api::follow_membership(request),
        "applyRecordChanges" => api::apply_record_changes(request),
        "parseDelivery" => api::parse_delivery(request),
        "openDelivery" => api::open_delivery(request),
        "parseRejection" => api::parse_rejection(request),
        "openRejection" => api::open_rejection(request),
        "nextBatch" => api::next_batch(request),
        "limits" => api::limits(),
        "isOwnerLevel" => api::is_owner_level(request),
        other => panic!("no export named {other}"),
    }
}

#[test]
fn every_case_gives_its_recorded_answer() {
    if std::env::var("PROTOCOL_WASM_FIXTURES").as_deref() == Ok("write") {
        let fixture = generate();
        let text = serde_json::to_string_pretty(&fixture).unwrap() + "\n";
        std::fs::create_dir_all(fixture_path().parent().unwrap()).unwrap();
        std::fs::write(fixture_path(), text).unwrap();
    }
    let text = std::fs::read_to_string(fixture_path()).unwrap();
    let fixture: Fixture = serde_json::from_str(&text).unwrap();
    assert!(fixture.cases.len() > 30);
    for case in &fixture.cases {
        let got = call(&case.function, &case.request.to_string());
        let want = match (&case.answer, &case.error) {
            (Some(answer), None) => Ok(answer.clone()),
            (None, Some(error)) => Err(error.clone()),
            _ => panic!("{}: needs exactly one of answer and error", case.name),
        };
        assert_eq!(got, want, "{}", case.name);
    }
}

// The scenario

const T0: u64 = 1_900_000_000;
const NOW: &str = "2030-03-17T09:00:00.000Z";

fn key(n: u8) -> Keys {
    Keys::new(SecretKey::from_slice(&[n; 32]).unwrap())
}

fn service() -> Keys {
    key(0x51)
}
fn owner() -> Keys {
    key(0x52)
}
fn backup() -> Keys {
    key(0x53)
}
fn viewer() -> Keys {
    key(0x54)
}

fn business() -> BusinessId {
    BusinessId::from_bytes([0xcd; 32])
}

fn salt(n: u8) -> String {
    hex::encode([n; 32])
}

/// A fictional business: one bank account, one expense account, one
/// category.
fn genesis_command() -> Command {
    let id = business().to_string();
    serde_json::from_value(json!({
        "type": "CreateBusiness",
        "service": service().public_key(),
        "recovery": { "type": "backupKey", "pubkey": backup().public_key() },
        "profile": {
            "id": id, "businessId": id, "name": "Juniper Lane Bindery",
            "industry": "Bookbinding", "currency": "USD", "initials": "JL", "color": "#7a9e7e"
        },
        "accounts": [
            { "id": "jl-bank", "businessId": id, "name": "Everyday account", "kind": "bank", "currency": "USD" },
            { "id": "jl-supplies", "businessId": id, "name": "Supplies", "kind": "expense", "currency": "USD" }
        ],
        "categories": [
            { "id": "jl-category-supplies", "businessId": id, "name": "Studio supplies", "accountId": "jl-supplies" }
        ]
    }))
    .unwrap()
}

fn draft_command() -> Command {
    let id = business().to_string();
    serde_json::from_value(json!({
        "type": "SaveDraft",
        "draft": {
            "id": "jl-draft-1", "businessId": id, "vendor": "Copperleaf Paper Mill",
            "date": "2030-03-11", "description": "Endpaper stock", "amountMinor": 2917,
            "currency": "USD", "categoryId": "jl-category-supplies", "accountId": "jl-bank",
            "purpose": "Endpaper stock for spring orders", "documentIds": [], "status": "draft",
            "duplicateReason": "", "createdAt": NOW
        }
    }))
    .unwrap()
}

/// A stand-in for the service, as in `crates/protocol/tests/flow.rs`, but
/// with real record changes so the books can be rebuilt.
struct Books {
    state: MembershipState,
    books: BusinessData,
    epochs: Vec<EpochKey>,
    chain: Vec<Event>,
    proposals: Vec<(Event, String)>,
    next_id: u32,
}

/// Epoch `n`'s key is fixed, so the fixture's keys don't change when it is
/// regenerated.
fn epoch_key(n: u64) -> EpochKey {
    EpochKey::from_secret(n, SecretKey::from_slice(&[0x60 + n as u8; 32]).unwrap())
}

impl Books {
    fn create() -> Books {
        let payload = ProposalPayload {
            v: PROTOCOL_VERSION,
            business: business(),
            base_seq: None,
            salt: salt(0x70),
            command: genesis_command(),
        };
        let proposal = build_proposal(&owner(), &service().public_key(), &payload, at(T0)).unwrap();
        let (_, committed) = open_proposal(&proposal, &service()).unwrap();
        let state = MembershipState::genesis(
            business(),
            owner().public_key(),
            service().public_key(),
            Recovery::BackupKey {
                pubkey: backup().public_key(),
            },
        )
        .unwrap();
        let books = genesis_business(business(), &committed.command).unwrap();
        let mut b = Books {
            state,
            books: books.clone(),
            epochs: vec![epoch_key(0)],
            chain: vec![],
            proposals: vec![],
            next_id: 0,
        };
        let changes = genesis_changes(&books);
        let env = EnvRecord {
            ids: vec![],
            now: NOW.into(),
        };
        b.append(&proposal, committed, env, changes, Outcome::Applied, T0);
        b
    }

    fn append(
        &mut self,
        proposal: &Event,
        payload: CommittedPayload,
        env: EnvRecord,
        changes: Vec<protocol::RecordChange>,
        outcome: Outcome,
        now: u64,
    ) {
        self.proposals
            .push((proposal.clone(), payload.text().to_string()));
        let epoch = self.epochs.last().unwrap();
        let header = AcceptanceHeader {
            business: business(),
            seq: self.chain.len() as u64,
            prev: self.chain.last().map(|e| e.id),
            proposal: proposal.id,
            epoch: epoch.epoch,
            next_service: None,
        };
        let content = AcceptanceContent {
            v: PROTOCOL_VERSION,
            proposal: proposal.clone(),
            payload,
            env,
            changes,
            outcome,
        };
        let event =
            build_acceptance(&service(), &epoch.public_key(), &header, &content, at(now)).unwrap();
        self.chain.push(event);
    }

    fn propose(&mut self, signer: &Keys, command: Command, n: u8) -> (Event, CommittedPayload) {
        let payload = ProposalPayload {
            v: PROTOCOL_VERSION,
            business: business(),
            base_seq: Some(self.chain.len() as u64 - 1),
            salt: salt(0x70 + n),
            command,
        };
        let now = T0 + self.chain.len() as u64;
        let proposal = build_proposal(signer, &service().public_key(), &payload, at(now)).unwrap();
        let (_, committed) = open_proposal(&proposal, &service()).unwrap();
        (proposal, committed)
    }

    /// Applies a ledger command, as the service does.
    fn command(&mut self, signer: &Keys, command: Command, n: u8) {
        let (proposal, committed) = self.propose(signer, command, n);
        self.state
            .authorize(&signer.public_key(), &committed.command)
            .unwrap();
        let mut counter = self.next_id;
        let mut env = RecordingEnv::new(
            || {
                counter += 1;
                format!("svc-{counter:04}")
            },
            NOW,
        );
        let after = apply_command(&self.books, &committed.command, &mut env).unwrap();
        assert_eq!(env.today(), "2030-03-17");
        let record = env.into_record();
        self.next_id += record.ids.len() as u32;
        let changes = diff(&self.books, &after).unwrap();
        self.state = self
            .state
            .accepted(&signer.public_key(), &committed.command);
        self.books = after;
        let now = T0 + self.chain.len() as u64;
        self.append(&proposal, committed, record, changes, Outcome::Applied, now);
    }

    fn membership(&mut self, change: MembershipChange, n: u8) {
        let (proposal, committed) = self.propose(&owner(), Command::ChangeMembership { change }, n);
        let now = T0 + self.chain.len() as u64;
        let update = self
            .state
            .propose(proposal.id, &owner().public_key(), &change, now)
            .unwrap();
        if update.new_epoch {
            self.epochs.push(epoch_key(update.state.epoch));
        }
        self.state = update.state;
        let env = EnvRecord {
            ids: vec![],
            now: NOW.into(),
        };
        self.append(&proposal, committed, env, vec![], update.outcome, now);
    }
}

fn at(secs: u64) -> Timestamp {
    Timestamp::from_secs(secs)
}

fn value<T: Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap()
}

/// Content altered after signing: the id no longer matches.
fn tampered(event: &Event) -> Value {
    let mut v = value(event);
    let content = v["content"].as_str().unwrap().to_string();
    let flipped = if content.starts_with('A') { "B" } else { "A" };
    v["content"] = Value::String(format!("{flipped}{}", content.get(1..).unwrap()));
    v
}

struct Cases(Vec<Case>);

impl Cases {
    /// Records a case with the answer the native build gives now.
    fn add(&mut self, name: &str, function: &str, request: Value) -> Option<Value> {
        let got = call(function, &request.to_string());
        let parsed = got.as_ref().ok().map(|a| serde_json::from_str(a).unwrap());
        let (answer, error) = match got {
            Ok(a) => (Some(a), None),
            Err(e) => (None, Some(e)),
        };
        self.0.push(Case {
            name: name.into(),
            function: function.into(),
            request,
            answer,
            error,
        });
        parsed
    }

    fn ok(&mut self, name: &str, function: &str, request: Value) -> Value {
        let len = self.0.len();
        self.add(name, function, request)
            .unwrap_or_else(|| panic!("{name}: {:?}", self.0[len].error))
    }

    fn err(&mut self, name: &str, function: &str, request: Value) {
        assert!(
            self.add(name, function, request).is_none(),
            "{name} should fail"
        );
    }
}

fn generate() -> Fixture {
    let mut books = Books::create(); // seq 0, epoch 0
    books.command(&owner(), draft_command(), 1); // 1
    books.membership(
        MembershipChange::Add {
            member: viewer().public_key(),
            role: Role::Viewer,
            history: History::FromJoin,
        },
        2,
    ); // 2, epoch 1
    books.command(
        &owner(),
        Command::Approve {
            expense_id: "jl-draft-1".into(),
        },
        3,
    ); // 3
    books.membership(
        MembershipChange::Remove {
            member: viewer().public_key(),
        },
        4,
    ); // 4, epoch 2
    assert_eq!(books.state.epoch, 2);

    let chain: Vec<Value> = books.chain.iter().map(value).collect();
    let svc = service().public_key();
    let biz = business();
    let other_biz = BusinessId::from_bytes([0xce; 32]);
    let mut c = Cases(vec![]);

    // verifyChain
    let all = c.ok(
        "verifyChain: the whole chain from genesis",
        "verifyChain",
        json!({ "business": biz, "service": svc, "acceptances": chain }),
    );
    assert_eq!(all["seq"], 4);
    let head2 = c.ok(
        "verifyChain: genesis to seq 2",
        "verifyChain",
        json!({ "business": biz, "service": svc, "acceptances": chain[..3] }),
    );
    c.ok(
        "verifyChain: continues from a stored head",
        "verifyChain",
        json!({ "business": biz, "service": svc, "after": head2, "acceptances": chain[3..] }),
    );
    c.ok(
        "verifyChain: nothing new after a stored head",
        "verifyChain",
        json!({ "business": biz, "service": svc, "after": head2, "acceptances": [] }),
    );
    c.ok(
        "verifyChain: an empty chain has no head",
        "verifyChain",
        json!({ "business": biz, "service": svc, "acceptances": [] }),
    );
    c.err(
        "verifyChain: altered content",
        "verifyChain",
        json!({ "business": biz, "service": svc, "acceptances": [chain[0], chain[1], tampered(&books.chain[2])] }),
    );
    c.err(
        "verifyChain: a missing acceptance",
        "verifyChain",
        json!({ "business": biz, "service": svc, "acceptances": [chain[0], chain[1], chain[3]] }),
    );
    c.err(
        "verifyChain: reordered acceptances",
        "verifyChain",
        json!({ "business": biz, "service": svc, "acceptances": [chain[0], chain[2], chain[1]] }),
    );
    c.err(
        "verifyChain: signed by another key",
        "verifyChain",
        json!({ "business": biz, "service": owner().public_key(), "acceptances": chain }),
    );
    c.err(
        "verifyChain: another business",
        "verifyChain",
        json!({ "business": other_biz, "service": svc, "acceptances": chain }),
    );
    // A fork: a different acceptance at seq 2 than the one stored.
    let mut fork = books.chain[..2].to_vec();
    let (proposal, text) = books.proposals[2].clone();
    let forked = build_acceptance(
        &service(),
        &books.epochs[1].public_key(),
        &AcceptanceHeader {
            business: biz,
            seq: 2,
            prev: Some(fork[1].id),
            proposal: proposal.id,
            epoch: 1,
            next_service: None,
        },
        &AcceptanceContent {
            v: PROTOCOL_VERSION,
            proposal: proposal.clone(),
            payload: CommittedPayload::from_text(text).unwrap(),
            env: EnvRecord {
                ids: vec![],
                now: "2030-03-18T09:00:00.000Z".into(),
            },
            changes: vec![],
            outcome: Outcome::Applied,
        },
        at(T0 + 2),
    )
    .unwrap();
    fork.push(forked);
    let fork_head = c.ok(
        "verifyChain: a forked seq 2 verifies on its own",
        "verifyChain",
        json!({ "business": biz, "service": svc, "acceptances": fork.iter().map(value).collect::<Vec<_>>() }),
    );
    assert_ne!(fork_head["id"], head2["id"]);
    c.err(
        "verifyChain: the service's chain doesn't extend a forked stored head",
        "verifyChain",
        json!({ "business": biz, "service": svc, "after": fork_head, "acceptances": chain[3..] }),
    );
    c.err(
        "verifyChain: a malformed request",
        "verifyChain",
        json!({ "business": "not-a-business", "service": svc, "acceptances": [] }),
    );

    // openAcceptance, followMembership, applyRecordChanges, seq by seq
    let mut state: Option<Value> = None;
    let mut snapshot: Option<Value> = None;
    let mut opened = vec![];
    for (seq, event) in chain.iter().enumerate() {
        let epoch = protocol::parse_acceptance(&books.chain[seq]).unwrap().epoch;
        let secret = books.epochs[epoch as usize].secret_key().to_secret_hex();
        let o = c.ok(
            &format!("openAcceptance: seq {seq}"),
            "openAcceptance",
            json!({ "event": event, "epoch": epoch, "epochSecret": secret, "service": svc, "business": biz }),
        );
        let s = c.ok(
            &format!("followMembership: seq {seq}"),
            "followMembership",
            json!({ "state": state, "acceptance": o }),
        );
        let b = c.ok(
            &format!("applyRecordChanges: seq {seq}"),
            "applyRecordChanges",
            json!({ "snapshot": snapshot, "acceptance": o }),
        );
        state = Some(s);
        snapshot = Some(b);
        opened.push(o);
    }
    // The books the browser rebuilds are the ones the service holds.
    let rebuilt: BusinessData = serde_json::from_value(snapshot.clone().unwrap()).unwrap();
    assert_eq!(rebuilt, books.books);
    let followed: MembershipState = serde_json::from_value(state.clone().unwrap()).unwrap();
    assert_eq!(followed, books.state);

    // A document with a stored copy is ordinary record data: the books keep
    // its address and key as the acceptance carries them.
    let mut with_stored = opened.last().unwrap().clone();
    let document = ledger_core::SourceDocument {
        id: "jl-receipt-1".into(),
        business_id: biz.to_string(),
        name: "receipt.pdf".into(),
        mime: "application/pdf".into(),
        size: 4096,
        kind: ledger_core::DocumentKind::Receipt,
        created_at: NOW.into(),
        demo: false,
        sha256: Some("4c".repeat(32)),
        stored: Some(ledger_core::StoredFile {
            hash: "a7".repeat(32),
            size: 4112,
            alg: "AES-256-GCM".into(),
            key: format!("{}w=", "q".repeat(42)),
            nonce: "Zm9yZXhhbXBsZTEy".into(),
        }),
    };
    with_stored["content"]["changes"] = json!([protocol::RecordChange::Put(
        protocol::Record::Document(document.clone())
    )]);
    let stored_books = c.ok(
        "applyRecordChanges: a document with a stored copy",
        "applyRecordChanges",
        json!({ "snapshot": snapshot, "acceptance": with_stored }),
    );
    let stored_books: BusinessData = serde_json::from_value(stored_books).unwrap();
    assert!(stored_books.documents.contains(&document));

    let secret = |n: usize| books.epochs[n].secret_key().to_secret_hex();
    c.err(
        "openAcceptance: an older epoch's key",
        "openAcceptance",
        json!({ "event": chain[4], "epoch": 1, "epochSecret": secret(1), "service": svc, "business": biz }),
    );
    c.err(
        "openAcceptance: the wrong secret for the epoch",
        "openAcceptance",
        json!({ "event": chain[4], "epoch": 2, "epochSecret": secret(1), "service": svc, "business": biz }),
    );
    c.err(
        "openAcceptance: another expected signer",
        "openAcceptance",
        json!({ "event": chain[1], "epoch": 0, "epochSecret": secret(0), "service": owner().public_key(), "business": biz }),
    );
    c.err(
        "openAcceptance: another expected business",
        "openAcceptance",
        json!({ "event": chain[1], "epoch": 0, "epochSecret": secret(0), "service": svc, "business": other_biz }),
    );
    c.err(
        "openAcceptance: altered content",
        "openAcceptance",
        json!({ "event": tampered(&books.chain[1]), "epoch": 0, "epochSecret": secret(0), "service": svc, "business": biz }),
    );

    let states: Vec<Value> = {
        // The state before each seq, again, for the negative cases.
        let mut s = vec![Value::Null];
        for (seq, o) in opened.iter().enumerate() {
            let next = call(
                "followMembership",
                &json!({ "state": s[seq], "acceptance": o }).to_string(),
            )
            .unwrap();
            s.push(serde_json::from_str(&next).unwrap());
        }
        s
    };
    let mut no_rotation = opened[4].clone();
    no_rotation["header"]["epoch"] = json!(1);
    c.err(
        "followMembership: a removal that doesn't start an epoch",
        "followMembership",
        json!({ "state": states[4], "acceptance": no_rotation }),
    );
    c.err(
        "followMembership: a later acceptance with no state",
        "followMembership",
        json!({ "acceptance": opened[1] }),
    );
    c.err(
        "followMembership: genesis on top of a state",
        "followMembership",
        json!({ "state": states[1], "acceptance": opened[0] }),
    );
    c.err(
        "followMembership: removing a member twice",
        "followMembership",
        json!({ "state": states[5], "acceptance": opened[4] }),
    );
    c.err(
        "applyRecordChanges: a later acceptance with no snapshot",
        "applyRecordChanges",
        json!({ "acceptance": opened[1] }),
    );
    // The same books, every record moved to another business.
    let elsewhere: Value = serde_json::from_str(
        &snapshot
            .clone()
            .unwrap()
            .to_string()
            .replace(&biz.to_string(), &other_biz.to_string()),
    )
    .unwrap();
    c.err(
        "applyRecordChanges: another business's books",
        "applyRecordChanges",
        json!({ "snapshot": elsewhere, "acceptance": opened[3] }),
    );

    // Deliveries: what the owner receives, and what the extension decrypts.
    let deliveries: Vec<(Event, String)> = books
        .epochs
        .iter()
        .map(|k| {
            let event =
                build_delivery(&service(), &owner().public_key(), biz, k, at(T0 + 10)).unwrap();
            let text = decrypt_from(owner().secret_key(), &svc, &event.content).unwrap();
            (event, text)
        })
        .collect();
    for (n, (event, text)) in deliveries.iter().enumerate() {
        c.ok(
            &format!("parseDelivery: epoch {n}"),
            "parseDelivery",
            json!({ "event": event, "member": owner().public_key(), "service": svc }),
        );
        let secret = c.ok(
            &format!("openDelivery: epoch {n}"),
            "openDelivery",
            json!({ "event": event, "member": owner().public_key(), "service": svc, "plaintext": text }),
        );
        assert_eq!(
            secret["secret"],
            books.epochs[n].secret_key().to_secret_hex()
        );
    }
    let (event, text) = &deliveries[2];
    c.err(
        "parseDelivery: addressed to someone else",
        "parseDelivery",
        json!({ "event": event, "member": viewer().public_key(), "service": svc }),
    );
    c.err(
        "parseDelivery: another expected signer",
        "parseDelivery",
        json!({ "event": event, "member": owner().public_key(), "service": owner().public_key() }),
    );
    c.err(
        "parseDelivery: altered content",
        "parseDelivery",
        json!({ "event": tampered(event), "member": owner().public_key(), "service": svc }),
    );
    c.err(
        "openDelivery: another epoch's content",
        "openDelivery",
        json!({ "event": event, "member": owner().public_key(), "service": svc, "plaintext": deliveries[1].1 }),
    );
    c.err(
        "openDelivery: content that isn't a delivery",
        "openDelivery",
        json!({ "event": event, "member": owner().public_key(), "service": svc, "plaintext": "{}" }),
    );
    assert!(!text.is_empty());

    // Proposals the extension signed.
    let (proposal, text) = books.proposals[3].clone();
    c.ok(
        "checkSignedProposal: as signed",
        "checkSignedProposal",
        json!({ "event": proposal, "plaintext": text, "author": owner().public_key() }),
    );
    let (genesis, genesis_text) = books.proposals[0].clone();
    c.ok(
        "checkSignedProposal: genesis",
        "checkSignedProposal",
        json!({ "event": genesis, "plaintext": genesis_text, "author": owner().public_key() }),
    );
    c.err(
        "checkSignedProposal: signed by another key",
        "checkSignedProposal",
        json!({ "event": proposal, "plaintext": text, "author": viewer().public_key() }),
    );
    c.err(
        "checkSignedProposal: another payload",
        "checkSignedProposal",
        json!({ "event": proposal, "plaintext": books.proposals[1].1, "author": owner().public_key() }),
    );
    c.err(
        "checkSignedProposal: altered after signing",
        "checkSignedProposal",
        json!({ "event": tampered(&proposal), "plaintext": text, "author": owner().public_key() }),
    );
    c.err(
        "checkSignedProposal: an acceptance instead",
        "checkSignedProposal",
        json!({ "event": chain[3], "plaintext": text, "author": svc }),
    );

    // Rejections: what the proposer receives, and what the extension decrypts.
    let header = protocol::parse_proposal(&proposal).unwrap();
    let rejections: Vec<(Event, String)> = [
        protocol::Reason::rejected(protocol::CHANGED_SINCE_OPENED),
        protocol::Reason::from_error(&protocol::ProtocolError::EventTooLarge {
            size: 70_000,
            max: 65_536,
        }),
    ]
    .iter()
    .map(|reason| {
        let event = build_rejection(&service(), &header, reason, at(T0 + 20)).unwrap();
        let text = decrypt_from(owner().secret_key(), &svc, &event.content).unwrap();
        (event, text)
    })
    .collect();
    for (n, (event, text)) in rejections.iter().enumerate() {
        c.ok(
            &format!("parseRejection: {n}"),
            "parseRejection",
            json!({ "event": event, "member": owner().public_key(), "service": svc }),
        );
        c.ok(
            &format!("openRejection: {n}"),
            "openRejection",
            json!({ "event": event, "member": owner().public_key(), "service": svc, "plaintext": text }),
        );
    }
    let (event, _) = &rejections[0];
    c.err(
        "parseRejection: addressed to someone else",
        "parseRejection",
        json!({ "event": event, "member": viewer().public_key(), "service": svc }),
    );
    c.err(
        "parseRejection: another expected signer",
        "parseRejection",
        json!({ "event": event, "member": owner().public_key(), "service": owner().public_key() }),
    );
    c.err(
        "parseRejection: altered content",
        "parseRejection",
        json!({ "event": tampered(event), "member": owner().public_key(), "service": svc }),
    );
    c.err(
        "openRejection: content that isn't a rejection",
        "openRejection",
        json!({ "event": event, "member": owner().public_key(), "service": svc, "plaintext": "{}" }),
    );

    // Import batching.
    c.ok("limits", "limits", json!({}));
    for (rows, size) in [(25, 70_000), (25, 131_072), (1, 900_000), (3, 65_537)] {
        c.ok(
            &format!("nextBatch: {rows} rows, {size} bytes"),
            "nextBatch",
            json!({ "rows": rows, "size": size, "limit": 65_536 }),
        );
    }

    // Which membership changes wait 48 hours.
    let mut state = MembershipState::genesis(
        business(),
        owner().public_key(),
        svc,
        Recovery::BackupKey {
            pubkey: backup().public_key(),
        },
    )
    .unwrap();
    state.members.insert(
        viewer().public_key(),
        Member {
            role: Role::Viewer,
            history: History::Full,
            joined_epoch: 0,
        },
    );
    let newcomer = key(60).public_key();
    for (name, change) in [
        (
            "adding an owner",
            json!({ "op": "add", "member": newcomer, "role": "owner", "history": "full" }),
        ),
        (
            "adding a bookkeeper",
            json!({ "op": "add", "member": newcomer, "role": "bookkeeper", "history": "full" }),
        ),
        (
            "removing an owner",
            json!({ "op": "remove", "member": owner().public_key() }),
        ),
        (
            "removing a viewer",
            json!({ "op": "remove", "member": viewer().public_key() }),
        ),
        (
            "making a viewer an owner",
            json!({ "op": "changeRole", "member": viewer().public_key(), "role": "owner" }),
        ),
        (
            "making a viewer an assistant",
            json!({ "op": "changeRole", "member": viewer().public_key(), "role": "assistant" }),
        ),
        (
            "recovering an owner",
            json!({ "op": "recover", "lost": owner().public_key(), "replacement": newcomer }),
        ),
    ] {
        c.ok(
            &format!("isOwnerLevel: {name}"),
            "isOwnerLevel",
            json!({ "state": state, "change": change }),
        );
    }
    c.err(
        "isOwnerLevel: an unknown change",
        "isOwnerLevel",
        json!({ "state": state, "change": { "op": "promote", "member": newcomer } }),
    );

    // buildProposal's answers carry a fresh salt; only its refusals are fixed.
    c.err(
        "buildProposal: a malformed command",
        "buildProposal",
        json!({ "business": biz, "baseSeq": 4, "command": { "type": "Approve" } }),
    );
    c.err(
        "buildProposal: an unknown field",
        "buildProposal",
        json!({ "business": biz, "baseSeq": 4, "command": { "type": "Approve", "expenseId": "x" }, "salt": "00" }),
    );

    Fixture {
        note: "Invented values and test keys only. Generated by crates/protocol-wasm/tests/parity.rs; see there to regenerate.".into(),
        cases: c.0,
    }
}

// What a fixed answer can't cover: proposals with a fresh salt, and fresh
// keypairs.

/// Signs `parts` as a NIP-07 extension would: encrypt the plaintext to the
/// service, then sign kind, tags and content.
fn sign_as_extension(member: &Keys, service: &PublicKey, parts: &Value) -> Event {
    let plaintext = parts["plaintext"].as_str().unwrap();
    let content = encrypt_to(member.secret_key(), service, plaintext).unwrap();
    let tags: Vec<Tag> = parts["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            let t: Vec<String> = serde_json::from_value(t.clone()).unwrap();
            Tag::parse(t).unwrap()
        })
        .collect();
    let kind = parts["kind"].as_u64().unwrap() as u16;
    EventBuilder::new(Kind::from(kind), content)
        .tags(tags)
        .custom_created_at(at(T0))
        .finalize(member)
        .unwrap()
}

#[test]
fn a_built_proposal_signed_elsewhere_is_what_build_proposal_signs() {
    let svc = service().public_key();
    for (base_seq, command) in [
        (None, genesis_command()),
        (Some(0), draft_command()),
        (
            Some(3),
            Command::Approve {
                expense_id: "jl-draft-1".into(),
            },
        ),
    ] {
        let request = json!({ "business": business(), "baseSeq": base_seq, "command": command });
        let parts: Value =
            serde_json::from_str(&api::build_proposal(&request.to_string()).unwrap()).unwrap();
        let event = sign_as_extension(&owner(), &svc, &parts);
        let plaintext = parts["plaintext"].as_str().unwrap();
        let check =
            json!({ "event": event, "plaintext": plaintext, "author": owner().public_key() });
        let summary: Value =
            serde_json::from_str(&api::check_signed_proposal(&check.to_string()).unwrap()).unwrap();
        assert_eq!(summary["id"], json!(event.id));
        // The service opens it, and finds exactly the command that was built.
        let (header, committed) = open_proposal(&event, &service()).unwrap();
        assert_eq!(header.base_seq, base_seq);
        assert_eq!(committed.text(), plaintext);
        assert_eq!(committed.command, command);
        assert_eq!(header.payload_hash, committed.hash());
    }
}

#[test]
fn each_built_proposal_has_a_fresh_salt() {
    let request = json!({ "business": business(), "baseSeq": 3, "command": { "type": "Approve", "expenseId": "jl-draft-1" } });
    let a = api::build_proposal(&request.to_string()).unwrap();
    let b = api::build_proposal(&request.to_string()).unwrap();
    assert_ne!(a, b);
}

#[test]
fn a_recovery_keypair_is_fresh_and_consistent() {
    let a: Value = serde_json::from_str(&api::generate_recovery_keypair().unwrap()).unwrap();
    let b: Value = serde_json::from_str(&api::generate_recovery_keypair().unwrap()).unwrap();
    assert_ne!(a["secretKey"], b["secretKey"]);
    let secret = SecretKey::from_hex(a["secretKey"].as_str().unwrap()).unwrap();
    assert_eq!(json!(Keys::new(secret).public_key()), a["publicKey"]);
}
