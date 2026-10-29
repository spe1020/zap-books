//! Membership and permissions, as pure functions.
//!
//! The state is derived from accepted `CreateBusiness`, `ChangeMembership`
//! and `Veto` commands, so anyone holding the business key can rebuild it
//! from the chain ([`MembershipState::follow`]) and check the service's
//! decisions. Times are Unix seconds, passed in by the caller.

use std::collections::BTreeMap;
use std::ops::RangeInclusive;

use nostr::prelude::{EventId, PublicKey};
use serde::{Deserialize, Serialize};

use crate::business_id::BusinessId;
use crate::command::{Command, History, MembershipChange, Recovery, Role};
use crate::error::{ProtocolError, Result};
use crate::events::{AcceptanceContent, Outcome};

/// How long an owner-level change waits: 48 hours.
pub const OWNER_DELAY_SECS: u64 = 48 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub role: Role,
    pub history: History,
    /// The first epoch a from-join-date member may read.
    pub joined_epoch: u64,
}

/// An owner-level change waiting for its `effective_at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingChange {
    pub proposal: EventId,
    pub proposer: PublicKey,
    pub change: MembershipChange,
    pub effective_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MembershipState {
    /// The business this membership belongs to. `follow` refuses an
    /// acceptance for any other.
    pub business: BusinessId,
    pub members: BTreeMap<PublicKey, Member>,
    /// The offline backup key, if the business has one.
    pub recovery_key: Option<PublicKey>,
    /// The key that signs this business's acceptances.
    pub service: PublicKey,
    /// The current epoch.
    pub epoch: u64,
    /// Owner-level changes not yet active, in the order they were accepted.
    pub pending: Vec<PendingChange>,
}

/// Who signed a proposal, as far as this business is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    Member(Role),
    RecoveryKey,
    /// Not a member, or a removed one.
    Outsider,
}

/// The result of an accepted membership change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MembershipUpdate {
    pub state: MembershipState,
    /// How it was accepted: [`Outcome::Applied`], or [`Outcome::Pending`]
    /// for an owner-level change.
    pub outcome: Outcome,
    /// Whether the change started a new epoch (`state.epoch` is the new one).
    pub new_epoch: bool,
}

impl MembershipState {
    /// The state genesis creates: the founding owner, the recovery party,
    /// epoch 0.
    pub fn genesis(
        business: BusinessId,
        founder: PublicKey,
        service: PublicKey,
        recovery: Recovery,
    ) -> Result<Self> {
        let owner = Member {
            role: Role::Owner,
            history: History::Full,
            joined_epoch: 0,
        };
        let mut state = MembershipState {
            business,
            members: BTreeMap::from([(founder, owner)]),
            recovery_key: None,
            service,
            epoch: 0,
            pending: Vec::new(),
        };
        match recovery {
            Recovery::SecondOwner { pubkey } => {
                if pubkey == founder {
                    return Err(ProtocolError::AlreadyMember);
                }
                state.members.insert(pubkey, owner);
            }
            Recovery::BackupKey { pubkey } => state.recovery_key = Some(pubkey),
        }
        state.check()?;
        Ok(state)
    }

    pub fn actor(&self, key: &PublicKey) -> Actor {
        match self.members.get(key) {
            Some(m) => Actor::Member(m.role),
            None if self.recovery_key == Some(*key) => Actor::RecoveryKey,
            None => Actor::Outsider,
        }
    }

    fn owners(&self) -> usize {
        self.members
            .values()
            .filter(|m| m.role == Role::Owner)
            .count()
    }

    /// The invariants every accepted state keeps.
    fn check(&self) -> Result<()> {
        let owners = self.owners();
        if owners == 0 {
            return Err(ProtocolError::NoOwner);
        }
        if self.recovery_key.is_none() && owners < 2 {
            return Err(ProtocolError::NoRecovery);
        }
        if let Some(key) = self.recovery_key {
            if self.members.contains_key(&key) || key == self.service {
                return Err(ProtocolError::AlreadyMember);
            }
        }
        if self.members.contains_key(&self.service) {
            return Err(ProtocolError::AlreadyMember);
        }
        Ok(())
    }

