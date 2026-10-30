//! The assistant role (decision 0004): one test for each row of the
//! decision's permission table, what an assistant can do and what it can't,
//! plus the rules that hold them together.

mod common;

use common::*;
use ledger_core::{BusinessData, ExpenseDraft, ExpenseStatus, SequentialEnv};
use nostr::prelude::{EventId, Keys};
use protocol::{
    apply_command, diff, Actor, Command, DocumentRef, History, MembershipChange, MembershipState,
    Outcome, ProtocolError, RecordChange, RecordType, Refusal, Role, RuleInput, SettingsChange,
    SuggestionInput,
};

const T0: u64 = 1_800_000_000;

fn id(n: u8) -> EventId {
    EventId::from_slice(&[n; 32]).unwrap()
}

/// A second assistant, to show one can't edit another's drafts.
fn other_assistant() -> Keys {
    key(10)
}

fn add(keys: &Keys, role: Role, history: History) -> MembershipChange {
    MembershipChange::Add {
        member: pk(keys),
        role,
        history,
    }
}

/// Owner and backup key, a bookkeeper, and two assistants with full history.
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
        (assistant(), Role::Assistant),
        (other_assistant(), Role::Assistant),
    ]
    .iter()
    .enumerate()
    {
        s = s
            .propose(
                id(n as u8 + 10),
                &pk(&owner()),
                &add(keys, *role, History::Full),
                T0,
            )
            .unwrap()
            .state;
    }
    s
}

/// The membership and the books together, as the service keeps them.
struct Books {
    state: MembershipState,
    books: BusinessData,
    env: SequentialEnv,
}

impl Books {
    fn new() -> Books {
        let (books, env) = seed_business();
        Books {
            state: team(),
            books,
            env,
        }
    }

    /// Checks and applies a non-membership command as the service does:
    /// permission, then the books, then the membership it implies.
    fn propose(&mut self, signer: &Keys, command: &Command) -> Result<(), Refusal> {
        self.state.authorize(&pk(signer), command)?;
        self.books = apply_command(&self.books, command, &mut self.env)?;
        self.state = self.state.accepted(&pk(signer), command);
        Ok(())
    }

    /// A draft a person created, still open.
    fn people_draft(&self) -> ExpenseDraft {
        self.books
            .drafts
            .iter()
            .find(|d| d.status() == ExpenseStatus::Draft)
            .unwrap()
            .clone()
    }
}

/// A new draft with `id`, copied from a person's draft.
fn new_draft(books: &Books, id: &str) -> ExpenseDraft {
    let mut d = books.people_draft();
    d.id = id.into();
    d.payment_id = None;
    d.import_source = None;
    d.document_ids = vec![];
    d
}

fn submit(draft: ExpenseDraft, documents: Vec<DocumentRef>) -> Command {
    Command::SubmitReceiptDraft { draft, documents }
}

fn save(draft: ExpenseDraft) -> Command {
    Command::SaveDraft {
        draft,
        documents: vec![],
        extraction: None,
    }
}

fn suggestion(author: &Keys, draft_id: &str) -> Command {
    Command::Suggest {
        suggestion: SuggestionInput {
            draft_id: draft_id.into(),
            author: pk(author).to_hex(),
            category_id: None,
            purpose: Some("Invented purpose".into()),
            note: None,
            evidence: vec!["Invented line".into()],
        },
    }
}

fn forbidden(e: ProtocolError) -> Result<(), Refusal> {
    Err(Refusal::Protocol(e))
}

// Can: read the books (receives epoch keys, per its history choice).

#[test]
fn an_assistant_reads_the_books_per_its_history_choice() {
    let s = team();
    assert_eq!(s.actor(&pk(&assistant())), Actor::Member(Role::Assistant));
    assert!(Role::Assistant.reads());
    assert!(s.readers().any(|k| *k == pk(&assistant())));
    assert_eq!(s.readable_epochs(&pk(&assistant())), Some(0..=s.epoch));

    // From its join date: the epoch rotates, and it reads only the new one.
    let late = key(11);
    let u = s
        .propose(
            id(1),
            &pk(&owner()),
            &add(&late, Role::Assistant, History::FromJoin),
            T0,
        )
        .unwrap();
    assert_eq!(u.outcome, Outcome::Applied);
    assert!(u.new_epoch);
    assert_eq!(u.state.readable_epochs(&pk(&late)), Some(1..=1));
}

// Can: create drafts, and edit drafts it created.

