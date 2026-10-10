//! Correction memory — the persistent record of what the operator taught the system.
//!
//! Every correction is compiled once and enforced forever. Entries are append-only
//! and hash-chained: `hash = sha256(prev_hash || id || ts_utc || rule || category || source)`.
//! Genesis `prev_hash` is the literal `"GENESIS"`.
//!
//! Demand #4 from the chat analysis: an AI that learns from his corrections and
//! retains what he teaches it. This is the gap the indy-reads build has; this
//! module closes it.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// Seed corrections: the operator's verified standing orders, compiled from the
/// 2026-02-18 → 2026-05-01 chat corpus. Written into a fresh store on first open.
pub const SEED_JSONL: &str = include_str!("../corrections-seed.jsonl");

/// A single compiled correction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Correction {
    /// Monotonic sequence number within the store.
    pub id: u64,
    /// UTC timestamp, ISO-8601 to the second.
    pub ts_utc: String,
    /// The standing rule, in the operator's own words where possible.
    pub rule: String,
    /// One of: identity, autonomy, banned-phrase, epistemics, discipline, naming.
    pub category: String,
    /// Where the correction came from (chat name / date / document).
    pub source: String,
    /// Hash of the previous entry (`"GENESIS"` for id 0).
    pub prev_hash: String,
    /// `sha256(prev_hash || id || ts_utc || rule || category || source)`, hex.
    pub hash: String,
}

impl Correction {
    fn compute_hash(prev_hash: &str, id: u64, ts_utc: &str, rule: &str, category: &str, source: &str) -> String {
        let mut h = Sha256::new();
        h.update(prev_hash.as_bytes());
        h.update(id.to_be_bytes());
        h.update(ts_utc.as_bytes());
        h.update(rule.as_bytes());
        h.update(category.as_bytes());
        h.update(source.as_bytes());
        hex(&h.finalize())
    }

