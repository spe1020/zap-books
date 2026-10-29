//! Hash-chain verification over acceptances, without decrypting them.

use nostr::prelude::{Event, EventId, PublicKey};

use crate::business_id::BusinessId;
use crate::error::{ProtocolError, Result};
use crate::events::parse_acceptance;

/// Where a verified chain ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainHead {
    pub seq: u64,
    pub id: EventId,
    pub epoch: u64,
    /// The key that must sign the next acceptance.
    pub service: PublicKey,
}

/// Verifies `acceptances` as the continuation of a chain: each is validly
/// signed by the current service key, belongs to `business`, has the next
/// `seq`, links by `prev` to the one before, and moves to the next epoch
/// or stays in its own: genesis is epoch 0, and one acceptance can start at
/// most one epoch.
///
/// From the start, pass `after: None` and `service` as declared in the
/// genesis proposal; to continue, pass the last head. Any altered, missing,
/// inserted or reordered acceptance fails: an altered one no longer matches
/// its id or signature, and the others break `seq` or `prev`.
pub fn verify_chain(
    business: BusinessId,
    service: PublicKey,
    after: Option<ChainHead>,
    acceptances: &[Event],
) -> Result<Option<ChainHead>> {
    let mut head = after;
    let mut signer = after.map_or(service, |h| h.service);
    for event in acceptances {
        let header = parse_acceptance(event)?;
        let expected = match head {
            None => 0,
            Some(h) => h.seq.checked_add(1).ok_or(ProtocolError::Overflow("seq"))?,
        };
        if header.seq != expected {
            return Err(ProtocolError::ChainSeq {
                expected,
                found: header.seq,
            });
        }
        if event.pubkey != signer {
            return Err(ProtocolError::ChainSigner(header.seq));
        }
        if header.business != business {
            return Err(ProtocolError::ChainBusiness(header.seq));
        }
        if header.prev != head.map(|h| h.id) {
            return Err(ProtocolError::ChainPrev(header.seq));
        }
        let epochs = match head {
            None => 0..=0,
            Some(h) => h.epoch..=h.epoch.saturating_add(1),
        };
        if !epochs.contains(&header.epoch) {
            return Err(ProtocolError::ChainEpoch(header.seq));
        }
        if let Some(next) = header.next_service {
            signer = next;
        }
        head = Some(ChainHead {
            seq: header.seq,
            id: event.id,
            epoch: header.epoch,
            service: signer,
        });
    }
    Ok(head)
}
