# AUDIT-APPLICATION — lucidota-core

**Slice:** brutal-audit rollout 3/8. **Date:** 2026-10-10. **Repo:** ~/workspace/lucidota-core.
**Audited at:** `c0e49ea`. **This pass:** `78ad3ff`.

## What this is

Deterministic exocortex kernel in Rust: `correction.rs` (append-only
hash-chained `CorrectionStore` of operator rules, seeded from
`corrections-seed.jsonl`), `gate.rs` (claim gate: trinary verdicts from
evidence refs), `lint.rs` (hedge-phrase style scanner), `receipt.rs`
(`WorkerReceipt`/`AuditHandoff`), `fleet.rs`, `organelle.rs`.

## Per-finding application

- **R-1 (instance-only fix claims, one book):** N/A. No fix-verification claim
  log; corrections are operator rules, not fix claims.
- **R-2 (honest guardrail bounds):** APPLIED. Documented on
  `gate.rs::evaluate()`: the gate enforces claim STRUCTURE, not truth. It
  trusts the caller-set `verified` flag on `EvidenceRef` — a caller marking
  garbage verified gets `Proven`. "Verified" means "someone asserted
  verification", not "this gate verified it". Do not cite a `Proven` verdict
  as independent verification.
- **R-3 (out-of-band head copy):** APPLIED.
  `~/workspace/dogfood/audit/oob-heads/lucidota-core.json` — seed head: seq
  10, head_hash `70fa4d9a4603154136ae4aa427aba92d776882b4588347ff6f1be9d7596a4b43`,
  head id 9, recorded 2026-10-10. Documented: the seed fixture is static and
  committed; the LIVE store lives outside the repo (`$LUCIDOTA_CORRECTIONS`
  else `~/.local/share/lucidota/corrections.jsonl`) and grows by append — a
  live head copy must be taken from the deployed path; until then the live
  chain is a structured log, not tamper-evident. Same-VM limitation and
  update procedure documented.
- **R-4 (backlog triage):** N/A. No defect backlog in-repo.
- **R-5 (signature games):** N/A. `lint.rs` is a prose-style scanner (Zack's
  banned hedge phrases from the 2026-03-07 anti-bullshit thread); it makes no
  security claim. Paraphrase evasion is irrelevant to its purpose (flagging
  the author's own hedges).
- **R-6 (detector + consumer + gate):** N/A as a gap. No warn-only detectors;
  the gate IS the enforcement: no evidence → `Unknown` + claim-laundering
  flag; `ready_for_audit()` refuses empty handoffs outright.
- **R-7 (property vs change-detection):** APPLIED. Test-module markers:
  `gate.rs` and `correction.rs` PROPERTY (gate-behavior invariants; chain
  invariants — seeded records verify, tampered links fail, appends preserve
  the chain); `lint.rs` CHANGE-DETECTION (asserts the current banned-phrase
  list and matching semantics). Policy: property expectations are never
  rewritten to match a behavior change without adversarial sign-off.
- **R-8 (unsigned provenance):** APPLIED. `WorkerReceipt`/`AuditHandoff`
  declared honestly-unsigned in `src/receipt.rs`: evidence-attached, not
  signed; verification power is "the auditor can check the attached
  evidence", not provenance. The audit step is the control, not the struct.
- **R-9 (frozen claim schema):** N/A. The corrections log has no verdict
  field; records are rules (`id/ts/rule/category/source/prev_hash/hash`),
  not claims. Append-only schema documented in `correction.rs`.
- **R-10 (record-keeping in the same motion):** APPLIED. All changes written
  in this motion, 2026-10-10.
- **R-11:** explicit UNKNOWN list below.
- **R-12 (no unsourced footnotes):** APPLIED. No unsourced oral footnotes
  found; the seed's source strings are noted in R-11 as UNVERIFIED provenance
  notes, not evidence claims.

## Commits

This pass (`TBD`): src/receipt.rs, src/gate.rs, src/lint.rs,
src/correction.rs (docstrings/test markers), this file.

## Test results

Measured 2026-10-10: **36/36 green**.

## Wont-fix list (R-4)

None — no backlog, no standing HIGHs.

## Explicit UNKNOWN list (R-11)

1. Live corrections store contents — outside the repo, not inspected this
   pass.
2. Who sets `EvidenceRef.verified` in production — the gate's trust root.
3. `fleet.rs` / `organelle.rs` behavior — not reviewed this pass.
4. `corrections-seed.jsonl` source strings cite chat threads (e.g.
   "2026-02-18 LUCIDOTA_BOOT.md") — provenance notes; contents not
   independently verified this pass; treated as UNVERIFIED.

## Top critical issues

1. **The gate trusts the caller-set `verified` flag.** `Proven` is not
   independent verification — any caller can mark garbage verified and get
   `Proven`. The trust root (who sets the flag in production) is unknown.
2. **Live corrections store unexamined.** It lives outside the repo; until a
   deployed live head is copied out-of-band, the live chain is a structured
   log, not tamper-evident.
3. **Seed provenance unverified.** The seed's source strings cite chat
   threads whose contents were not independently verified this pass.