    fn new(id: u64, ts: DateTime<Utc>, rule: &str, category: &str, source: &str, prev_hash: &str) -> Self {
        let ts_utc = ts.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let hash = Self::compute_hash(prev_hash, id, &ts_utc, rule, category, source);
        Self { id, ts_utc, rule: rule.into(), category: category.into(), source: source.into(), prev_hash: prev_hash.into(), hash }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Append-only, hash-chained store of corrections, persisted as JSONL.
pub struct CorrectionStore {
    path: PathBuf,
    entries: Vec<Correction>,
}

impl CorrectionStore {
    /// Default store location: `$LUCIDOTA_CORRECTIONS`, else
    /// `~/.local/share/lucidota/corrections.jsonl`.
    pub fn default_path() -> PathBuf {
        if let Ok(p) = std::env::var("LUCIDOTA_CORRECTIONS") {
            return PathBuf::from(p);
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        Path::new(&home).join(".local/share/lucidota/corrections.jsonl")
    }

    /// Open (or create + seed) the store at `path`. Verifies the hash chain on load;
    /// returns `Err` on a broken chain.
    pub fn open(path: &Path) -> io::Result<Self> {
        if !path.exists() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, SEED_JSONL)?;
        }
        let mut store = Self { path: path.to_path_buf(), entries: Vec::new() };
        store.load()?;
        if !store.verify_chain() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "correction chain broken — tampering or corruption"));
        }
        Ok(store)
    }

    fn load(&mut self) -> io::Result<()> {
        let f = fs::File::open(&self.path)?;
        for line in BufReader::new(f).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let c: Correction = serde_json::from_str(&line)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
            self.entries.push(c);
        }
        Ok(())
    }

    /// Verify every link in the hash chain. Pure; does not touch disk.
    pub fn verify_chain(&self) -> bool {
        let mut prev = "GENESIS".to_string();
        for c in &self.entries {
            if c.prev_hash != prev {
                return false;
            }
            let want = Correction::compute_hash(&c.prev_hash, c.id, &c.ts_utc, &c.rule, &c.category, &c.source);
            if want != c.hash {
                return false;
            }
            prev = c.hash.clone();
        }
        true
    }

    /// Append a new correction. The rule is written to disk immediately (append-only).
    pub fn add(&mut self, rule: &str, category: &str, source: &str) -> io::Result<&Correction> {
        self.add_at(Utc::now(), rule, category, source)
    }

    fn add_at(&mut self, ts: DateTime<Utc>, rule: &str, category: &str, source: &str) -> io::Result<&Correction> {
        let id = self.entries.len() as u64;
        let prev_hash = self.entries.last().map(|c| c.hash.clone()).unwrap_or_else(|| "GENESIS".into());
        let c = Correction::new(id, ts, rule, category, source, &prev_hash);
        let mut f = OpenOptions::new().create(true).append(true).open(&self.path)?;
        writeln!(f, "{}", serde_json::to_string(&c).unwrap())?;
        self.entries.push(c);
        Ok(self.entries.last().unwrap())
    }

    /// All corrections, oldest first.
    pub fn list(&self) -> &[Correction] {
        &self.entries
    }

    /// Case-insensitive substring search over rule text, category, and source.
    pub fn find(&self, query: &str) -> Vec<&Correction> {
        let q = query.to_lowercase();
        self.entries
            .iter()
            .filter(|c| {
                c.rule.to_lowercase().contains(&q)
                    || c.category.to_lowercase().contains(&q)
                    || c.source.to_lowercase().contains(&q)
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn seeded() -> (tempfile::TempDir, CorrectionStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = CorrectionStore::open(&dir.path().join("c.jsonl")).unwrap();
        (dir, store)
    }

    // Borrow the store out of the seeded pair; the TempDir guard must stay alive.
    fn store(pair: &(tempfile::TempDir, CorrectionStore)) -> &CorrectionStore {
        &pair.1
    }

    #[test]
    fn seed_loads_ten_verified_corrections() {
        let pair = seeded();
        let s = store(&pair);
        assert_eq!(s.len(), 10);
        assert!(s.verify_chain());
    }

    #[test]
    fn seed_ids_are_monotonic_from_zero() {
        let pair = seeded();
        let s = store(&pair);
        for (i, c) in s.list().iter().enumerate() {
            assert_eq!(c.id, i as u64);
        }
    }

    #[test]
    fn add_appends_and_rechains() {
        let (_dir, mut s) = seeded();
        let ts = Utc.with_ymd_and_hms(2026, 10, 8, 12, 0, 0).unwrap();
        let prev_last_hash = s.list().last().unwrap().hash.clone();
        let c = s.add_at(ts, "test rule", "discipline", "unit test").unwrap();
        assert_eq!(c.id, 10);
        assert_eq!(c.prev_hash, prev_last_hash);
        assert_eq!(s.len(), 11);
        assert!(s.verify_chain());
    }

    #[test]
    fn add_persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.jsonl");
        {
            let mut s = CorrectionStore::open(&p).unwrap();
            s.add("persisted rule", "epistemics", "unit test").unwrap();
        }
        let s2 = CorrectionStore::open(&p).unwrap();
        assert_eq!(s2.len(), 11);
        assert!(s2.verify_chain());
        assert!(s2.find("persisted rule").len() == 1);
    }

    #[test]
    fn tampered_entry_breaks_chain() {
        let (_dir, mut s) = seeded();
        s.entries[3].rule = "rewritten history".into();
        assert!(!s.verify_chain());
    }

    #[test]
    fn tampered_prev_link_breaks_chain() {
        let (_dir, mut s) = seeded();
        s.entries[5].prev_hash = "forged".into();
        assert!(!s.verify_chain());
    }

    #[test]
    fn find_is_case_insensitive_across_fields() {
        let pair = seeded();
        let s = store(&pair);
        assert!(!s.find("PORGES").is_empty());
        assert!(!s.find("claim laundering").is_empty());
        assert!(!s.find("sophia").is_empty());
        assert!(s.find("zzz-no-such-rule").is_empty());
    }

    #[test]
    fn genesis_prev_hash_is_literal() {
        let pair = seeded();
        let s = store(&pair);
        assert_eq!(s.list()[0].prev_hash, "GENESIS");
    }

    #[test]
    fn corrupted_file_refuses_to_open() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.jsonl");
        fs::write(&p, "not json at all\n").unwrap();
        assert!(CorrectionStore::open(&p).is_err());
    }
}
