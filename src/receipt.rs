//! Worker discipline — the 2026-03-18 standing order, as code.
//!
//! "Do not say 'done' as if you are the auditor. ... Prove it. Hand it off at
//! ready_for_audit. Exit cleanly."
//!
//! A [`WorkerReceipt`] accumulates evidence while work runs. `ready_for_audit()`
//! fails outright when no evidence is attached — there is no path from "no proof"
//! to "done".

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// What the auditor receives: the claim of completion plus the evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditHandoff {
    pub worker_id: String,
    pub task: String,
    pub evidence: Vec<String>,
    pub handed_off_utc: String,
}

/// Why a handoff was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiptError {
    /// The worker tried to finish with nothing to show.
    NoEvidence,
}

impl std::fmt::Display for ReceiptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReceiptError::NoEvidence => write!(
                f,
                "refused: no evidence attached — do not say 'done' as if you are the auditor"
            ),
        }
    }
}

impl std::error::Error for ReceiptError {}

/// A worker's running record of one assigned task.
#[derive(Debug, Clone)]
pub struct WorkerReceipt {
    worker_id: String,
    task: String,
    evidence: Vec<String>,
    started_utc: DateTime<Utc>,
}

impl WorkerReceipt {
    pub fn new(worker_id: &str, task: &str) -> Self {
        Self {
            worker_id: worker_id.into(),
            task: task.into(),
            evidence: Vec::new(),
            started_utc: Utc::now(),
        }
    }

    /// Attach one piece of proof: a test log, a file path, a measured number.
    pub fn add_evidence(&mut self, evidence: &str) {
        self.evidence.push(evidence.into());
    }

    pub fn evidence_count(&self) -> usize {
        self.evidence.len()
    }

    /// Hand off for audit. Fails with [`ReceiptError::NoEvidence`] when empty.
    pub fn ready_for_audit(&self) -> Result<AuditHandoff, ReceiptError> {
        if self.evidence.is_empty() {
            return Err(ReceiptError::NoEvidence);
        }
        Ok(AuditHandoff {
            worker_id: self.worker_id.clone(),
            task: self.task.clone(),
            evidence: self.evidence.clone(),
            handed_off_utc: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        })
    }

    pub fn started_utc(&self) -> DateTime<Utc> {
        self.started_utc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_receipt_refuses_handoff() {
        let r = WorkerReceipt::new("w1", "build the thing");
        let err = r.ready_for_audit().unwrap_err();
        assert_eq!(err, ReceiptError::NoEvidence);
    }

    #[test]
    fn error_message_names_the_rule() {
        let r = WorkerReceipt::new("w1", "build the thing");
        let msg = r.ready_for_audit().unwrap_err().to_string();
        assert!(msg.contains("auditor"));
    }

    #[test]
    fn evidence_allows_handoff() {
        let mut r = WorkerReceipt::new("w1", "build the thing");
        r.add_evidence("cargo test: 34 passed, 0 failed");
        r.add_evidence("~/workspace/lucidota-core/target/release/lucid");
        let h = r.ready_for_audit().unwrap();
        assert_eq!(h.worker_id, "w1");
        assert_eq!(h.evidence.len(), 2);
    }

    #[test]
    fn handoff_carries_utc_timestamp() {
        let mut r = WorkerReceipt::new("w1", "t");
        r.add_evidence("proof");
        let h = r.ready_for_audit().unwrap();
        assert!(h.handed_off_utc.ends_with('Z'));
        assert!(h.handed_off_utc.len() == 20);
    }

    #[test]
    fn blank_evidence_string_still_counts_as_attachment_shape() {
        // Honest limitation: the receipt enforces *presence*, not quality.
        // Quality is the auditor's job — this is stated, not hidden.
        let mut r = WorkerReceipt::new("w1", "t");
        r.add_evidence("x");
        assert!(r.ready_for_audit().is_ok());
    }
}
