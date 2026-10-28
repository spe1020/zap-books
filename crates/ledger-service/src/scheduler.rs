//! Activates owner-level changes when their 48 hours are up.
//!
//! The work is [`Ledger::activate_due`], which reads the ledger's clock, so
//! tests drive it with a hand-set clock and no waiting. The service also
//! activates a business's due changes before handling its proposals and
//! reads, so the loop only matters for businesses nobody is using.

use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::ledger::Ledger;

/// How often the loop looks for due changes. A change activates at most
/// this long after its `effectiveAt`.
pub const INTERVAL: Duration = Duration::from_secs(30);

/// Runs [`Ledger::activate_due`] every `every`, until the task is dropped.
pub fn spawn(ledger: Arc<Ledger>, every: Duration) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(every);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            match ledger.activate_due().await {
                Ok(0) => {}
                Ok(n) => tracing::info!(acceptances = n, "activated due changes"),
                Err(e) => tracing::error!(error = %e, "the scheduler failed"),
            }
        }
    })
}
