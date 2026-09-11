//! `sdo` — the Stellar Developer Observatory command line interface.
//!
//! # Status
//!
//! **Milestone M1 (foundation).** `sdo explain` fetches and decodes a
//! transaction and reports what it found, but it **cannot yet tell you why the
//! transaction failed** — no failure rules are implemented (milestone M4) and
//! failure-stage classification is not implemented (milestone M2).
//!
//! The command prints exactly what it knows and exactly what it does not. That
//! is the point: a diagnostic tool that overstates its confidence is worse than
//! no tool. See `ROADMAP.md`.

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use soroban_failure_analysis::analyze;
use soroban_failure_rpc::{fixture, RpcClient, TransactionStatus};

/// Default public Stellar mainnet RPC endpoint.
const DEFAULT_RPC: &str = "https://mainnet.sorobanrpc.com";

#[derive(Parser)]
#[command(
    name = "sdo",
    version,
    about = "Explain why a Soroban transaction failed",
    long_about = "Stellar Developer Observatory.\n\n\
                  EARLY DEVELOPMENT: this build can decode and structure a failed \
                  transaction, but cannot yet attribute a cause. See ROADMAP.md."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Analyse a failed transaction.
    Explain {
        /// Hex transaction hash. Omit when using `--fixture`.
        tx: Option<String>,

        /// RPC endpoint URL.
        #[arg(long, default_value = DEFAULT_RPC)]
        rpc: String,

        /// Analyse a recorded fixture directory instead of querying the network.
        #[arg(long, conflicts_with = "tx")]
        fixture: Option<std::path::PathBuf>,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Command::Explain { tx, rpc, fixture } => {
            let decoded = match (&fixture, &tx) {
                (Some(dir), _) => fixture::load(dir)?,
                (None, Some(hash)) => RpcClient::new(&rpc).fetch_transaction(hash)?,
                (None, None) => return Err("provide a transaction hash, or --fixture <dir>".into()),
            };

            if decoded.status == TransactionStatus::Success {
                println!("Transaction succeeded. This tool analyses failures.");
                return Ok(());
            }

            let diagnosis = analyze(&decoded.input);

            println!("Transaction");
            println!(
                "  {}",
                diagnosis.transaction_hash.as_deref().unwrap_or("<unknown>")
            );
            println!();
            println!("Status");
            println!("  FAILED");
            println!();
            println!("Failure stage");
            match diagnosis.stage {
                Some(stage) => println!("  {} — {}", stage, stage.description()),
                None => println!("  none — the transaction succeeded"),
            }
            println!();

            println!("Evidence available");
            println!(
                "  diagnostic events: {}",
                match decoded.diagnostic_source {
                    Some(src) =>
                        format!("{} (from {:?})", decoded.input.diagnostic_events.len(), src),
                    None => "none returned".to_string(),
                }
            );
            println!(
                "  transaction metadata: {}",
                if decoded.input.meta.is_some() {
                    "present"
                } else {
                    "absent"
                }
            );
            println!();

            if diagnosis.is_undetermined() {
                println!("Candidate causes");
                println!("  none — this build cannot yet attribute a cause");
            } else {
                println!("Candidate causes");
                for (i, c) in diagnosis.candidate_causes.iter().enumerate() {
                    println!(
                        "  {}. [{}] {} ({})",
                        i + 1,
                        c.confidence.id(),
                        c.summary,
                        c.class
                    );
                    for ev in &c.evidence {
                        println!("       evidence: {}", ev.observation);
                    }
                    if let Some(r) = &c.remediation {
                        println!("       next step: {r}");
                    }
                }
            }
            println!();

            println!("Limitations");
            for l in &diagnosis.limitations {
                println!("  - {l}");
            }

            Ok(())
        }
    }
}
