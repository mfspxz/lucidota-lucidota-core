//! Daemon fleet — the 24/7 pattern ported from demand #5.
//!
//! The operator built this once himself (Lucidota runner with heartbeats, queues;
//! "FUCK YES. PERSISTENT DAEMON ARCHITECTURE" — 2026-04-28). This module ports the
//! pattern into the core as deterministic types: [`Role`]s (Foreman / Worker /
//! Auditor), [`Heartbeat`]s, a [`TaskQueue`], and a [`Supervisor`] that flags
//! daemons whose heartbeats went stale. No threads, no timers inside the library —
//! `tick` takes the current time explicitly, so tests and supervisors stay honest.
//!
//! Liveness vs readiness (WIRE-3, Letta Code v0.34.24 lesson, 2026-10-25):
//! a heartbeat proves a daemon is *alive*, nothing more. A listener can hold a
//! healthy heartbeat forever while startup is blocked, while its execution
//! queue is permanently jammed, or while it never admits a single task. So the
//! supervisor tracks three independent facts per daemon and never lets one
//! substitute for another:
//!   - liveness: is the heartbeat fresh? (`stale_after_secs`)
//!   - readiness: what the daemon reports it can do ([`Readiness`]: Connected,
//!     Ready, Executing, Stalled, Recovering) — and, crucially, what it has
//!     *proven* by claiming/completing work
//!   - startup: did a Starting/Connected/Recovering daemon become Ready within
//!     `startup_deadline_secs` (default 60s, separate from the heartbeat bound)?
//! Re-registration fences stale owners: `register()`/`reregister()` issue an
//! epoch token, heartbeats carrying an older token are rejected, and the old
//! incarnation's claimed tasks are released back to the queue.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

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

/// Readiness: what an organelle can actually do right now. Distinct from
/// liveness (heartbeat freshness) and from [`DaemonState`].
///
/// - `Connected`: registered and heartbeating, but NOT admitted to work.
///   This is the conservative default for a bare heartbeat.
/// - `Ready`: admitted and able to claim tasks.
/// - `Executing`: actively working a claimed task.
/// - `Stalled`: heartbeating but unable to make execution progress.
/// - `Recovering`: restarting / re-registering, not yet readmitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Readiness {
    Connected,
    Ready,
    Executing,
    Stalled,
    Recovering,
}

/// One heartbeat from one daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Heartbeat {
    pub daemon_id: String,
    pub role: Role,
    pub state: DaemonState,
    pub readiness: Readiness,
    /// Fencing epoch issued by [`Supervisor::register`]. `None` is the legacy
    /// path: accepted only while no epoch has ever been issued for this daemon.
    pub owner_token: Option<u64>,
    pub seq: u64,
    pub ts_utc: DateTime<Utc>,
}

impl Heartbeat {
    pub fn new(daemon_id: &str, role: Role, state: DaemonState, seq: u64, ts: DateTime<Utc>) -> Self {
        Self {
            daemon_id: daemon_id.into(),
            role,
            state,
            // Conservative default: a heartbeat proves liveness, never readiness.
            readiness: Readiness::Connected,
            owner_token: None,
            seq,
            ts_utc: ts,
        }
    }

    pub fn now(daemon_id: &str, role: Role, state: DaemonState, seq: u64) -> Self {
        Self::new(daemon_id, role, state, seq, Utc::now())
    }

    pub fn with_readiness(mut self, readiness: Readiness) -> Self {
        self.readiness = readiness;
        self
    }

    pub fn with_owner(mut self, owner_token: u64) -> Self {
        self.owner_token = Some(owner_token);
        self
    }
}

/// One unit of work waiting in (or claimed from) the queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskItem {
    pub id: String,
    pub payload: String,
    pub claimed_by: Option<String>,
    pub claimed_at: Option<DateTime<Utc>>,
}

/// The supervisor's verdict on one daemon at one instant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonHealth {
    pub daemon_id: String,
    /// Heartbeat fresh within `stale_after_secs`.
    pub alive: bool,
    pub readiness: Readiness,
    /// True when `tick` would flag this daemon.
    pub flagged: bool,
    pub reason: Option<&'static str>,
}

