//! LUCIDOTA CORE — deterministic exocortex kernel.
//!
//! Modules:
//! - [`correction`]: append-only, hash-chained correction memory. Corrections are
//!   compiled once and enforced forever — the operator's scar tissue with a version number.
//! - [`gate`]: Claim Evidence Gate. Trinary verdicts (Proven / Disproven / Unknown).
//!   A claim with no evidence is Unknown and flagged as a claim-laundering suspect.
//! - [`receipt`]: worker discipline. A worker may not say "done"; it hands off at
//!   `ready_for_audit` with evidence attached, or it fails.
//! - [`fleet`]: the 24/7 daemon fleet pattern — Foreman / Worker / Auditor roles,
//!   heartbeats, task queue, stale-daemon detection. Deterministic; no threads.
//! - [`lint`]: banned hedge-phrase scanner (the 2026-03-07 standing list).

pub mod correction;
pub mod fleet;
pub mod gate;
pub mod lint;
pub mod organelle;
pub mod receipt;
