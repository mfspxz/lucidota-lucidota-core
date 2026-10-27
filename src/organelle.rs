//! lucidota-core as an organelle (Tier A native migration).
//!
//! One organelle (`ld`), thirteen ops over the five kernel modules:
//!   correct_add / correct_list / correct_find  — hash-chained correction memory
//!   gate                                        — trinary claim-evidence gate (pure)
//!   lint                                        — banned hedge-phrase scan (pure)
//!   receipt_new / receipt_attach / receipt_handoff — worker receipt discipline,
//!       stateless-by-value: the receipt travels as JSON between calls; the
//!       organelle holds no worker state. `started_utc` resets on rebuild — the
//!       discipline (refusal without evidence) is what is preserved.
//!   fleet_heartbeat / fleet_tick / fleet_enqueue / fleet_claim / fleet_status
//!       — the organelle owns one Mutex<Supervisor>; tick takes explicit time.
//!
//! Execution personality: fs read-write on the corrections store only
//! (default $LUCIDOTA_CORRECTIONS else ~/.local/share/lucidota/corrections.jsonl).
//! No network, no secrets. Envelope: 1 cpu / 256 MB / 30 s.

use organelle_host::{CallContext, HostRef, Organelle, OrganelleError};
use organelle_manifest::{
    Authority, Contract, MutationClass, OrganelleManifest, ReceiptPolicy, Transport, Valence,
    MANIFEST_SCHEMA_VERSION, RECEIPT_ALGORITHM,
};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::correction::CorrectionStore;
use crate::fleet::{DaemonState, Heartbeat, Readiness, Role, Supervisor};
use crate::gate::{Claim, EvidenceRef};
use crate::{gate, lint, receipt};

fn str_in(v: &Value, key: &str) -> Result<String, OrganelleError> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| OrganelleError::failed(format!("missing string field {key:?}")))
}