    /// The permission table in `docs/protocol.md`. A removed member is an
    /// outsider and is refused everything.
    pub fn authorize(&self, signer: &PublicKey, command: &Command) -> Result<()> {
        use Role::{Bookkeeper, Owner, Submitter};
        let actor = self.actor(signer);
        if actor == Actor::Outsider {
            return Err(ProtocolError::NotMember);
        }
        let allowed = match command {
            Command::CreateBusiness { .. } => return Err(ProtocolError::AlreadyCreated),
            Command::SaveDraft { .. }
            | Command::Approve { .. }
            | Command::Correct { .. }
            | Command::CommitImport { .. }
            | Command::UpdateSettings { .. }
            | Command::SaveRule { .. } => {
                matches!(actor, Actor::Member(Owner | Bookkeeper))
            }
            Command::ChangeMembership {
                change: MembershipChange::Recover { .. },
            } => actor == Actor::RecoveryKey,
            Command::ChangeMembership { .. } => actor == Actor::Member(Owner),
            Command::Veto { .. } => matches!(actor, Actor::Member(Owner) | Actor::RecoveryKey),
            Command::SubmitReceiptDraft { .. } => {
                matches!(actor, Actor::Member(Owner | Bookkeeper | Submitter))
            }
        };
        if !allowed {
            return Err(ProtocolError::Forbidden);
        }
        Ok(())
    }

    /// Owner-level changes wait 48 hours and can be vetoed: adding or
    /// removing an owner (including by a role change), changing the
    /// recovery key, replacing the service key, and recovery.
    pub fn is_owner_level(&self, change: &MembershipChange) -> bool {
        let is_owner = |k: &PublicKey| self.members.get(k).is_some_and(|m| m.role == Role::Owner);
        match change {
            MembershipChange::Add { role, .. } => *role == Role::Owner,
            MembershipChange::Remove { member } => is_owner(member),
            MembershipChange::ChangeRole { member, role } => {
                *role == Role::Owner || is_owner(member)
            }
            MembershipChange::ReplaceRecoveryKey { .. }
            | MembershipChange::ReplaceServiceKey { .. }
            | MembershipChange::Recover { .. } => true,
        }
    }

    /// Applies a change now, returning the new state and whether it started
    /// a new epoch. Pending changes from anyone who loses owner or recovery
    /// standing are cancelled with it.
    fn apply(&self, change: &MembershipChange) -> Result<(MembershipState, bool)> {
        let mut next = self.clone();
        let mut rotate = false;
        let taken = |k: &PublicKey| {
            self.members.contains_key(k) || self.recovery_key == Some(*k) || *k == self.service
        };
        match *change {
            MembershipChange::Add {
                member,
                role,
                history,
            } => {
                if taken(&member) {
                    return Err(ProtocolError::AlreadyMember);
                }
                // A from-join-date reader must not hold the current epoch's
                // key, which covers data from before they joined.
                rotate = role.reads() && history == History::FromJoin;
                next.members.insert(
                    member,
                    Member {
                        role,
                        history,
                        joined_epoch: self.epoch + u64::from(rotate),
                    },
                );
            }
            MembershipChange::Remove { member } => {
                if next.members.remove(&member).is_none() {
                    return Err(ProtocolError::UnknownMember);
                }
                rotate = true;
            }
            MembershipChange::ChangeRole { member, role } => {
                let m = next
                    .members
                    .get_mut(&member)
                    .ok_or(ProtocolError::UnknownMember)?;
                let (was, now) = (m.role.reads(), role.reads());
                m.role = role;
                // Losing read access is a removal for the keys' purposes.
                // Gaining it with from-join-date history is a new join.
                if was && !now {
                    rotate = true;
                } else if !was && now && m.history == History::FromJoin {
                    rotate = true;
                    m.joined_epoch = self.epoch + 1;
                }
            }
            MembershipChange::ReplaceRecoveryKey { key } => {
                if let Some(k) = key {
                    if self.members.contains_key(&k) || k == self.service {
                        return Err(ProtocolError::AlreadyMember);
                    }
                }
                next.recovery_key = key;
            }
            MembershipChange::ReplaceServiceKey { key } => {
                if taken(&key) {
                    return Err(ProtocolError::AlreadyMember);
                }
                next.service = key;
                rotate = true;
            }
            MembershipChange::Recover { lost, replacement } => {
                if self.members.get(&lost).map(|m| m.role) != Some(Role::Owner) {
                    return Err(ProtocolError::UnknownMember);
                }
                if taken(&replacement) {
                    return Err(ProtocolError::AlreadyMember);
                }
                next.members.remove(&lost);
                next.members.insert(
                    replacement,
                    Member {
                        role: Role::Owner,
                        history: History::Full,
                        joined_epoch: 0,
                    },
                );
                rotate = true;
            }
        }
        if rotate {
            next.epoch += 1;
        }
        let standing = |k: &PublicKey| {
            next.members.get(k).is_some_and(|m| m.role == Role::Owner)
                || next.recovery_key == Some(*k)
        };
        next.pending.retain(|p| standing(&p.proposer));
        next.check()?;
        Ok((next, rotate))
    }

