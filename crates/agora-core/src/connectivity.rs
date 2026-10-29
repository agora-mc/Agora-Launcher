//! What the outbound requests Agora has actually made have reported.
//!
//! The Privacy page and the offline banner used to ask the browser
//! (`navigator.onLine`). That answers "is there a network interface?", which a
//! proxy, a firewall, or a dead DNS server can all leave as "yes" while every
//! request fails. This module records the outcome of real requests instead:
//! a request that got a response means the internet is reachable; a request
//! that failed to connect means it is not, right now.
//!
//! Only transport outcomes count. A lockdown refusal, a blocked host, or an
//! invalid URL never left the machine, so it says nothing about the network.
//! An HTTP error status (404, 503) is a response, so it counts as reachable.

use serde::Serialize;
use std::sync::Mutex;

/// What the most recent request reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectivityState {
    /// No request has been made since the process started.
    Unknown,
    /// The most recent request received a response.
    Reachable,
    /// The most recent request could not connect to its server.
    Unreachable,
}

/// The state, and when the request that set it finished (unix seconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ConnectivitySnapshot {
    pub state: ConnectivityState,
    pub last_outcome_at: Option<i64>,
}

/// One tracker. The process uses [`global`]; tests make their own.
#[derive(Debug)]
pub struct Connectivity {
    last: Mutex<Option<(bool, i64)>>,
}

impl Connectivity {
    pub const fn new() -> Self {
        Self {
            last: Mutex::new(None),
        }
    }

    /// Record that a request reached its server.
    pub fn record_reachable_at(&self, at: i64) {
        self.set(true, at);
    }

    /// Record that a request could not reach its server.
    pub fn record_unreachable_at(&self, at: i64) {
        self.set(false, at);
    }

    pub fn snapshot(&self) -> ConnectivitySnapshot {
        let last = *self.lock();
        match last {
            None => ConnectivitySnapshot {
                state: ConnectivityState::Unknown,
                last_outcome_at: None,
            },
            Some((reachable, at)) => ConnectivitySnapshot {
                state: if reachable {
                    ConnectivityState::Reachable
                } else {
                    ConnectivityState::Unreachable
                },
                last_outcome_at: Some(at),
            },
        }
    }

    fn set(&self, reachable: bool, at: i64) {
        *self.lock() = Some((reachable, at));
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<(bool, i64)>> {
        // A poisoned lock still holds the last value written. Reading it is
        // safe, and a status display should not go blank because another
        // thread panicked.
        self.last
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Default for Connectivity {
    fn default() -> Self {
        Self::new()
    }
}

static GLOBAL: Connectivity = Connectivity::new();

/// The tracker every checked request reports to.
pub fn global() -> &'static Connectivity {
    &GLOBAL
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Called by the request helpers when a request got a response.
pub fn record_reachable() {
    GLOBAL.record_reachable_at(now());
}

/// Called by the request helpers when a request failed to connect.
pub fn record_unreachable() {
    GLOBAL.record_unreachable_at(now());
}

/// The process-wide state, for the desktop status command and the UI.
pub fn snapshot() -> ConnectivitySnapshot {
    GLOBAL.snapshot()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_recorded_is_unknown_not_online() {
        let tracker = Connectivity::new();
        assert_eq!(
            tracker.snapshot(),
            ConnectivitySnapshot {
                state: ConnectivityState::Unknown,
                last_outcome_at: None,
            }
        );
    }

    #[test]
    fn a_failed_request_after_a_success_reads_unreachable() {
        let tracker = Connectivity::new();
        tracker.record_reachable_at(100);
        tracker.record_unreachable_at(200);
        let snap = tracker.snapshot();
        assert_eq!(snap.state, ConnectivityState::Unreachable);
        assert_eq!(snap.last_outcome_at, Some(200));
    }

    #[test]
    fn a_later_success_clears_an_earlier_failure() {
        let tracker = Connectivity::new();
        tracker.record_unreachable_at(100);
        tracker.record_reachable_at(300);
        let snap = tracker.snapshot();
        assert_eq!(snap.state, ConnectivityState::Reachable);
        assert_eq!(snap.last_outcome_at, Some(300));
    }

    #[test]
    fn an_out_of_order_timestamp_is_recorded_as_given() {
        // Requests finish in whatever order they like. The state follows the
        // most recent *recorded* outcome, not the largest timestamp.
        let tracker = Connectivity::new();
        tracker.record_unreachable_at(500);
        tracker.record_reachable_at(400);
        assert_eq!(tracker.snapshot().state, ConnectivityState::Reachable);
    }

    #[test]
    fn state_serializes_as_snake_case_for_the_ui() {
        let json = serde_json::to_value(ConnectivitySnapshot {
            state: ConnectivityState::Unreachable,
            last_outcome_at: Some(1),
        })
        .unwrap();
        assert_eq!(json["state"], "unreachable");
        assert_eq!(json["last_outcome_at"], 1);
    }
}
