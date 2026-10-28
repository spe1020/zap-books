//! The Zap Books ledger service: the signer of record for each business's
//! books (`docs/briefs/stage-2-accounting-service.md`, `docs/protocol.md`).
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

pub mod clock;
pub mod http;
pub mod key;
pub mod ledger;
pub mod nip98;
pub mod ratelimit;
pub mod scheduler;

pub use ledger::{Acknowledgment, Ledger, LedgerError, Page, Status};