#[test]
fn an_assistant_creates_drafts_and_edits_the_ones_it_created() {
    let mut b = Books::new();
    let mut draft = new_draft(&b, "assistant-draft");
    b.propose(&assistant(), &submit(draft.clone(), vec![]))
        .unwrap();
    assert_eq!(
        b.state.assistant_drafts.get("assistant-draft"),
        Some(&pk(&assistant()))
    );

    draft.purpose = "Edited by the assistant".into();
    b.propose(&assistant(), &save(draft)).unwrap();
    let stored = b
        .books
        .drafts
        .iter()
        .find(|d| d.id == "assistant-draft")
        .unwrap();
    assert_eq!(stored.purpose, "Edited by the assistant");

    // Saving it again keeps it the assistant's.
    let mut again = stored.clone();
    again.purpose = "Edited again".into();
    b.propose(&assistant(), &save(again)).unwrap();
    // A person's new draft is not recorded as anyone's.
    b.propose(
        &bookkeeper(),
        &submit(new_draft(&b, "person-draft"), vec![]),
    )
    .unwrap();
    assert_eq!(b.state.assistant_drafts.len(), 1);
}

#[test]
fn a_person_who_saves_an_assistants_draft_takes_it_over() {
    let mut b = Books::new();
    let mut draft = new_draft(&b, "assistant-draft");
    b.propose(&assistant(), &submit(draft.clone(), vec![]))
        .unwrap();
    // Untouched drafts of the same assistant stay its own.
    b.propose(&assistant(), &submit(new_draft(&b, "untouched"), vec![]))
        .unwrap();

    draft.purpose = "Edited by a person".into();
    b.propose(&bookkeeper(), &save(draft.clone())).unwrap();
    assert!(!b.state.assistant_drafts.contains_key("assistant-draft"));
    assert!(b.state.assistant_drafts.contains_key("untouched"));

    // From then on the assistant can only suggest on it.
    assert_eq!(
        b.propose(&assistant(), &save(draft.clone())),
        forbidden(ProtocolError::AssistantOwnDraftsOnly)
    );
    b.propose(&assistant(), &suggestion(&assistant(), &draft.id))
        .unwrap();

    // An owner's save takes it over just the same.
    let mut untouched = new_draft(&b, "untouched");
    untouched.purpose = "Edited by the owner".into();
    b.propose(&owner(), &save(untouched)).unwrap();
    assert!(b.state.assistant_drafts.is_empty());
}

#[test]
fn removing_an_assistant_or_changing_its_role_drops_its_drafts() {
    let mut b = Books::new();
    for (keys, id) in [
        (assistant(), "first"),
        (assistant(), "second"),
        (other_assistant(), "theirs"),
    ] {
        b.propose(&keys, &submit(new_draft(&b, id), vec![]))
            .unwrap();
    }
    let change = |b: &mut Books, n: u8, change: MembershipChange| {
        b.state = b
            .state
            .propose(id(n), &pk(&owner()), &change, T0)
            .unwrap()
            .state;
    };
    let mine = |b: &Books, keys: &Keys| {
        b.state
            .assistant_drafts
            .values()
            .filter(|k| **k == pk(keys))
            .count()
    };

    // Removed: every entry goes, and the other assistant keeps its own.
    change(
        &mut b,
        1,
        MembershipChange::Remove {
            member: pk(&assistant()),
        },
    );
    assert_eq!(mine(&b, &assistant()), 0);
    assert_eq!(mine(&b, &other_assistant()), 1);
    // Added back, it doesn't get them back.
    change(&mut b, 2, add(&assistant(), Role::Assistant, History::Full));
    assert_eq!(mine(&b, &assistant()), 0);
    assert_eq!(
        b.propose(&assistant(), &save(new_draft(&b, "first"))),
        forbidden(ProtocolError::AssistantOwnDraftsOnly)
    );

    // A role change to anything but assistant drops them too, and changing
    // back doesn't restore them.
    for (n, role) in [Role::Viewer, Role::Assistant].into_iter().enumerate() {
        change(
            &mut b,
            n as u8 + 3,
            MembershipChange::ChangeRole {
                member: pk(&other_assistant()),
                role,
            },
        );
        assert_eq!(mine(&b, &other_assistant()), 0);
    }
    assert!(b.state.assistant_drafts.is_empty());
}

#[test]
fn every_role_change_away_from_assistant_drops_its_drafts() {
    for role in [Role::Bookkeeper, Role::Viewer, Role::Submitter, Role::Owner] {
        let mut b = Books::new();
        b.propose(&assistant(), &submit(new_draft(&b, "mine"), vec![]))
            .unwrap();
        let change = MembershipChange::ChangeRole {
            member: pk(&assistant()),
            role,
        };
        let u = b.state.propose(id(1), &pk(&owner()), &change, T0).unwrap();
        let state = match u.outcome {
            // Becoming an owner waits 48 hours: it keeps its drafts until
            // then, since it is still an assistant.
            Outcome::Pending { effective_at } => {
                assert_eq!(u.state.assistant_drafts.len(), 1);
                u.state.activate(&id(1), effective_at).unwrap().0
            }
            _ => u.state,
        };
        assert!(state.assistant_drafts.is_empty(), "{role:?}");
    }
}

