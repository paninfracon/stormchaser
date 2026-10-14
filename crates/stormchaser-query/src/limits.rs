//! How much hydration the query service does at once.
//!
//! A hydration can read a repository and then run every query in the workflow: SQL,
//! HTTP and cloud API calls. Without a bound, one user could start as many as they
//! liked and load this service, its outbound connections and the systems the queries
//! reach. Each user may have a few hydrations in flight, and the service a fixed number
//! in all; a request beyond either is refused rather than queued.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Most hydrations in flight across the service. Default 64.
pub const MAX_HYDRATIONS_ENV: &str = "STORMCHASER_QUERY_MAX_HYDRATIONS";
/// Most hydrations one user may have in flight. Default 4.
pub const MAX_HYDRATIONS_PER_USER_ENV: &str = "STORMCHASER_QUERY_MAX_HYDRATIONS_PER_USER";
/// Most queries one hydration runs at the same time.
pub const MAX_CONCURRENT_QUERIES: usize = 8;

const DEFAULT_MAX_HYDRATIONS: usize = 64;
const DEFAULT_MAX_HYDRATIONS_PER_USER: usize = 4;

/// Counts the hydrations in flight.
#[derive(Clone)]
pub struct HydrationSlots {
    inner: Arc<Inner>,
}

struct Inner {
    max_total: usize,
    max_per_user: usize,
    in_flight: Mutex<InFlight>,
}

#[derive(Default)]
struct InFlight {
    total: usize,
    by_user: HashMap<String, usize>,
}

/// A hydration's place; it is given back when dropped.
pub struct Slot {
    inner: Arc<Inner>,
    user: String,
}

/// The service's slots, with limits read from the environment on first use.
pub fn hydration_slots() -> &'static HydrationSlots {
    static SLOTS: std::sync::OnceLock<HydrationSlots> = std::sync::OnceLock::new();
    SLOTS.get_or_init(HydrationSlots::from_env)
}

impl HydrationSlots {
    /// Slots with the given limits.
    pub fn new(max_total: usize, max_per_user: usize) -> Self {
        Self {
            inner: Arc::new(Inner {
                max_total,
                max_per_user,
                in_flight: Mutex::new(InFlight::default()),
            }),
        }
    }

    /// Slots with the limits from the environment, or the defaults.
    pub fn from_env() -> Self {
        let read = |name: &str, default: usize| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.trim().parse().ok())
                .filter(|n: &usize| *n > 0)
                .unwrap_or(default)
        };
        Self::new(
            read(MAX_HYDRATIONS_ENV, DEFAULT_MAX_HYDRATIONS),
            read(MAX_HYDRATIONS_PER_USER_ENV, DEFAULT_MAX_HYDRATIONS_PER_USER),
        )
    }

    /// A slot for `user`, unless they or the service are at their limit.
    pub fn try_acquire(&self, user: &str) -> Option<Slot> {
        let mut in_flight = self.inner.in_flight.lock().unwrap();
        let mine = in_flight.by_user.get(user).copied().unwrap_or(0);
        if in_flight.total >= self.inner.max_total || mine >= self.inner.max_per_user {
            return None;
        }
        in_flight.total += 1;
        in_flight.by_user.insert(user.to_string(), mine + 1);
        Some(Slot {
            inner: self.inner.clone(),
            user: user.to_string(),
        })
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut in_flight = self.inner.in_flight.lock().unwrap();
        in_flight.total -= 1;
        if let Some(count) = in_flight.by_user.get_mut(&self.user) {
            *count -= 1;
            if *count == 0 {
                in_flight.by_user.remove(&self.user);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_and_the_service_are_each_limited() {
        let slots = HydrationSlots::new(3, 2);
        let a1 = slots.try_acquire("alice").unwrap();
        let _a2 = slots.try_acquire("alice").unwrap();
        assert!(
            slots.try_acquire("alice").is_none(),
            "alice is at her limit"
        );

        let _b1 = slots.try_acquire("bob").unwrap();
        assert!(slots.try_acquire("carol").is_none(), "the service is full");

        drop(a1);
        assert!(slots.try_acquire("carol").is_some());
    }

    #[test]
    fn slots_are_given_back_when_dropped() {
        let slots = HydrationSlots::new(1, 1);
        drop(slots.try_acquire("alice").unwrap());
        drop(slots.try_acquire("bob").unwrap());
        assert!(slots.inner.in_flight.lock().unwrap().by_user.is_empty());
    }
}