/// The supervisor: tracks heartbeats and the task queue.
#[derive(Debug, Default)]
pub struct Supervisor {
    heartbeats: HashMap<String, Heartbeat>,
    queue: Vec<TaskItem>,
    /// Seconds after which a missing heartbeat means the daemon is stale.
    pub stale_after_secs: u64,
    /// Seconds a Starting/Connected/Recovering daemon has to become Ready,
    /// even while heartbeating fresh. Separate from `stale_after_secs`.
    pub startup_deadline_secs: u64,
    /// Seconds a claimed task may sit uncompleted before its owner counts as
    /// execution-stalled, even with fresh heartbeats.
    pub execution_stall_after_secs: u64,
    /// Seconds a Ready daemon that has never claimed may leave work sitting
    /// unclaimed before it counts as admission-stalled.
    pub admission_grace_secs: u64,
    first_seen: HashMap<String, DateTime<Utc>>,
    ready_since: HashMap<String, DateTime<Utc>>,
    claimed_ever: HashSet<String>,
    owner_token: HashMap<String, u64>,
}

impl Supervisor {
    pub fn new(stale_after_secs: u64) -> Self {
        Self {
            heartbeats: HashMap::new(),
            queue: Vec::new(),
            stale_after_secs,
            // Letta's fix used a 60s startup deadline; keep it independent of
            // the heartbeat bound so tightening one never silently moves the other.
            startup_deadline_secs: 60,
            execution_stall_after_secs: stale_after_secs.saturating_mul(2),
            admission_grace_secs: stale_after_secs,
            first_seen: HashMap::new(),
            ready_since: HashMap::new(),
            claimed_ever: HashSet::new(),
            owner_token: HashMap::new(),
        }
    }

    pub fn with_startup_deadline(mut self, secs: u64) -> Self {
        self.startup_deadline_secs = secs;
        self
    }

    pub fn with_execution_stall_after(mut self, secs: u64) -> Self {
        self.execution_stall_after_secs = secs;
        self
    }

    pub fn with_admission_grace(mut self, secs: u64) -> Self {
        self.admission_grace_secs = secs;
        self
    }

    /// Register (or re-register) a daemon and issue its fencing epoch.
    /// Re-registration invalidates every heartbeat from the previous
    /// incarnation and releases its claimed tasks back to the queue, so a
    /// stale owner can neither report nor hold work after a restart.
    pub fn register(&mut self, daemon_id: &str) -> u64 {
        let next = self.owner_token.get(daemon_id).copied().unwrap_or(0) + 1;
        self.owner_token.insert(daemon_id.into(), next);
        // A new incarnation starts clean: its seq baseline, startup clock,
        // and admission proof all reset, so the stale owner cannot shadow it.
        self.heartbeats.remove(daemon_id);
        self.first_seen.remove(daemon_id);
        self.ready_since.remove(daemon_id);
        self.claimed_ever.remove(daemon_id);
        for task in self.queue.iter_mut() {
            if task.claimed_by.as_deref() == Some(daemon_id) {
                task.claimed_by = None;
                task.claimed_at = None;
            }
        }
        next
    }

    /// Alias for [`Supervisor::register`] that reads as what it is: a new
    /// incarnation taking over from a possibly-still-heartbeating stale one.
    pub fn reregister(&mut self, daemon_id: &str) -> u64 {
        self.register(daemon_id)
    }

    /// Register a heartbeat. A lower-or-equal `seq` from the same daemon is
    /// rejected as a replay — heartbeats only move forward. A heartbeat
    /// carrying a fencing token older than the current epoch (or none, once
    /// an epoch exists) is rejected as a stale owner.
    pub fn heartbeat(&mut self, hb: Heartbeat) -> Result<(), String> {
        if let Some(epoch) = self.owner_token.get(&hb.daemon_id).copied() {
            if hb.owner_token != Some(epoch) {
                return Err(format!(
                    "stale owner for {}: heartbeat token {:?} != epoch {}",
                    hb.daemon_id, hb.owner_token, epoch
                ));
            }
        }
        if let Some(prev) = self.heartbeats.get(&hb.daemon_id) {
            if hb.seq <= prev.seq {
                return Err(format!("stale seq {} <= {} from {}", hb.seq, prev.seq, hb.daemon_id));
            }
        }
        self.first_seen.entry(hb.daemon_id.clone()).or_insert(hb.ts_utc);
        if hb.readiness == Readiness::Ready {
            self.ready_since.entry(hb.daemon_id.clone()).or_insert(hb.ts_utc);
        }
        self.heartbeats.insert(hb.daemon_id.clone(), hb);
        Ok(())
    }

