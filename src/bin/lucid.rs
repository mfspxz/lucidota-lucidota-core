//! `lucid` — the LUCIDOTA CORE command line.
//!
//! - `lucid correct add --rule ... --category ... --source ...` — compile a correction forever
//! - `lucid correct list [--category ...]` — show the standing orders
//! - `lucid correct find <query>` — search the correction memory
//! - `lucid lint <file>` — scan text for banned hedge phrases
//! - `lucid gate --claim ... --evidence ptr [--verified]...` — run the Claim Evidence Gate

use clap::{Args, Parser, Subcommand};
use lucidota_core::correction::CorrectionStore;
use lucidota_core::gate::{evaluate, Claim, EvidenceRef, Verdict};
use lucidota_core::lint::lint_text;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "lucid", about = "LUCIDOTA CORE — correction memory, claim gate, hedge lint")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    Correct(CorrectArgs),
    Lint { file: PathBuf },
    Gate(GateArgs),
}

#[derive(Args)]
struct CorrectArgs {
    #[command(subcommand)]
    op: CorrectOp,
}

#[derive(Subcommand)]
enum CorrectOp {
    /// Compile a new correction into standing memory (append-only).
    Add { #[arg(long)] rule: String, #[arg(long)] category: String, #[arg(long)] source: String },
    /// List standing corrections, oldest first.
    List { #[arg(long)] category: Option<String> },
    /// Search rule text, category, and source.
    Find { query: String },
}

#[derive(Args)]
struct GateArgs {
    #[arg(long)]
    claim: String,
    /// Evidence pointers. Prefix with `!` to mark contradicting, `?` for unverified.
    /// Plain pointers count as verified.
    #[arg(long)]
    evidence: Vec<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lucid: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.cmd {
        Cmd::Correct(a) => match a.op {
            CorrectOp::Add { rule, category, source } => {
                let mut s = CorrectionStore::open(&CorrectionStore::default_path()).map_err(|e| e.to_string())?;
                let c = s.add(&rule, &category, &source).map_err(|e| e.to_string())?;
                println!("compiled correction #{} [{}]: {}", c.id, c.category, c.rule);
                Ok(())
            }
            CorrectOp::List { category } => {
                let s = CorrectionStore::open(&CorrectionStore::default_path()).map_err(|e| e.to_string())?;
                for c in s.list() {
                    if let Some(cat) = &category {
                        if &c.category != cat {
                            continue;
                        }
                    }
                    println!("#{} [{}] {} ({})", c.id, c.category, c.rule, c.source);
                }
                Ok(())
            }
            CorrectOp::Find { query } => {
                let s = CorrectionStore::open(&CorrectionStore::default_path()).map_err(|e| e.to_string())?;
                let hits = s.find(&query);
                for c in &hits {
                    println!("#{} [{}] {} ({})", c.id, c.category, c.rule, c.source);
                }
                if hits.is_empty() {
                    println!("no corrections match '{}'", query);
                }
                Ok(())
            }
        },
        Cmd::Lint { file } => {
            let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
            let hits = lint_text(&text);
            for h in &hits {
                println!("line {}: banned phrase '{}' — {}", h.line, h.phrase, h.replacement);
                println!("    {}", h.context);
            }
            println!("{} banned phrase(s) found", hits.len());
            if hits.is_empty() { Ok(()) } else { Err("hedge phrases present — rewrite in verified/uncertain vocabulary".into()) }
        }
        Cmd::Gate(a) => {
            let GateArgs { claim, evidence } = a;
            let mut c = Claim::new(&claim);
            for e in &evidence {
                let r = if let Some(p) = e.strip_prefix('!') {
                    EvidenceRef::contradicting(p)
                } else if let Some(p) = e.strip_prefix('?') {
                    EvidenceRef::unverified(p)
                } else {
                    EvidenceRef::verified(e)
                };
                c = c.with_evidence(r);
            }
            let v = evaluate(&c);
            let verdict = match v.verdict {
                Verdict::Proven => "PROVEN",
                Verdict::Disproven => "DISPROVEN",
                Verdict::Unknown => "UNKNOWN",
            };
            println!("verdict: {}", verdict);
            if v.claim_laundering_suspect {
                println!("flag: CLAIM LAUNDERING SUSPECT — no evidence cited");
            }
            for n in &v.notes {
                println!("note: {}", n);
            }
            match v.verdict {
                Verdict::Proven => Ok(()),
                _ => Err("claim not proven".into()),
            }
        }
    }
}
