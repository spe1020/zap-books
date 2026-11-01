//! Activates owner-level changes when their 48 hours are up, and deletes
//! uploaded receipts no accepted record referred to within 24 hours.
//!
//! The work is [`Ledger::activate_due`] and [`Ledger::clean_blobs`], which
//! read the ledger's clock, so tests drive them with a hand-set clock and no
//! waiting. The service also activates a business's due changes before
//! handling its proposals and reads, so the loop only matters for
//! activations in businesses nobody is using.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::ledger::{Ledger, LedgerError};

/// How often the loop looks for due changes. A change activates at most
/// this long after its `effectiveAt`.
pub const INTERVAL: Duration = Duration::from_secs(30);

/// Runs [`Ledger::activate_due`], then [`Ledger::clean_blobs`], every
/// `every`, until the task is aborted.
pub fn spawn(ledger: Arc<Ledger>, every: Duration) -> JoinHandle<()> {
    supervise(every, move || {
        let ledger = ledger.clone();
        async move {
            let activated = ledger.activate_due().await;
            // A failed cleanup is logged; activations still count.
            if let Err(e) = ledger.clean_blobs().await {
                tracing::error!(error = %e, "deleting unreferenced blobs failed");
            }
            activated
        }
    })
}

/// Runs `work` every `every`, each run as its own task, so a run that
/// panics is logged and the next tick runs anyway: one bad row can't stop
/// activations for every business. The returned task ends only when
/// aborted; the caller keeps its handle and logs if it ever does end.
pub fn supervise<F, Fut>(every: Duration, work: F) -> JoinHandle<()>
where
    F: Fn() -> Fut + Send + 'static,
    Fut: Future<Output = Result<usize, LedgerError>> + Send + 'static,
{
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(every);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            match tokio::spawn(work()).await {
                Ok(Ok(0)) => {}
                Ok(Ok(n)) => tracing::info!(acceptances = n, "activated due changes"),
                Ok(Err(e)) => tracing::error!(error = %e, "the scheduler failed"),
                Err(e) if e.is_panic() => {
                    tracing::error!("the scheduler's run panicked; it runs again next tick")
                }
                Err(e) => tracing::error!(error = %e, "the scheduler's run was cancelled"),
            }
        }
    })
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn a_run_that_panics_doesnt_stop_the_loop() {
        // SVC-5: before, a panic ended the task for good.
        let runs = Arc::new(AtomicUsize::new(0));
        let counted = runs.clone();
        let handle = supervise(Duration::from_millis(5), move || {
            let n = counted.fetch_add(1, Ordering::SeqCst);
            async move {
                if n == 0 {
                    panic!("an invented failure");
                }
                Ok(0)
            }
        });
        for _ in 0..200 {
            if runs.load(Ordering::SeqCst) >= 3 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(runs.load(Ordering::SeqCst) >= 3, "the loop stopped");
        assert!(!handle.is_finished());
        handle.abort();
    }
}