fn opt_path(v: &Value) -> PathBuf {
    v.get("path")
        .and_then(|x| x.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(CorrectionStore::default_path)
}

fn open_store(v: &Value) -> Result<CorrectionStore, OrganelleError> {
    let path = opt_path(v);
    CorrectionStore::open(&path)
        .map_err(|e| OrganelleError::failed(format!("cannot open correction store: {e}")))
}

fn parse_role(s: &str) -> Result<Role, OrganelleError> {
    match s {
        "foreman" => Ok(Role::Foreman),
        "worker" => Ok(Role::Worker),
        "auditor" => Ok(Role::Auditor),
        _ => Err(OrganelleError::failed(format!("unknown role {s:?}"))),
    }
}

fn parse_readiness(s: &str) -> Result<Readiness, OrganelleError> {
    match s {
        // Conservative default: a bare heartbeat proves liveness, never readiness.
        "connected" | "" => Ok(Readiness::Connected),
        "ready" => Ok(Readiness::Ready),
        "executing" => Ok(Readiness::Executing),
        "stalled" => Ok(Readiness::Stalled),
        "recovering" => Ok(Readiness::Recovering),
        _ => Err(OrganelleError::failed(format!("unknown readiness {s:?}"))),
    }
}

fn parse_state(s: &str) -> Result<DaemonState, OrganelleError> {
    match s {
        "starting" => Ok(DaemonState::Starting),
        "running" => Ok(DaemonState::Running),
        "idle" => Ok(DaemonState::Idle),
        "stalled" => Ok(DaemonState::Stalled),
        "stopped" => Ok(DaemonState::Stopped),
        _ => Err(OrganelleError::failed(format!("unknown state {s:?}"))),
    }
}

/// The receipt travel form: worker_id + task + evidence carried as JSON.
/// Rebuilt into a real WorkerReceipt on every op (stateless-by-value).
fn travel_parts(v: &Value) -> Result<(String, String, Vec<String>), OrganelleError> {
    let worker_id = str_in(v, "worker_id")?;
    let task = str_in(v, "task")?;
    let evidence = v
        .get("evidence")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|e| e.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    Ok((worker_id, task, evidence))
}

fn rebuild_receipt(v: &Value) -> Result<(receipt::WorkerReceipt, String, String, Vec<String>), OrganelleError> {
    let (worker_id, task, evidence) = travel_parts(v)?;
    let mut r = receipt::WorkerReceipt::new(&worker_id, &task);
    for e in &evidence {
        r.add_evidence(e);
    }
    Ok((r, worker_id, task, evidence))
}

fn receipt_json(worker_id: &str, task: &str, evidence: &[String]) -> Value {
    // started_utc resets on rebuild (documented in MAPPING.md); the discipline
    // (refusal without evidence) is what the ops preserve.
    serde_json::json!({
        "worker_id": worker_id,
        "task": task,
        "evidence": evidence,
        "evidence_count": evidence.len(),
    })
}

fn contract(op: &str, description: &str, schema: Value) -> Contract {
    Contract { op: op.to_string(), description: description.to_string(), input_schema: schema, mutation_class: None, required_capability: None }
}

fn contract_gated(op: &str, description: &str, schema: Value, class: MutationClass, cap: &str) -> Contract {
    Contract { op: op.to_string(), description: description.to_string(), input_schema: schema, mutation_class: Some(class), required_capability: Some(cap.to_string()) }
}

fn obj_schema(props: Value, required: &[&str]) -> Value {
    serde_json::json!({
        "type": "object",
        "properties": props,
        "required": required,
    })
}

fn manifest() -> OrganelleManifest {
    let path_prop = serde_json::json!({"path": {"type": "string"}});
    let contracts = vec![
        contract_gated("correct_add", "append a correction to the hash-chained store",
            obj_schema(serde_json::json!({"path": {"type": "string"}, "rule": {"type": "string"}, "category": {"type": "string"}, "source": {"type": "string"}}), &["rule", "category", "source"]),
            MutationClass::Write, "ld.correct.write"),
        contract("correct_list", "list all corrections in the store", obj_schema(path_prop.clone(), &[])),
        contract("correct_find", "search corrections by substring",
            obj_schema(serde_json::json!({"path": {"type": "string"}, "query": {"type": "string"}}), &["query"])),
        contract("gate", "trinary gate verdict over a claim + evidence refs",
            obj_schema(serde_json::json!({"text": {"type": "string"}, "evidence": {"type": "array", "items": {"type": "object", "properties": {"pointer": {"type": "string"}, "verified": {"type": "boolean"}, "contradicts": {"type": "boolean"}}, "required": ["pointer"]}}}), &["text"])),
        contract("lint", "scan text for banned hedge phrases",
            obj_schema(serde_json::json!({"text": {"type": "string"}}), &["text"])),
        contract("receipt_new", "open a worker receipt (stateless-by-value JSON)",
            obj_schema(serde_json::json!({"worker_id": {"type": "string"}, "task": {"type": "string"}}), &["worker_id", "task"])),
        contract("receipt_attach", "attach evidence to a receipt JSON",
            obj_schema(serde_json::json!({"receipt": {"type": "object"}, "evidence": {"type": "string"}}), &["receipt", "evidence"])),
        contract("receipt_handoff", "attempt ready_for_audit handoff; refuses without evidence",
            obj_schema(serde_json::json!({"receipt": {"type": "object"}}), &["receipt"])),
        contract_gated("fleet_heartbeat", "record a daemon heartbeat",
            obj_schema(serde_json::json!({"daemon_id": {"type": "string"}, "role": {"type": "string", "enum": ["foreman", "worker", "auditor"]}, "state": {"type": "string", "enum": ["starting", "running", "idle", "stalled", "stopped"]}, "readiness": {"type": "string", "enum": ["connected", "ready", "executing", "stalled", "recovering"]}, "seq": {"type": "integer"}}), &["daemon_id", "role", "state", "seq"]),
            MutationClass::Write, "ld.fleet.write"),
        contract_gated("fleet_tick", "tick the supervisor; returns stale daemon ids",
            obj_schema(serde_json::json!({"now_ms": {"type": "integer"}}), &[]),
            MutationClass::Write, "ld.fleet.write"),
        contract_gated("fleet_enqueue", "enqueue a task",
            obj_schema(serde_json::json!({"id": {"type": "string"}, "payload": {"type": "string"}}), &["id", "payload"]),
            MutationClass::Write, "ld.fleet.write"),
        contract_gated("fleet_claim", "a daemon claims the next queued task",
            obj_schema(serde_json::json!({"daemon_id": {"type": "string"}}), &["daemon_id"]),
            MutationClass::Write, "ld.fleet.write"),
        contract("fleet_status", "supervisor counters: daemons, queue depth",
            obj_schema(serde_json::json!({}), &[])),
    ];
    OrganelleManifest {
        schema_version: MANIFEST_SCHEMA_VERSION.to_string(),
        name: "lucidota-core".to_string(),
        codon: "ld".to_string(),
        version: "0.1.0".to_string(),
        description: "deterministic exocortex kernel: correction memory, claim-evidence gate, hedge lint, worker receipt discipline, fleet supervision".to_string(),
        contracts,
        authority: Authority {
            mutation_class: MutationClass::ReadOnly,
            declared_valence: Valence::Proven,
            required_capability: None,
            granted_capabilities: vec![],
        },
        transports: Transport::all().to_vec(),
        receipt_policy: ReceiptPolicy { content_addressed: true, algorithm: RECEIPT_ALGORITHM.to_string(), chain: true },
    }
}

pub struct LucidotaCore {
    manifest: OrganelleManifest,
    supervisor: Mutex<Supervisor>,
}

pub fn organelle() -> Arc<dyn Organelle> {
    Arc::new(LucidotaCore { manifest: manifest(), supervisor: Mutex::new(Supervisor::new(300)) })
}

impl Organelle for LucidotaCore {
    fn manifest(&self) -> OrganelleManifest {
        self.manifest.clone()
    }

    fn handle(&self, _ctx: &CallContext, _host: &dyn HostRef, op: &str, input: Value) -> Result<Value, OrganelleError> {
        match op {
            "correct_add" => {
                let mut store = open_store(&input)?;
                let c = store.add(&str_in(&input, "rule")?, &str_in(&input, "category")?, &str_in(&input, "source")?)
                    .map_err(|e| OrganelleError::failed(format!("add failed: {e}")))?;
                Ok(serde_json::to_value(c).map_err(|e| OrganelleError::failed(e.to_string()))?)
            }
            "correct_list" => {
                let store = open_store(&input)?;
                Ok(serde_json::to_value(store.list()).map_err(|e| OrganelleError::failed(e.to_string()))?)
            }
            "correct_find" => {
                let store = open_store(&input)?;
                let hits = store.find(&str_in(&input, "query")?);
                Ok(serde_json::to_value(hits).map_err(|e| OrganelleError::failed(e.to_string()))?)
            }
            "gate" => {
                let mut claim = Claim::new(&str_in(&input, "text")?);
                if let Some(ev) = input.get("evidence").and_then(|x| x.as_array()) {
                    for e in ev {
                        let pointer = e.get("pointer").and_then(|x| x.as_str()).unwrap_or("").to_string();
                        let r = if e.get("contradicts").and_then(|x| x.as_bool()).unwrap_or(false) {
                            EvidenceRef::contradicting(&pointer)
                        } else if e.get("verified").and_then(|x| x.as_bool()).unwrap_or(false) {
                            EvidenceRef::verified(&pointer)
                        } else {
                            EvidenceRef::unverified(&pointer)
                        };
                        claim = claim.with_evidence(r);
                    }
                }
                let v = gate::evaluate(&claim);
                Ok(serde_json::json!({
                    "verdict": format!("{:?}", v.verdict).to_lowercase(),
                    "claim_laundering_suspect": v.claim_laundering_suspect,
                    "notes": v.notes,
                }))
            }
            "lint" => {
                let hits = lint::lint_text(&str_in(&input, "text")?);
                Ok(Value::Array(hits.into_iter().map(|h| serde_json::json!({
                    "phrase": h.phrase, "replacement": h.replacement,
                    "line": h.line, "context": h.context,
                })).collect()))
            }
            "receipt_new" => {
                let (worker_id, task) = (str_in(&input, "worker_id")?, str_in(&input, "task")?);
                let _r = receipt::WorkerReceipt::new(&worker_id, &task);
                Ok(receipt_json(&worker_id, &task, &[]))
            }
            "receipt_attach" => {
                let rv = input.get("receipt").ok_or_else(|| OrganelleError::failed("missing receipt"))?;
                let (_r, worker_id, task, mut evidence) = rebuild_receipt(rv)?;
                evidence.push(str_in(&input, "evidence")?);
                Ok(receipt_json(&worker_id, &task, &evidence))
            }
            "receipt_handoff" => {
                let rv = input.get("receipt").ok_or_else(|| OrganelleError::failed("missing receipt"))?;
                let (r, _wid, _task, _ev) = rebuild_receipt(rv)?;
                match r.ready_for_audit() {
                    Ok(h) => Ok(serde_json::json!({
                        "handed_off": true, "worker_id": h.worker_id, "task": h.task,
                        "evidence": h.evidence, "handed_off_utc": h.handed_off_utc,
                    })),
                    Err(e) => Ok(serde_json::json!({"handed_off": false, "refusal": e.to_string()})),
                }
            }
            "fleet_heartbeat" => {
                let hb = Heartbeat::now(&str_in(&input, "daemon_id")?, parse_role(input.get("role").and_then(|x| x.as_str()).unwrap_or(""))?, parse_state(input.get("state").and_then(|x| x.as_str()).unwrap_or(""))?,
                    input.get("seq").and_then(|x| x.as_u64()).unwrap_or(0))
                    .with_readiness(parse_readiness(input.get("readiness").and_then(|x| x.as_str()).unwrap_or("connected"))?);
                let mut s = self.supervisor.lock().map_err(|_| OrganelleError::failed("supervisor lock poisoned"))?;
                s.heartbeat(hb).map_err(OrganelleError::failed)?;
                Ok(serde_json::json!({"recorded": true}))
            }
            "fleet_tick" => {
                let now = input.get("now_ms").and_then(|x| x.as_i64())
                    .map(|ms| chrono::DateTime::from_timestamp_millis(ms).unwrap_or_else(chrono::Utc::now))
                    .unwrap_or_else(chrono::Utc::now);
                let s = self.supervisor.lock().map_err(|_| OrganelleError::failed("supervisor lock poisoned"))?;
                Ok(serde_json::json!({"stale": s.tick(now)}))
            }
            "fleet_enqueue" => {
                let mut s = self.supervisor.lock().map_err(|_| OrganelleError::failed("supervisor lock poisoned"))?;
                s.enqueue(&str_in(&input, "id")?, &str_in(&input, "payload")?);
                Ok(serde_json::json!({"enqueued": true, "queue_depth": s.queue_depth()}))
            }
            "fleet_claim" => {
                let mut s = self.supervisor.lock().map_err(|_| OrganelleError::failed("supervisor lock poisoned"))?;
                let item = s.claim(&str_in(&input, "daemon_id")?);
                Ok(serde_json::json!({"claimed": item}))
            }
            "fleet_status" => {
                let s = self.supervisor.lock().map_err(|_| OrganelleError::failed("supervisor lock poisoned"))?;
                Ok(serde_json::json!({"daemons": s.daemon_count(), "queue_depth": s.queue_depth()}))
            }
            _ => Err(OrganelleError::not_found(format!("unknown op {op}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use organelle_host::Host;
    use organelle_manifest::validate_manifest_json;

    fn manifest_json() -> Value {
        let txt = include_str!("../organelles/lucidota-core.manifest.json");
        serde_json::from_str(txt).expect("manifest JSON parses")
    }

    #[test]
    fn static_manifest_validates() {
        let m = validate_manifest_json(&manifest_json()).expect("static manifest must validate");
        assert_eq!(m.name, "lucidota-core");
        assert_eq!(m.codon, "ld");
    }

    #[test]
    fn rust_manifest_matches_static_json() {
        let from_rust = serde_json::to_value(manifest()).unwrap();
        assert_eq!(from_rust, manifest_json(), "Rust manifest() and organelles/*.json must agree");
    }

    #[test]
    fn registers_and_serves_through_host_with_receipts() {
        let host = Host::new();
        host.register(organelle()).expect("registration must succeed");
        let ctx = CallContext::new("ld-test", vec!["ld.correct.write".to_string(), "ld.fleet.write".to_string()]);

        // gate: no evidence -> Unknown + laundering suspect (pure, real code)
        let g = host.call(&ctx, "ld", "gate", serde_json::json!({"text": "it works", "evidence": []})).expect("gate");
        assert_eq!(g.output["verdict"], "unknown");
        assert_eq!(g.output["claim_laundering_suspect"], true);

        // gate: verified evidence -> Proven
        let g2 = host.call(&ctx, "ld", "gate", serde_json::json!({
            "text": "it works", "evidence": [{"pointer": "test-1", "verified": true}]}))
            .expect("gate proven");
        assert_eq!(g2.output["verdict"], "proven");

        // lint: banned phrase detected (pure, real code)
        let l = host.call(&ctx, "ld", "lint", serde_json::json!({"text": "it seems to work"})).expect("lint");
        assert!(l.output.as_array().unwrap().len() > 0);

        // receipt discipline: handoff without evidence is REFUSED (real code)
        let r = host.call(&ctx, "ld", "receipt_new", serde_json::json!({"worker_id": "w1", "task": "t1"})).expect("receipt_new");
        let h = host.call(&ctx, "ld", "receipt_handoff", serde_json::json!({"receipt": r.output.clone()})).expect("handoff");
        assert_eq!(h.output["handed_off"], false);
        // with evidence -> handed off
        let r2 = host.call(&ctx, "ld", "receipt_attach", serde_json::json!({"receipt": r.output, "evidence": "proof.txt"})).expect("attach");
        let h2 = host.call(&ctx, "ld", "receipt_handoff", serde_json::json!({"receipt": r2.output})).expect("handoff2");
        assert_eq!(h2.output["handed_off"], true);

        // fleet: heartbeat -> claim roundtrip (real supervisor)
        // Ready (not merely Connected): under WIRE-3 semantics a bare heartbeat
        // proves liveness only and is never admitted to claim work, so the
        // roundtrip must report readiness explicitly. Omitting "readiness"
        // still defaults to Connected and claim stays denied.
        host.call(&ctx, "ld", "fleet_heartbeat", serde_json::json!({"daemon_id": "d1", "role": "worker", "state": "running", "readiness": "ready", "seq": 1})).expect("hb");
        host.call(&ctx, "ld", "fleet_enqueue", serde_json::json!({"id": "job1", "payload": "do thing"})).expect("enq");
        let c = host.call(&ctx, "ld", "fleet_claim", serde_json::json!({"daemon_id": "d1"})).expect("claim");
        assert_eq!(c.output["claimed"]["id"], "job1");

        // correction store against a temp path (real chain: seed -> add -> find)
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.jsonl");
        let ps = p.to_str().unwrap();
        host.call(&ctx, "ld", "correct_add", serde_json::json!({"path": ps, "rule": "test rule", "category": "discipline", "source": "wave-4a"})).expect("add");
        let f = host.call(&ctx, "ld", "correct_find", serde_json::json!({"path": ps, "query": "test rule"})).expect("find");
        assert_eq!(f.output.as_array().unwrap().len(), 1);

        let n = host.verify_chain().expect("receipt chain must verify");
        assert!(n >= 10, "expected >=10 receipts, got {n}");
    }
}
