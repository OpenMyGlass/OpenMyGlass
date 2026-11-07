//! How long a phone waits before it tries the Glass again. Pure, so that the apps on both phones
//! wait alike: every attempt on BLE costs a scan, which Android throttles, and wakes the Glass.

/// The wait after the first failure, in milliseconds.
///
/// Also what keeps an Unpair from pairing again by itself: after `unpair`, the core closes the
/// link and the Android app stops its supervisor at once, which must land before this wait ends
/// and the supervisor reconnects, with pairing allowed since the bond is gone. Keep it at least a
/// second; the assert below fails the build otherwise.
pub const MIN_WAIT_MS: u32 = 1_000;

const _: () = assert!(
    MIN_WAIT_MS >= 1_000,
    "an Unpair relies on a reconnect waiting at least a second"
);
/// The longest wait, in milliseconds.
pub const MAX_WAIT_MS: u32 = 30_000;

/// Doubles the wait after each attempt that ended soon.
///
/// Reaching the Glass does not reset it: a Glass that refuses this phone or a link that drops at
/// once would otherwise be retried every second, for ever. Only a link that outlasted the longest
/// wait does.
#[derive(Debug, Clone)]
pub struct Backoff {
    next_ms: u32,
}

impl Backoff {
    pub fn new() -> Backoff {
        Backoff {
            next_ms: MIN_WAIT_MS,
        }
    }

    /// An attempt ended, after a link of `lasted_ms`, or 0 if none opened. How long to wait now.
    pub fn next_wait_ms(&mut self, lasted_ms: u64) -> u32 {
        if lasted_ms >= u64::from(MAX_WAIT_MS) {
            self.reset();
        }
        let wait = self.next_ms;
        self.next_ms = wait.saturating_mul(2).min(MAX_WAIT_MS);
        wait
    }

    /// The owner asked for a link: the next failure starts from the shortest wait again.
    pub fn reset(&mut self) {
        self.next_ms = MIN_WAIT_MS;
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Backoff::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_double_up_to_the_limit() {
        let mut b = Backoff::new();
        let waits: Vec<u32> = (0..8).map(|_| b.next_wait_ms(0)).collect();
        assert_eq!(
            waits,
            [1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000, 30_000]
        );
    }

    #[test]
    fn a_link_that_ends_soon_does_not_reset_it() {
        let mut b = Backoff::new();
        assert_eq!(b.next_wait_ms(5_000), 1_000);
        assert_eq!(b.next_wait_ms(5_000), 2_000);
        assert_eq!(b.next_wait_ms(u64::from(MAX_WAIT_MS) - 1), 4_000);
    }

    #[test]
    fn a_link_that_lasted_resets_it() {
        let mut b = Backoff::new();
        for _ in 0..4 {
            b.next_wait_ms(0);
        }
        assert_eq!(b.next_wait_ms(u64::from(MAX_WAIT_MS)), MIN_WAIT_MS);
        assert_eq!(b.next_wait_ms(0), 2_000);
    }

    #[test]
    fn the_owner_asking_resets_it() {
        let mut b = Backoff::new();
        for _ in 0..4 {
            b.next_wait_ms(0);
        }
        b.reset();
        assert_eq!(b.next_wait_ms(0), MIN_WAIT_MS);
    }

    /// Android blocks a sixth scan start within 30 s. This counts only the waits, and the time each
    /// attempt takes adds to the margin.
    #[test]
    fn immediate_failures_stay_under_the_scan_throttle() {
        let mut b = Backoff::new();
        let mut starts = vec![0u64];
        for _ in 0..20 {
            let at = starts.last().unwrap() + u64::from(b.next_wait_ms(0));
            starts.push(at);
        }
        for pair in starts.windows(6) {
            assert!(pair[5] - pair[0] >= 30_000, "six starts in {pair:?}");
        }
    }
}
