//! Claim Evidence Gate — the 2026-04-25 standing order, as code.
//!
//! Trinary logic only: [`Verdict::Proven`], [`Verdict::Disproven`], [`Verdict::Unknown`].
//! A claim that cites no evidence is not "probably fine" — it is Unknown and flagged
//! as a claim-laundering suspect. The Sophia invariant holds: adapting the interface
//! never changes what counts as evidence.

use serde::{Deserialize, Serialize};

/// Trinary verdict. There is no "true" or "false" here — only what the evidence holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    Proven,
    Disproven,
    Unknown,
}

/// One pointer to evidence behind a claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRef {
    /// Where the evidence lives: file path, ledger seq, URL, test name, etc.
    pub pointer: String,
    /// Has a human or a test actually checked it?
    pub verified: bool,
    /// Does it contradict the claim?
    pub contradicts: bool,
}

impl EvidenceRef {
    pub fn verified(pointer: &str) -> Self {
        Self { pointer: pointer.into(), verified: true, contradicts: false }
    }
    pub fn unverified(pointer: &str) -> Self {
        Self { pointer: pointer.into(), verified: false, contradicts: false }
    }
    pub fn contradicting(pointer: &str) -> Self {
        Self { pointer: pointer.into(), verified: true, contradicts: true }
    }
}

/// A claim awaiting judgment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    pub text: String,
    pub evidence: Vec<EvidenceRef>,
}

impl Claim {
    pub fn new(text: &str) -> Self {
        Self { text: text.into(), evidence: Vec::new() }
    }
    pub fn with_evidence(mut self, e: EvidenceRef) -> Self {
        self.evidence.push(e);
        self
    }
}

/// The gate's judgment on a claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateVerdict {
    pub verdict: Verdict,
    /// True when the claim carries zero evidence — the claim-laundering signature:
    /// performative, fake, did no work at all, tried nothing, gave up, said success.
    pub claim_laundering_suspect: bool,
    pub notes: Vec<String>,
}

/// Run the gate. Deterministic: same claim, same verdict, every time.
///
/// HONEST BOUNDS (R-2): the gate enforces claim STRUCTURE, not truth. It
/// trusts the `verified` flag the CALLER set on each EvidenceRef — a caller
/// that marks garbage `verified` gets `Proven`. Contradictions beat support
/// and empty evidence is flagged as claim-laundering, but "verified" here
/// means "someone asserted verification", not "this gate verified it". Do
/// not cite a `Proven` verdict as independent verification.
pub fn evaluate(claim: &Claim) -> GateVerdict {
    let mut notes = Vec::new();
    if claim.evidence.iter().any(|e| e.contradicts) {
        notes.push("contradicting evidence present".into());
        return GateVerdict { verdict: Verdict::Disproven, claim_laundering_suspect: false, notes };
    }
    if claim.evidence.is_empty() {
        notes.push("no evidence cited — claim laundering signature".into());
        return GateVerdict { verdict: Verdict::Unknown, claim_laundering_suspect: true, notes };
    }
    if claim.evidence.iter().any(|e| e.verified) {
        let n = claim.evidence.iter().filter(|e| e.verified).count();
        notes.push(format!("{} verified evidence ref(s)", n));
        return GateVerdict { verdict: Verdict::Proven, claim_laundering_suspect: false, notes };
    }
    notes.push("evidence cited but none verified".into());
    GateVerdict { verdict: Verdict::Unknown, claim_laundering_suspect: false, notes }
}

#[cfg(test)]
mod tests {
    // PROPERTY TESTS (R-7) — gate-behavior invariants: no-evidence is
    // Unknown, contradiction beats support, verified-only-if-flagged.
    // Expectations here may never be rewritten to match a behavior change
    // without adversarial sign-off.
    use super::*;

    #[test]
    fn no_evidence_is_unknown_and_laundering_suspect() {
        let v = evaluate(&Claim::new("it works"));
        assert_eq!(v.verdict, Verdict::Unknown);
        assert!(v.claim_laundering_suspect);
    }

    #[test]
    fn verified_evidence_proves() {
        let c = Claim::new("172 tests green").with_evidence(EvidenceRef::verified("cargo test log"));
        let v = evaluate(&c);
        assert_eq!(v.verdict, Verdict::Proven);
        assert!(!v.claim_laundering_suspect);
    }

    #[test]
    fn unverified_evidence_stays_unknown() {
        let c = Claim::new("probably fine").with_evidence(EvidenceRef::unverified("some guy said so"));
        let v = evaluate(&c);
        assert_eq!(v.verdict, Verdict::Unknown);
        assert!(!v.claim_laundering_suspect);
    }

    #[test]
    fn contradicting_evidence_disproves() {
        let c = Claim::new("all green")
            .with_evidence(EvidenceRef::verified("pass log"))
            .with_evidence(EvidenceRef::contradicting("failing test: token_count_snapshot"));
        let v = evaluate(&c);
        assert_eq!(v.verdict, Verdict::Disproven);
    }

    #[test]
    fn contradiction_beats_verified_support() {
        let c = Claim::new("shipped")
            .with_evidence(EvidenceRef::verified("deploy log"))
            .with_evidence(EvidenceRef::contradicting("404 on /api/tools/*"));
        assert_eq!(evaluate(&c).verdict, Verdict::Disproven);
    }

    #[test]
    fn empty_claim_text_still_judged_on_evidence() {
        let v = evaluate(&Claim::new(""));
        assert_eq!(v.verdict, Verdict::Unknown);
        assert!(v.claim_laundering_suspect);
    }

    #[test]
    fn deterministic_same_claim_same_verdict() {
        let c = Claim::new("x").with_evidence(EvidenceRef::verified("y"));
        assert_eq!(evaluate(&c), evaluate(&c));
    }
}
