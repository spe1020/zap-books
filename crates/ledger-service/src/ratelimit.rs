//! A token bucket per public key, in memory. Enough for one service
//! instance; a second instance would need a shared store.

use std::collections::HashMap;
use std::sync::Mutex;

use nostr::prelude::PublicKey;

pub struct RateLimit {
    burst: f64,
    per_sec: f64,
    buckets: Mutex<HashMap<PublicKey, (f64, u64)>>,
}

impl RateLimit {
    pub fn new(burst: u32, per_sec: u32) -> RateLimit {
        RateLimit {
            burst: f64::from(burst),
            per_sec: f64::from(per_sec),
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Takes one token for `key` at `now_millis`; false when none is left.
    pub fn allow(&self, key: &PublicKey, now_millis: u64) -> bool {
        let Ok(mut buckets) = self.buckets.lock() else {
            return false;
        };
        // Forget idle keys so the map can't grow without bound.
        if buckets.len() > 100_000 {
            buckets.retain(|_, (_, at)| now_millis.saturating_sub(*at) < 60_000);
        }
        let (tokens, at) = buckets.entry(*key).or_insert((self.burst, now_millis));
        let elapsed = now_millis.saturating_sub(*at) as f64 / 1000.0;
        *tokens = (*tokens + elapsed * self.per_sec).min(self.burst);
        *at = now_millis;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    }
}