    /// Accepts a `ChangeMembership` from `signer` at time `now`. Owner-level
    /// changes are checked against the current state and become pending;
    /// they are checked again when they fall due.
    pub fn propose(
        &self,
        proposal: EventId,
        signer: &PublicKey,
        change: &MembershipChange,
        now: u64,
    ) -> Result<MembershipUpdate> {
        self.authorize(signer, &Command::ChangeMembership { change: *change })?;
        if !self.is_owner_level(change) {
            let (state, new_epoch) = self.apply(change)?;
            return Ok(MembershipUpdate {
                state,
                outcome: Outcome::Applied,
                new_epoch,
            });
        }
        self.apply(change)?;
        let effective_at = now.saturating_add(OWNER_DELAY_SECS);
        Ok(MembershipUpdate {
            state: self.with_pending(proposal, *signer, *change, effective_at),
            outcome: Outcome::Pending { effective_at },
            new_epoch: false,
        })
    }

    fn with_pending(
        &self,
        proposal: EventId,
        proposer: PublicKey,
        change: MembershipChange,
        effective_at: u64,
    ) -> MembershipState {
        let mut next = self.clone();
        next.pending.push(PendingChange {
            proposal,
            proposer,
            change,
            effective_at,
        });
        next
    }

    fn pending_index(&self, proposal: &EventId) -> Result<usize> {
        self.pending
            .iter()
            .position(|p| p.proposal == *proposal)
            .ok_or(ProtocolError::NoSuchPending)
    }

    /// Cancels a pending change. Any owner or the recovery key may veto,
    /// except whoever proposed it, and except the owner key a `Recover`
    /// names as lost: recovery outranks the owner being recovered, since a
    /// thief holding that key would otherwise veto every recovery.
    pub fn veto(&self, signer: &PublicKey, pending: &EventId) -> Result<MembershipState> {
        self.authorize(signer, &Command::Veto { pending: *pending })?;
        let index = self.pending_index(pending)?;
        let Some(p) = self.pending.get(index) else {
            return Err(ProtocolError::NoSuchPending);
        };
        if p.proposer == *signer {
            return Err(ProtocolError::OwnVeto);
        }
        if matches!(p.change, MembershipChange::Recover { lost, .. } if lost == *signer) {
            return Err(ProtocolError::RecoveredVeto);
        }
        let mut next = self.clone();
        next.pending.remove(index);
        Ok(next)
    }

    /// The pending changes due at `now`, in acceptance order.
    pub fn due(&self, now: u64) -> Vec<EventId> {
        self.pending
            .iter()
            .filter(|p| p.effective_at <= now)
            .map(|p| p.proposal)
            .collect()
    }