    /// The single reason `tick` would flag this daemon, if any. Priority:
    /// stale heartbeat > self-reported stall/stop > startup deadline >
    /// execution stall > admission stall. A daemon can be alive, fresh, and
    /// still flagged — that is the whole point of the split.
    fn flag_reason(&self, hb: &Heartbeat, now: DateTime<Utc>) -> Option<&'static str> {
        if (now - hb.ts_utc).num_seconds() > self.stale_after_secs as i64 {
            return Some("heartbeat stale");
        }
        if matches!(hb.state, DaemonState::Stalled | DaemonState::Stopped)
            || hb.readiness == Readiness::Stalled
        {
            return Some("self-reported stalled");
        }
        if matches!(
            hb.state,
            DaemonState::Starting
        ) || matches!(
            hb.readiness,
            Readiness::Connected | Readiness::Recovering
        ) {
            if let Some(seen) = self.first_seen.get(&hb.daemon_id) {
                if (now - *seen).num_seconds() > self.startup_deadline_secs as i64 {
                    return Some("startup deadline exceeded");
                }
            }
        }
        // Execution stall: a claimed task uncompleted past the bound, whatever
        // the daemon reports. Heartbeats do not refresh this clock; only
        // completion or explicit progress does.
        for task in &self.queue {
            if task.claimed_by.as_deref() == Some(hb.daemon_id.as_str()) {
                if let Some(at) = task.claimed_at {
                    if (now - at).num_seconds() > self.execution_stall_after_secs as i64 {
                        return Some("execution stalled");
                    }
                }
            }
        }
        // Admission stall: Ready, has never proven admission by claiming,
        // work waiting unclaimed past the grace.
        if hb.readiness == Readiness::Ready && !self.claimed_ever.contains(&hb.daemon_id) {
            if self.queue.iter().any(|t| t.claimed_by.is_none()) {
                if let Some(since) = self.ready_since.get(&hb.daemon_id) {
                    if (now - *since).num_seconds() > self.admission_grace_secs as i64 {
                        return Some("admission stalled");
                    }
                }
            }
        }
        None
    }

    /// Health verdict for one daemon, or `None` if it never heartbeated.
    pub fn health(&self, daemon_id: &str, now: DateTime<Utc>) -> Option<DaemonHealth> {
        let hb = self.heartbeats.get(daemon_id)?;
        let alive = (now - hb.ts_utc).num_seconds() <= self.stale_after_secs as i64;
        let reason = self.flag_reason(hb, now);
        Some(DaemonHealth {
            daemon_id: daemon_id.into(),
            alive,
            readiness: hb.readiness,
            flagged: reason.is_some(),
            reason,
        })
    }

    pub fn readiness_of(&self, daemon_id: &str) -> Option<Readiness> {
        self.heartbeats.get(daemon_id).map(|hb| hb.readiness)
    }

    /// Daemon ids the supervisor flags at `now`: stale heartbeats, self-reported
    /// stalls, startup-deadline breaches, execution stalls, admission stalls.
    /// Sorted for determinism. A fresh heartbeat alone never keeps a daemon
    /// off this list.
    pub fn tick(&self, now: DateTime<Utc>) -> Vec<String> {
        let mut out: Vec<String> = self
            .heartbeats
            .values()
            .filter(|hb| self.flag_reason(hb, now).is_some())
            .map(|hb| hb.daemon_id.clone())
            .collect();
        out.sort();
        out.dedup();
        out
    }

    pub fn enqueue(&mut self, id: &str, payload: &str) {
        self.queue.push(TaskItem { id: id.into(), payload: payload.into(), claimed_by: None, claimed_at: None });
    }

    /// A daemon claims the oldest unclaimed task at `now`. A daemon the
    /// supervisor knows about must be Ready or Executing to be admitted;
    /// Connected / Recovering / Stalled daemons are refused, so a heartbeat
    /// never substitutes for admission. Unknown daemons take the legacy path
    /// (admitted) so pre-readiness callers keep working.
    pub fn claim_at(&mut self, daemon_id: &str, now: DateTime<Utc>) -> Option<TaskItem> {
        if let Some(hb) = self.heartbeats.get(daemon_id) {
            if !matches!(hb.readiness, Readiness::Ready | Readiness::Executing) {
                return None;
            }
        }
        let item = self.queue.iter_mut().find(|t| t.claimed_by.is_none())?;
        item.claimed_by = Some(daemon_id.into());
        item.claimed_at = Some(now);
        self.claimed_ever.insert(daemon_id.into());
        Some(item.clone())
    }

    /// Legacy convenience wrapper; deterministic callers should use `claim_at`.
    pub fn claim(&mut self, daemon_id: &str) -> Option<TaskItem> {
        self.claim_at(daemon_id, Utc::now())
    }

    /// Mark a claimed task complete. Returns false unless this daemon held it.
    pub fn complete(&mut self, daemon_id: &str, task_id: &str) -> bool {
        if let Some(pos) = self.queue.iter().position(|t| {
            t.id == task_id && t.claimed_by.as_deref() == Some(daemon_id)
        }) {
            self.queue.remove(pos);
            return true;
        }
        false
    }

    /// A claimed task reports real progress, refreshing its stall clock.
    /// This is the legitimate escape hatch for long tasks: progress must be
    /// reported, not merely heartbeated.
    pub fn report_progress(&mut self, daemon_id: &str, task_id: &str, now: DateTime<Utc>) -> bool {
        if let Some(task) = self.queue.iter_mut().find(|t| {
            t.id == task_id && t.claimed_by.as_deref() == Some(daemon_id)
        }) {
            task.claimed_at = Some(now);
            return true;
        }
        false
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

    fn hb(id: &str, state: DaemonState, readiness: Readiness, seq: u64, ts: DateTime<Utc>) -> Heartbeat {
        Heartbeat::new(id, Role::Worker, state, seq, ts).with_readiness(readiness)
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

    // ---- WIRE-3: liveness is not readiness ----

    #[test]
    fn heartbeating_blocked_worker_is_detected_as_stalled() {
        // The required proof: fresh heartbeats every 10s, a claimed task that
        // never completes and never reports progress. Healthy-looking, dead
        // in fact — and the supervisor must say so.
        let mut s = Supervisor::new(60); // execution stall default: 2 * 60 = 120s
        s.enqueue("t1", "work");
        s.heartbeat(hb("w1", DaemonState::Running, Readiness::Ready, 1, t(1000))).unwrap();
        assert_eq!(s.claim_at("w1", t(1000)).unwrap().id, "t1");
        // Worker keeps heartbeating, now Executing, still not completing.
        for (i, ts) in [1010, 1020, 1030, 1040, 1050, 1060, 1070, 1080, 1090, 1100, 1110].iter().enumerate() {
            s.heartbeat(hb("w1", DaemonState::Running, Readiness::Executing, 2 + i as u64, t(*ts))).unwrap();
        }
        // Inside the stall window: not yet flagged, despite no progress.
        assert!(s.tick(t(1100)).is_empty());
        // One fresh heartbeat past the window, still no completion/progress.
        s.heartbeat(hb("w1", DaemonState::Running, Readiness::Executing, 13, t(1121))).unwrap();
        assert_eq!(s.tick(t(1121)), vec!["w1".to_string()]);
        let h = s.health("w1", t(1121)).unwrap();
        assert!(h.alive, "heartbeat is fresh — liveness must still read alive");
        assert!(h.flagged, "fresh liveness must not mask an execution stall");
        assert_eq!(h.reason, Some("execution stalled"));
        assert_eq!(h.readiness, Readiness::Executing);
    }

    #[test]
    fn startup_deadline_flags_blocked_startup_despite_fresh_heartbeats() {
        // Letta's case: startup blocked indefinitely, listener heartbeating.
        let mut s = Supervisor::new(60); // startup deadline: 60s, independent
        for (i, ts) in [1000, 1010, 1020, 1030, 1040, 1050, 1060].iter().enumerate() {
            s.heartbeat(hb("w1", DaemonState::Starting, Readiness::Connected, 1 + i as u64, t(*ts))).unwrap();
        }
        assert!(s.tick(t(1059)).is_empty());
        s.heartbeat(hb("w1", DaemonState::Starting, Readiness::Connected, 8, t(1061))).unwrap();
        assert_eq!(s.tick(t(1061)), vec!["w1".to_string()]);
        assert_eq!(s.health("w1", t(1061)).unwrap().reason, Some("startup deadline exceeded"));
        // A daemon that actually becomes Ready before the deadline is fine,
        // even much later, as long as its heartbeats stay fresh.
        let mut s2 = Supervisor::new(60);
        s2.heartbeat(hb("w2", DaemonState::Starting, Readiness::Connected, 1, t(1000))).unwrap();
        s2.heartbeat(hb("w2", DaemonState::Running, Readiness::Ready, 2, t(1030))).unwrap();
        s2.heartbeat(hb("w2", DaemonState::Running, Readiness::Ready, 3, t(1990))).unwrap();
        assert!(s2.tick(t(2000)).is_empty());
    }

    #[test]
    fn ready_worker_that_never_admits_is_flagged() {
        let mut s = Supervisor::new(60); // admission grace: 60s
        s.enqueue("t1", "waiting");
        s.heartbeat(hb("w1", DaemonState::Running, Readiness::Ready, 1, t(1000))).unwrap();
        // Fresh heartbeats, zero claims, work waiting.
        s.heartbeat(hb("w1", DaemonState::Running, Readiness::Ready, 2, t(1059))).unwrap();
        assert!(s.tick(t(1059)).is_empty());
        s.heartbeat(hb("w1", DaemonState::Running, Readiness::Ready, 3, t(1061))).unwrap();
        assert_eq!(s.tick(t(1061)), vec!["w1".to_string()]);
        assert_eq!(s.health("w1", t(1061)).unwrap().reason, Some("admission stalled"));
    }

    #[test]
    fn heartbeat_alone_never_admits_to_work() {
        let mut s = Supervisor::new(60);
        s.enqueue("t1", "work");
        // Connected (heartbeating) but not Ready: claim refused.
        s.heartbeat(hb("w1", DaemonState::Running, Readiness::Connected, 1, t(1000))).unwrap();
        assert!(s.claim_at("w1", t(1000)).is_none());
        assert_eq!(s.queue_depth(), 1);
        // Same daemon, now Ready: admitted.
        s.heartbeat(hb("w1", DaemonState::Running, Readiness::Ready, 2, t(1001))).unwrap();
        assert_eq!(s.claim_at("w1", t(1001)).unwrap().id, "t1");
    }

    #[test]
    fn stale_owner_fenced_on_reregistration() {
        let mut s = Supervisor::new(60);
        s.enqueue("t1", "work");
        let tok1 = s.register("w1");
        s.heartbeat(hb("w1", DaemonState::Running, Readiness::Ready, 1, t(1000)).with_owner(tok1)).unwrap();
        assert_eq!(s.claim_at("w1", t(1000)).unwrap().id, "t1");
        assert_eq!(s.queue_depth(), 0);
        // Old incarnation restarts: new epoch, its claims are released.
        let tok2 = s.reregister("w1");
        assert!(tok2 > tok1);
        assert_eq!(s.queue_depth(), 1, "stale owner's claim must be released");
        // Stale owner's heartbeat (even with a higher seq) is rejected.
        assert!(s.heartbeat(hb("w1", DaemonState::Running, Readiness::Ready, 99, t(1001)).with_owner(tok1)).is_err());
        assert!(s.heartbeat(hb("w1", DaemonState::Running, Readiness::Ready, 99, t(1001))).is_err());
        // New incarnation heartbeats fine.
        assert!(s.heartbeat(hb("w1", DaemonState::Running, Readiness::Ready, 1, t(1001)).with_owner(tok2)).is_ok());
    }

    #[test]
    fn progress_resets_execution_stall_clock() {
        let mut s = Supervisor::new(60);
        s.enqueue("t1", "long job");
        s.heartbeat(hb("w1", DaemonState::Running, Readiness::Ready, 1, t(1000))).unwrap();
        s.claim_at("w1", t(1000)).unwrap();
        s.heartbeat(hb("w1", DaemonState::Running, Readiness::Executing, 2, t(1100))).unwrap();
        assert!(s.report_progress("w1", "t1", t(1100)));
        // 121s after claim but only 21s after progress: not stalled.
        s.heartbeat(hb("w1", DaemonState::Running, Readiness::Executing, 3, t(1121))).unwrap();
        assert!(s.tick(t(1121)).is_empty());
        // 121s after the last progress report: stalled.
        s.heartbeat(hb("w1", DaemonState::Running, Readiness::Executing, 4, t(1221))).unwrap();
        assert_eq!(s.tick(t(1221)), vec!["w1".to_string()]);
    }
}
