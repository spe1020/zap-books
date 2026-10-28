//! Event kinds and tag names, defined once.
//!
//! The kinds sit in the regular range (1000–9999: relays store them all).
//! `docs/protocol.md` records the check that they are not registered.

use nostr::prelude::Kind;

/// A member's proposal, NIP-44 encrypted to the service.
pub const PROPOSAL: u16 = 4130;
/// The service's acceptance: one link of the books' hash chain.
pub const ACCEPTANCE: u16 = 4131;
/// The service's rejection, addressed to the proposer only.
pub const REJECTION: u16 = 4132;
/// One epoch private key, NIP-44 encrypted to one member.
pub const EPOCH_KEY_DELIVERY: u16 = 4133;

pub fn kind(number: u16) -> Kind {
    Kind::from(number)
}

/// Tag names. `e` and `p` keep their NIP-01 meanings; the rest are ours.
pub mod tag {
    /// `["business", <business id>]`
    pub const BUSINESS: &str = "business";
    /// `["cmd", <command type>]`, on proposals.
    pub const COMMAND: &str = "cmd";
    /// `["base_seq", <decimal>]`, on proposals other than genesis.
    pub const BASE_SEQ: &str = "base_seq";
    /// `["service", <hex public key>]`, on genesis and service-key proposals.
    pub const SERVICE: &str = "service";
    /// `["seq", <decimal>]`, on acceptances.
    pub const SEQ: &str = "seq";
    /// `["prev", <event id>]`, on every acceptance but seq 0.
    pub const PREV: &str = "prev";
    /// `["epoch", <decimal>]`, on acceptances and deliveries.
    pub const EPOCH: &str = "epoch";
    /// `["e", <proposal event id>]`: the proposal accepted or rejected.
    pub const EVENT: &str = "e";
    /// `["p", <hex public key>]`: the member a rejection or delivery is for.
    pub const PUBKEY: &str = "p";
}