    /// Activates a due pending change. If it no longer applies, the error
    /// says why; the service then records it as [`Outcome::Lapsed`] with
    /// [`MembershipState::lapse`].
    pub fn activate(&self, pending: &EventId, now: u64) -> Result<(MembershipState, bool)> {
        let index = self.pending_index(pending)?;
        let p = self
            .pending
            .get(index)
            .copied()
            .ok_or(ProtocolError::NoSuchPending)?;
        if p.effective_at > now {
            return Err(ProtocolError::NotDue);
        }
        let mut without = self.clone();
        without.pending.remove(index);
        without.apply(&p.change)
    }

    /// Drops a due pending change that could not activate.
    pub fn lapse(&self, pending: &EventId, now: u64) -> Result<MembershipState> {
        let index = self.pending_index(pending)?;
        if self
            .pending
            .get(index)
            .is_some_and(|p| p.effective_at > now)
        {
            return Err(ProtocolError::NotDue);
        }
        let mut next = self.clone();
        next.pending.remove(index);
        Ok(next)
    }

    /// The epochs whose keys `key` is entitled to, or `None` if it may read
    /// nothing (submitters, the recovery key, outsiders).
    pub fn readable_epochs(&self, key: &PublicKey) -> Option<RangeInclusive<u64>> {
        let m = self.members.get(key)?;
        if !m.role.reads() {
            return None;
        }
        let first = match m.history {
            History::Full => 0,
            History::FromJoin => m.joined_epoch,
        };
        Some(first..=self.epoch)
    }

    /// Everyone who receives the current epoch's key.
    pub fn readers(&self) -> impl Iterator<Item = &PublicKey> {
        self.members
            .iter()
            .filter(|(_, m)| m.role.reads())
            .map(|(k, _)| k)
    }

    /// Re-derives the state from the genesis acceptance's content.
    pub fn from_genesis(content: &AcceptanceContent) -> Result<MembershipState> {
        match &content.payload.command {
            Command::CreateBusiness {
                service, recovery, ..
            } => MembershipState::genesis(
                content.payload.business,
                content.proposal.pubkey,
                *service,
                *recovery,
            ),
            _ => Err(ProtocolError::NotGenesis),
        }
    }

    /// Re-derives the state after one later acceptance, re-checking the
    /// service's decision. `at` is the acceptance's `created_at`. An
    /// acceptance for another business is refused, whatever it says.
    pub fn follow(&self, content: &AcceptanceContent, at: u64) -> Result<MembershipState> {
        if content.payload.business != self.business {
            return Err(ProtocolError::WrongBusiness);
        }
        let proposer = content.proposal.pubkey;
        let id = content.proposal.id;
        match (&content.payload.command, content.outcome) {
            (Command::CreateBusiness { .. }, _) => Err(ProtocolError::AlreadyCreated),
            (Command::ChangeMembership { change }, Outcome::Applied) => {
                self.authorize(&proposer, &content.payload.command)?;
                if self.is_owner_level(change) {
                    return Err(ProtocolError::Forbidden);
                }
                Ok(self.apply(change)?.0)
            }
            (Command::ChangeMembership { change }, Outcome::Pending { effective_at }) => {
                self.authorize(&proposer, &content.payload.command)?;
                if !self.is_owner_level(change)
                    || effective_at < at.saturating_add(OWNER_DELAY_SECS)
                {
                    return Err(ProtocolError::Forbidden);
                }
                self.apply(change)?;
                Ok(self.with_pending(id, proposer, *change, effective_at))
            }
            (Command::ChangeMembership { .. }, Outcome::Activated) => Ok(self.activate(&id, at)?.0),
            (Command::ChangeMembership { .. }, Outcome::Lapsed) => {
                if self.activate(&id, at).is_ok() {
                    return Err(ProtocolError::BadChange(
                        "a change that could apply lapsed".into(),
                    ));
                }
                self.lapse(&id, at)
            }
            (Command::Veto { pending }, Outcome::Applied) => self.veto(&proposer, pending),
            (command, Outcome::Applied) => {
                self.authorize(&proposer, command)?;
                Ok(self.clone())
            }
            _ => Err(ProtocolError::BadChange(
                "unexpected outcome for this command".into(),
            )),
        }
    }
}
