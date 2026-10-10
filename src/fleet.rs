//! Daemon fleet — the 24/7 pattern ported from demand #5.
//!
//! The operator built this once himself (Lucidota runner with heartbeats, queues;
//! "FUCK YES. PERSISTENT DAEMON ARCHITECTURE" — 2026-04-28). This module ports the
//! pattern into the core as deterministic types: [`Role`]s (Foreman / Worker /
//! Auditor), [`Heartbeat`]s, a [`TaskQueue`], and a [`Supervisor`] that flags
//! daemons whose heartbeats went stale. No threads, no timers inside the library —
//! `tick` takes the current time explicitly, so tests and supervisors stay honest.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Fleet roles, straight from the operator's own architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Foreman,
    Worker,
    Auditor,
}

/// What a daemon reports about itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DaemonState {
    Starting,
    Running,
    Idle,
    Stalled,
    Stopped,
}

/// One heartbeat from one daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Heartbeat {
    pub daemon_id: String,
    pub role: Role,
    pub state: DaemonState,
    pub seq: u64,
    pub ts_utc: DateTime<Utc>,
}

impl Heartbeat {
    pub fn new(daemon_id: &str, role: Role, state: DaemonState, seq: u64, ts: DateTime<Utc>) -> Self {
        Self { daemon_id: daemon_id.into(), role, state, seq, ts_utc: ts }
    }

    pub fn now(daemon_id: &str, role: Role, state: DaemonState, seq: u64) -> Self {
        Self::new(daemon_id, role, state, seq, Utc::now())
    }
}

/// One unit of work waiting in (or claimed from) the queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskItem {
    pub id: String,
    pub payload: String,
    pub claimed_by: Option<String>,
}

/// The supervisor: tracks heartbeats and the task queue.
#[derive(Debug, Default)]
pub struct Supervisor {
    heartbeats: HashMap<String, Heartbeat>,
    queue: Vec<TaskItem>,
    /// Seconds after which a missing heartbeat means the daemon is stale.
    pub stale_after_secs: u64,
}

impl Supervisor {
    pub fn new(stale_after_secs: u64) -> Self {
        Self { heartbeats: HashMap::new(), queue: Vec::new(), stale_after_secs }
    }

    /// Register a heartbeat. A lower-or-equal `seq` from the same daemon is
    /// rejected as a replay — heartbeats only move forward.
    pub fn heartbeat(&mut self, hb: Heartbeat) -> Result<(), String> {
        if let Some(prev) = self.heartbeats.get(&hb.daemon_id) {
            if hb.seq <= prev.seq {
                return Err(format!("stale seq {} <= {} from {}", hb.seq, prev.seq, hb.daemon_id));
            }
        }
        self.heartbeats.insert(hb.daemon_id.clone(), hb);
        Ok(())
    }

    /// Daemon ids whose last heartbeat is older than `stale_after_secs`, or that
    /// reported `Stalled` / `Stopped`.
    pub fn tick(&self, now: DateTime<Utc>) -> Vec<String> {
        self.heartbeats
            .values()
            .filter(|hb| {
                matches!(hb.state, DaemonState::Stalled | DaemonState::Stopped)
                    || (now - hb.ts_utc).num_seconds() > self.stale_after_secs as i64
            })
            .map(|hb| hb.daemon_id.clone())
            .collect()
    }

    pub fn enqueue(&mut self, id: &str, payload: &str) {
        self.queue.push(TaskItem { id: id.into(), payload: payload.into(), claimed_by: None });
    }

    /// A daemon claims the oldest unclaimed task.
    pub fn claim(&mut self, daemon_id: &str) -> Option<TaskItem> {
        let item = self.queue.iter_mut().find(|t| t.claimed_by.is_none())?;
        item.claimed_by = Some(daemon_id.into());
        Some(item.clone())
    }

    pub fn queue_depth(&self) -> usize {
        self.queue.iter().filter(|t| t.claimed_by.is_none()).count()
    }

    pub fn daemon_count(&self) -> usize {
        self.heartbeats.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    #[test]
    fn fresh_heartbeats_are_not_stale() {
        let mut s = Supervisor::new(60);
        s.heartbeat(Heartbeat::new("w1", Role::Worker, DaemonState::Running, 1, t(1000))).unwrap();
        assert!(s.tick(t(1030)).is_empty());
    }

    #[test]
    fn silent_daemon_goes_stale() {
        let mut s = Supervisor::new(60);
        s.heartbeat(Heartbeat::new("w1", Role::Worker, DaemonState::Running, 1, t(1000))).unwrap();
        assert_eq!(s.tick(t(1061)), vec!["w1".to_string()]);
    }

    #[test]
    fn stalled_state_flags_immediately() {
        let mut s = Supervisor::new(3600);
        s.heartbeat(Heartbeat::new("a1", Role::Auditor, DaemonState::Stalled, 1, t(1000))).unwrap();
        assert_eq!(s.tick(t(1001)), vec!["a1".to_string()]);
    }

    #[test]
    fn replayed_seq_rejected() {
        let mut s = Supervisor::new(60);
        s.heartbeat(Heartbeat::new("w1", Role::Worker, DaemonState::Running, 5, t(1000))).unwrap();
        assert!(s.heartbeat(Heartbeat::new("w1", Role::Worker, DaemonState::Running, 5, t(1001))).is_err());
        assert!(s.heartbeat(Heartbeat::new("w1", Role::Worker, DaemonState::Running, 4, t(1001))).is_err());
        assert!(s.heartbeat(Heartbeat::new("w1", Role::Worker, DaemonState::Running, 6, t(1002))).is_ok());
    }

    #[test]
    fn queue_claims_oldest_first() {
        let mut s = Supervisor::new(60);
        s.enqueue("t1", "first");
        s.enqueue("t2", "second");
        assert_eq!(s.claim("w1").unwrap().id, "t1");
        assert_eq!(s.claim("w2").unwrap().id, "t2");
        assert!(s.claim("w3").is_none());
        assert_eq!(s.queue_depth(), 0);
    }

    #[test]
    fn fleet_roles_cover_foreman_worker_auditor() {
        let mut s = Supervisor::new(60);
        for (id, role) in [("f1", Role::Foreman), ("w1", Role::Worker), ("a1", Role::Auditor)] {
            s.heartbeat(Heartbeat::new(id, role, DaemonState::Running, 1, t(1000))).unwrap();
        }
        assert_eq!(s.daemon_count(), 3);
        assert!(s.tick(t(1010)).is_empty());
    }
}
