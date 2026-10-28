//! Time and ids. Commands take both through `ledger-core`'s `Env`, so tests
//! can freeze the clock, and acceptances record what was used.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The service's clock, in milliseconds since the Unix epoch.
pub trait Clock: Send + Sync {
    fn now_millis(&self) -> u64;

    fn now_secs(&self) -> u64 {
        self.now_millis() / 1000
    }
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_millis(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

/// A clock tests set by hand.
pub struct FixedClock(AtomicU64);

impl FixedClock {
    pub fn new(millis: u64) -> FixedClock {
        FixedClock(AtomicU64::new(millis))
    }

    pub fn set(&self, millis: u64) {
        self.0.store(millis, Ordering::SeqCst);
    }

    pub fn advance_secs(&self, secs: u64) {
        self.0.fetch_add(secs * 1000, Ordering::SeqCst);
    }
}

impl Clock for FixedClock {
    fn now_millis(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// `new Date(millis).toISOString()`: `2026-10-15T12:00:00.000Z`.
pub fn iso_millis(millis: u64) -> String {
    let secs = millis / 1000;
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        millis % 1000
    )
}

/// The proleptic Gregorian date of a day count since 1970-01-01 (Howard
/// Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// A random version-4 UUID, the shape of `crypto.randomUUID()`.
pub fn uuid() -> String {
    let mut b: [u8; 16] = rand::random();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex::encode(b);
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_matches_javascript() {
        assert_eq!(iso_millis(0), "1970-01-01T00:00:00.000Z");
        // 2026-10-15T12:00:00.000Z, the parity fixtures' clock.
        assert_eq!(iso_millis(1_792_065_600_000), "2026-10-15T12:00:00.000Z");
        assert_eq!(iso_millis(951_782_400_123), "2000-02-29T00:00:00.123Z");
        assert_eq!(iso_millis(4_107_542_399_999), "2100-02-28T23:59:59.999Z");
    }

    #[test]
    fn uuids_have_the_v4_shape() {
        let u = uuid();
        assert_eq!(u.len(), 36);
        assert_eq!(&u[14..15], "4");
        assert!(matches!(&u[19..20], "8" | "9" | "a" | "b"));
        assert_ne!(uuid(), u);
    }
}