// Can't: edit drafts created by people.

#[test]
fn an_assistant_cannot_edit_drafts_people_or_other_assistants_created() {
    let mut b = Books::new();
    let people = b.people_draft();
    assert_eq!(
        b.propose(&assistant(), &save(people)),
        forbidden(ProtocolError::AssistantOwnDraftsOnly)
    );

    let theirs = new_draft(&b, "other-assistant-draft");
    b.propose(&other_assistant(), &submit(theirs.clone(), vec![]))
        .unwrap();
    assert_eq!(
        b.propose(&assistant(), &save(theirs)),
        forbidden(ProtocolError::AssistantOwnDraftsOnly)
    );

    // A new draft goes through SubmitReceiptDraft, not SaveDraft.
    assert_eq!(
        b.propose(&assistant(), &save(new_draft(&b, "never-submitted"))),
        forbidden(ProtocolError::AssistantOwnDraftsOnly)
    );
    // And SubmitReceiptDraft can't overwrite an existing draft.
    let people = b.people_draft();
    assert_eq!(
        b.propose(&assistant(), &submit(people, vec![])),
        forbidden(ProtocolError::SubmitterNewDraftsOnly)
    );
}

// Can: attach suggestions to any draft.

#[test]
fn an_assistant_attaches_suggestions_to_any_draft_without_changing_it() {
    let mut b = Books::new();
    let people = b.people_draft();
    let before = b.books.clone();
    b.propose(&assistant(), &suggestion(&assistant(), &people.id))
        .unwrap();
    let s = &b.books.suggestions[0];
    assert_eq!(s.draft_id, people.id);
    assert_eq!(s.author, pk(&assistant()).to_hex());
    assert_eq!(s.evidence, vec!["Invented line".to_string()]);
    // Only the suggestion is new: the draft and everything else are as they
    // were.
    let changes = diff(&before, &b.books).unwrap();
    assert_eq!(changes.len(), 1);
    assert!(
        matches!(&changes[0], RecordChange::Put(r) if r.key().record_type == RecordType::Suggestion)
    );

    // Its own drafts, and another assistant's, take suggestions too.
    b.propose(&other_assistant(), &submit(new_draft(&b, "theirs"), vec![]))
        .unwrap();
    b.propose(&assistant(), &suggestion(&assistant(), "theirs"))
        .unwrap();
    assert_eq!(b.books.suggestions.len(), 2);
}

#[test]
fn a_suggestion_names_its_signer_and_comes_only_from_an_assistant() {
    let s = team();
    let draft = Books::new().people_draft();
    assert_eq!(
        s.authorize(
            &pk(&assistant()),
            &suggestion(&other_assistant(), &draft.id)
        ),
        Err(ProtocolError::SuggestionAuthor)
    );
    for person in [owner(), bookkeeper()] {
        assert_eq!(
            s.authorize(&pk(&person), &suggestion(&person, &draft.id)),
            Err(ProtocolError::Forbidden)
        );
    }
}

// Can: submit receipts.

#[test]
fn an_assistant_submits_receipts() {
    let mut b = Books::new();
    let mut draft = new_draft(&b, "receipt-draft");
    draft.document_ids = vec!["receipt-1".into()];
    let document = DocumentRef {
        id: "receipt-1".into(),
        sha256: "7d".repeat(32),
        name: "receipt.pdf".into(),
        mime: "application/pdf".into(),
        size: 2048,
    };
    b.propose(&assistant(), &submit(draft, vec![document]))
        .unwrap();
    assert!(b.books.documents.iter().any(|d| d.id == "receipt-1"));
    assert!(b.state.assistant_drafts.contains_key("receipt-draft"));
}

// Can't: approve, correct, or import.

#[test]
fn an_assistant_cannot_approve_correct_or_import() {
    let mut b = Books::new();
    let mine = new_draft(&b, "mine");
    b.propose(&assistant(), &submit(mine.clone(), vec![]))
        .unwrap();
    for command in [
        // Not even a draft it created.
        Command::Approve {
            expense_id: mine.id.clone(),
        },
        Command::Correct {
            original_id: mine.id.clone(),
            replacement: new_draft(&b, "replacement"),
            reason: "Invented reason".into(),
        },
        Command::CommitImport {
            account_id: b.books.accounts[0].id.clone(),
            rows: vec![],
        },
    ] {
        assert_eq!(
            b.state.authorize(&pk(&assistant()), &command),
            Err(ProtocolError::Forbidden),
            "{:?}",
            command.command_type()
        );
    }
}

