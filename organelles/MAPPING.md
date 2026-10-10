# lucidota-core — organelle mapping decisions

Decomposition reviewed 2026-10-10 under the corrected doctrine
(decompose ACTUAL capabilities; map-or-design per unit, justified).

## What the code actually does

`lucidota-core` is a small (7-file) deterministic exocortex kernel library +
`lucid` CLI. Five modules, five genuinely different capabilities — but one
indivisible packaging: a single lib crate, no workspace, shared
deterministic discipline. The seam is **op-level, not organelle-level**
(same argument the ledger-graph pilot used for parse+view): splitting five
modules that share one crate and one discipline into five organelles would
manufacture five hosts for one kernel. One organelle (`ld`), thirteen ops,
per-op `mutation_class` + `required_capability` carry the differences the
manifest schema was designed for.

## Capability units → ops

| Unit | Source | Ops | Decision |
|---|---|---|---|
| Correction memory: append-only, hash-chained store of the operator's standing orders, seeded from `corrections-seed.jsonl` | `correction.rs` (CorrectionStore::open/add/list/find/verify_chain) | `correct_add` (Write, `ld.correct.write`), `correct_list`, `correct_find` (ReadOnly) | **Bespoke** (`ld`). Justification: the computation is a domain hash chain (`sha256(prev\|\|id\|\|ts\|\|rule\|\|category\|\|source)`), seed-on-first-open, and chain-verification-on-load. `ref-ledger-append` is a generic blind appender — it knows nothing of seed corrections, chain verification, or the correction schema. Adopting it would flatten the unit's identity into a file append. |
| Claim Evidence Gate: trinary verdict over claim + evidence refs | `gate.rs` (Claim, EvidenceRef, `evaluate`) | `gate` (ReadOnly, Proven) | **Bespoke** (`ld`). Justification: pure domain logic (trinary verdict + claim-laundering detection). `ref-json-validate` checks schema conformance; this *adjudicates*. Different computation. |
| Banned hedge-phrase scanner | `lint.rs` (`lint_text`) | `lint` (ReadOnly, Proven) | **Bespoke** (`ld`). Justification: `ref-text-transform` transforms text; this scans against the operator's standing banned-phrase list and returns located hits. A transform with a fixed rule set is not a scan. |
| Worker receipt discipline: evidence-gated handoff | `receipt.rs` (WorkerReceipt::new/add_evidence/ready_for_audit) | `receipt_new`, `receipt_attach`, `receipt_handoff` (ReadOnly — stateless-by-value) | **Bespoke** (`ld`). Justification: no ref organelle models evidence-gated handoff. Ops are stateless-by-value: the receipt travels as JSON between calls; the organelle never holds worker state. `started_utc` resets on rebuild (documented in execution.json) — the discipline (refusal without evidence) is preserved, which is the unit's whole point. |
| Daemon fleet supervisor: heartbeats, task queue, stale detection | `fleet.rs` (Supervisor, Heartbeat, TaskItem) | `fleet_heartbeat`, `fleet_tick`, `fleet_enqueue`, `fleet_claim`, `fleet_status` (Write, `ld.fleet.write`) | **Bespoke** (`ld`). Justification: deterministic fleet supervision is not any ref organelle's computation. The organelle owns one `Mutex<Supervisor>`; `tick` takes explicit time (the library's honesty rule) — the op accepts an optional `now_ms`, defaulting to real time. |

## Deliberate exclusions

- `src/bin/lucid.rs` (the `lucid` CLI): a human CLI entrypoint, not an
  independently callable capability — the organelle ops expose the same
  functions the CLI calls. Exposing the CLI as an op would add a process
  boundary with no new contract.

## Reference-organelle check (all 12 considered)

- `ref-ledger-append`: generic appends; correction memory needs seed +
  chain-verify + domain schema. Rejected (wrong layer).
- `ref-json-validate`: schema conformance; the gate adjudicates. Rejected.
- `ref-text-transform`: transforms; lint scans. Rejected.
- `ref-kv-store`, `ref-hash-seal`, `ref-time-oracle`, others: no overlap.
- Verdict: genuinely bespoke. Nothing reinvented that already existed.

## Execution facts

- Valence Proven (deterministic; correction store verifies its chain on open,
  fail-closed on tampering).
- fs: correction store path (default `$LUCIDOTA_CORRECTIONS` else
  `~/.local/share/lucidota/corrections.jsonl`), read-write for `correct_add`,
  read-only for the rest. No network, no secrets.
- Stateful parts: one `Mutex<Supervisor>` (fleet) — suspend = no-op
  (deterministic; tick takes explicit time), kill = drop. Correction store is
  file-backed and re-verified on every open.
- Envelope: 1 cpu / 256 MB / 30 s wall-time.
- Receipts: content-addressed sha256, chained; `verify_chain` green in tests.
