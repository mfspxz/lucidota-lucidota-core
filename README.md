# LUCIDOTA CORE

Deterministic exocortex kernel. Rust. No LLM in the core — judgment stays human.

## What it is

The personal-agent core compiled directly from the operator's own standing orders.
Every module is a 2026-02-18 → 2026-05-01 chat-corp directive, turned into code:

| Module | Standing order (verified from chat corpus) |
|---|---|
| `correction` | Demand #4: an AI that learns from his corrections and retains what he teaches. Append-only, hash-chained correction memory, seeded with his 10 verified standing orders. |
| `gate` | 2026-04-25 Claim Evidence Gate. Trinary verdicts only — Proven / Disproven / Unknown. A claim with no evidence is Unknown and flagged as a claim-laundering suspect. |
| `receipt` | 2026-03-18 worker discipline. A worker may not say "done"; it hands off at `ready_for_audit` with evidence attached, or the handoff is refused. |
| `fleet` | Demand #5: the 24/7 daemon fleet pattern he built himself. Foreman / Worker / Auditor roles, heartbeats with replay protection, task queue, stale-daemon detection. Deterministic — `tick` takes the time explicitly. |
| `lint` | 2026-03-07 banned hedge-phrase list ("should now", "likely", "it seems", …). Scans text, exits 1 with rewrite guidance. |

## Spec provenance

Not re-derived. The spec is `~/workspace/chat-ingest/work/analysis-chatgpt.md`
(genesis window: LUCIDOTA_BOOT.md identity, Porges Protocol, Claim Evidence Gate,
worker discipline) and `~/workspace/chat-ingest/work/demands.md` (items #4, #5).

## The `lucid` CLI

```
lucid correct add --rule "..." --category discipline --source "2026-10-08 side chat"
lucid correct list [--category epistemics]
lucid correct find porges
lucid lint <file>                                   # exits 1 on banned phrases
lucid gate --claim "..." --evidence "cargo test log" [--evidence "?unverified tip"] [--evidence "!contradiction"]
```

Correction store: `$LUCIDOTA_CORRECTIONS`, else `~/.local/share/lucidota/corrections.jsonl`.
First open seeds the 10 verified standing orders; every later add is hash-chained
append-only. Tamper with one byte and `open` refuses the store.

## Build

```
cargo test    # 33 tests, all green
cargo build --release
```

## Honest limits (stated, not hidden)

- The receipt enforces evidence *presence*, not quality — quality is the auditor's job.
- The gate trusts the `verified` flag on evidence refs; verification itself is human.
- The fleet is types + logic, not a running supervisor — wire `tick` to your own loop.
- Corrections are the operator's words. New ones come from him, not from the machine.
