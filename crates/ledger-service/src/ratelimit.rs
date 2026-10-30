//! Token buckets in memory: per public key, per client address, or one for
//! everything. Enough for one service instance; a second instance would
//! need a shared store.
//!
//! The map is bounded: when it holds [`RateLimit::capacity`] keys, the one
//! used longest ago is forgotten (and so starts again with a full bucket).
//! Every call costs O(log n), never a scan of every key.

use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;
use std::sync::Mutex;

/// The default number of keys remembered.
pub const DEFAULT_CAPACITY: usize = 100_000;

struct Bucket {
    tokens: f64,
    at: u64,
    /// When this key was last used, in [`Buckets::tick`] order.
    used: u64,
}

struct Buckets<K> {
    by_key: HashMap<K, Bucket>,
    /// Keys by last use, oldest first.
    by_use: BTreeMap<u64, K>,
    tick: u64,
}

pub struct RateLimit<K> {
    burst: f64,
    per_sec: f64,
    capacity: usize,
    buckets: Mutex<Buckets<K>>,
}

impl<K: Hash + Eq + Clone> RateLimit<K> {
    pub fn new(burst: u32, per_sec: u32) -> RateLimit<K> {
        RateLimit::with_capacity(burst, per_sec, DEFAULT_CAPACITY)
    }

    pub fn with_capacity(burst: u32, per_sec: u32, capacity: usize) -> RateLimit<K> {
        RateLimit {
            burst: f64::from(burst),
            per_sec: f64::from(per_sec),
            capacity: capacity.max(1),
            buckets: Mutex::new(Buckets {
                by_key: HashMap::new(),
                by_use: BTreeMap::new(),
                tick: 0,
            }),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// How many keys are remembered now.
    pub fn len(&self) -> usize {
        self.buckets.lock().map_or(0, |b| b.by_key.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Takes one token for `key` at `now_millis`; false when none is left.
    pub fn allow(&self, key: &K, now_millis: u64) -> bool {
        let Ok(mut guard) = self.buckets.lock() else {
            return false;
        };
        let b = &mut *guard;
        b.tick = b.tick.wrapping_add(1);
        let tick = b.tick;
        if let Some(bucket) = b.by_key.get_mut(key) {
            b.by_use.remove(&bucket.used);
            bucket.used = tick;
        } else {
            if b.by_key.len() >= self.capacity {
                if let Some((_, oldest)) = b.by_use.pop_first() {
                    b.by_key.remove(&oldest);
                }
            }
            b.by_key.insert(
                key.clone(),
                Bucket {
                    tokens: self.burst,
                    at: now_millis,
                    used: tick,
                },
            );
        }
        b.by_use.insert(tick, key.clone());
        let Some(bucket) = b.by_key.get_mut(key) else {
            return false;
        };
        let elapsed = now_millis.saturating_sub(bucket.at) as f64 / 1000.0;
        bucket.tokens = (bucket.tokens + elapsed * self.per_sec).min(self.burst);
        bucket.at = now_millis;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bucket_empties_and_refills() {
        let limit = RateLimit::new(2, 1);
        assert!(limit.allow(&"a", 0));
        assert!(limit.allow(&"a", 0));
        assert!(!limit.allow(&"a", 0));
        assert!(limit.allow(&"b", 0), "keys have their own buckets");
        assert!(limit.allow(&"a", 1_000));
        assert!(!limit.allow(&"a", 1_000));
    }

    #[test]
    fn the_map_never_holds_more_than_its_capacity() {
        // HTTP-2: fresh keys can't grow the map, or make each call scan it.
        let limit = RateLimit::with_capacity(1, 1, 3);
        for n in 0..10_000u32 {
            assert!(limit.allow(&n, 0));
            assert!(limit.len() <= 3);
        }
        assert_eq!(limit.len(), 3);
    }

    #[test]
    fn the_key_used_longest_ago_is_forgotten_first() {
        // HTTP-2: least recently used, not least recently created.
        let limit = RateLimit::with_capacity(1, 0, 2);
        assert!(limit.allow(&"a", 0));
        assert!(limit.allow(&"b", 0));
        assert!(!limit.allow(&"a", 0), "a is used again, so b is now oldest");
        assert!(limit.allow(&"c", 0));
        // b was forgotten and starts full; a was kept, still empty.
        assert!(!limit.allow(&"a", 0));
        assert!(limit.allow(&"b", 0));
    }

    #[test]
    fn many_keys_stay_fast() {
        // HTTP-2: with the map full, a call is a lookup, not a scan.
        let limit = RateLimit::with_capacity(1, 1, DEFAULT_CAPACITY);
        for n in 0..DEFAULT_CAPACITY as u64 {
            limit.allow(&n, 0);
        }
        let started = std::time::Instant::now();
        for n in 0..20_000u64 {
            limit.allow(&(DEFAULT_CAPACITY as u64 + n), 0);
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
    }
}