// Can't: change settings, rules or membership.

#[test]
fn an_assistant_cannot_change_settings_rules_or_membership() {
    let s = team();
    let member = pk(&bookkeeper());
    let changes = [
        add(&key(12), Role::Viewer, History::Full),
        add(&key(12), Role::Assistant, History::Full),
        add(&key(12), Role::Owner, History::Full),
        MembershipChange::Remove { member },
        // Not even to remove itself, or promote itself.
        MembershipChange::Remove {
            member: pk(&assistant()),
        },
        MembershipChange::ChangeRole {
            member: pk(&assistant()),
            role: Role::Bookkeeper,
        },
        MembershipChange::ReplaceRecoveryKey {
            key: Some(pk(&key(12))),
        },
        MembershipChange::ReplaceServiceKey { key: pk(&key(12)) },
    ];
    let commands = changes
        .into_iter()
        .map(|change| Command::ChangeMembership { change })
        .chain([
            Command::UpdateSettings {
                change: SettingsChange::Profile {
                    name: "Invented".into(),
                    industry: "Invented".into(),
                },
            },
            Command::UpdateSettings {
                change: SettingsChange::AddCategory {
                    name: "Invented".into(),
                    currency: ledger_core::Currency::Usd,
                },
            },
            Command::SaveRule {
                rule: RuleInput {
                    id: None,
                    vendor_includes: "Invented".into(),
                    category_id: "c".into(),
                    purpose: "Invented".into(),
                    enabled: true,
                },
            },
        ]);
    for command in commands {
        assert_eq!(
            s.authorize(&pk(&assistant()), &command),
            Err(ProtocolError::Forbidden),
            "{command:?}"
        );
    }
}

// Can't: veto, recover, or see pending owner changes.

#[test]
fn an_assistant_cannot_veto_recover_or_see_pending_owner_changes() {
    let s = team()
        .propose(
            id(1),
            &pk(&owner()),
            &add(&key(12), Role::Owner, History::Full),
            T0,
        )
        .unwrap()
        .state;
    assert_eq!(s.pending.len(), 1);
    assert_eq!(
        s.authorize(&pk(&assistant()), &Command::Veto { pending: id(1) }),
        Err(ProtocolError::Forbidden)
    );
    assert_eq!(
        s.veto(&pk(&assistant()), &id(1)),
        Err(ProtocolError::Forbidden)
    );
    let recover = Command::ChangeMembership {
        change: MembershipChange::Recover {
            lost: pk(&owner()),
            replacement: pk(&assistant()),
        },
    };
    assert_eq!(
        s.authorize(&pk(&assistant()), &recover),
        Err(ProtocolError::Forbidden)
    );

    assert!(!s.sees_pending(&pk(&assistant())));
    assert!(!s.sees_pending(&pk(&bookkeeper())));
    assert!(s.sees_pending(&pk(&owner())));
    assert!(s.sees_pending(&pk(&backup())));
    assert!(!s.sees_pending(&pk(&outsider())));
}

// Removing the agent works like removing anyone.

#[test]
fn an_assistant_is_added_and_removed_at_once_and_removal_rotates() {
    let s = team();
    let u = s
        .propose(
            id(1),
            &pk(&owner()),
            &MembershipChange::Remove {
                member: pk(&assistant()),
            },
            T0,
        )
        .unwrap();
    assert_eq!(u.outcome, Outcome::Applied);
    assert!(u.new_epoch);
    assert_eq!(u.state.actor(&pk(&assistant())), Actor::Outsider);
    assert_eq!(u.state.readable_epochs(&pk(&assistant())), None);
    assert_eq!(
        u.state
            .authorize(&pk(&assistant()), &suggestion(&assistant(), "anything")),
        Err(ProtocolError::NotMember)
    );
}

#[test]
fn membership_saved_before_the_assistant_role_still_loads() {
    let s = team();
    let mut json = serde_json::to_value(&s).unwrap();
    assert!(json.get("assistantDrafts").is_none());
    json.as_object_mut().unwrap().remove("assistantDrafts");
    let loaded: MembershipState = serde_json::from_value(json).unwrap();
    assert_eq!(loaded, s);

    let mut b = Books::new();
    b.propose(&assistant(), &submit(new_draft(&b, "mine"), vec![]))
        .unwrap();
    let json = serde_json::to_value(&b.state).unwrap();
    assert_eq!(
        json["assistantDrafts"]["mine"],
        serde_json::json!(pk(&assistant()).to_hex())
    );
    let loaded: MembershipState = serde_json::from_value(json).unwrap();
    assert_eq!(loaded, b.state);
}
