//! `sdo-probe` — the M0 feasibility instrument.
//!
//! This tool exists to answer one question with evidence rather than assumption:
//!
//! > For a real failed Soroban transaction, does a given Stellar RPC endpoint
//! > return enough information to attribute a cause?
//!
//! It is a research tool, not part of the product. It is excluded from
//! publication (`publish = false`) and it is the only crate in the workspace
//! permitted to talk to the network during normal use.

mod probe;
mod render;
mod result_codes;
mod rpc;
mod scan;

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Serialize)]
struct FixtureMetadata {
    transaction_hash: String,
    network: String,
    ledger: u64,
    captured_at: String,
    rpc_provider: String,
    failure_category: String,
    fee_bumped: bool,
    diagnostic_event_count: usize,
    purpose: String,
}

use crate::probe::probe_transaction;
use crate::rpc::RpcClient;

/// Default public Stellar mainnet RPC endpoint.
const DEFAULT_RPC: &str = "https://mainnet.sorobanrpc.com";

#[derive(Parser)]
#[command(
    name = "sdo-probe",
    version,
    about = "Measure what a Stellar RPC endpoint exposes for failed Soroban transactions"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Probe one transaction on one endpoint.
    Probe {
        /// RPC endpoint URL.
        #[arg(long, default_value = DEFAULT_RPC)]
        rpc: String,
        /// Hex transaction hash.
        #[arg(long)]
        tx: String,
        /// Emit machine-readable JSON instead of a human report.
        #[arg(long)]
        json: bool,
        /// Per-request timeout in seconds.
        #[arg(long, default_value_t = 30)]
        timeout: u64,
    },

    /// Walk recent ledgers to find failed Soroban transactions.
    Scan {
        /// RPC endpoint URL.
        #[arg(long, default_value = DEFAULT_RPC)]
        rpc: String,
        /// Ledger to start from. Defaults to the endpoint's oldest retained ledger.
        #[arg(long)]
        from_ledger: Option<u32>,
        /// How many failed Soroban transactions to collect.
        #[arg(long, default_value_t = 5)]
        want: usize,
        /// Maximum number of pages to walk before giving up.
        #[arg(long, default_value_t = 40)]
        max_pages: usize,
        /// Transactions per page.
        #[arg(long, default_value_t = 200)]
        page_size: u32,
        /// Emit JSON.
        #[arg(long)]
        json: bool,
        /// Per-request timeout in seconds.
        #[arg(long, default_value_t = 60)]
        timeout: u64,
    },

    /// Fetch a transaction and write it to disk as an offline test fixture.
    Capture {
        /// RPC endpoint URL.
        #[arg(long, default_value = DEFAULT_RPC)]
        rpc: String,
        /// Hex transaction hash.
        #[arg(long)]
        tx: String,
        /// Directory to write the fixture into.
        #[arg(long)]
        out: PathBuf,

        /// Human-readable explanation of why this fixture is useful.
        #[arg(long)]
        purpose: String,

        /// Human-readable failure category for this fixture.
        #[arg(long)]
        failure_category: String,

        /// Per-request timeout in seconds.
        #[arg(long, default_value_t = 30)]
        timeout: u64,
    },
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::from(2)
        }
    }
}

fn run() -> Result<std::process::ExitCode, Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Command::Probe {
            rpc,
            tx,
            json,
            timeout,
        } => {
            let client = RpcClient::new(&rpc, Duration::from_secs(timeout));
            let passphrase = network_passphrase(&client);
            let result = client.get_transaction(&tx)?;
            let report = probe_transaction(&rpc, passphrase, &tx, &result);

            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", render::render(&report));
            }

            // Exit code carries the verdict so this is usable from scripts and CI.
            Ok(match report.verdict {
                probe::Verdict::SufficientForAnalysis => std::process::ExitCode::SUCCESS,
                _ => std::process::ExitCode::from(1),
            })
        }

        Command::Scan {
            rpc,
            from_ledger,
            want,
            max_pages,
            page_size,
            json,
            timeout,
        } => {
            let client = RpcClient::new(&rpc, Duration::from_secs(timeout));

            let start = match from_ledger {
                Some(l) => l,
                None => {
                    let health = client.get_health()?;
                    let oldest = health
                        .get("oldestLedger")
                        .and_then(Value::as_u64)
                        .ok_or("endpoint did not report oldestLedger")?;
                    // Start a little after the oldest retained ledger so the
                    // window does not slide out from under the scan.
                    (oldest as u32).saturating_add(100)
                }
            };

            if !json {
                eprintln!("scanning from ledger {start} for {want} failed Soroban transactions...");
            }

            let found = scan::scan_for_failed_soroban(
                &client,
                start,
                want,
                max_pages,
                page_size,
                |page, ledger, hits| {
                    if !json {
                        eprintln!("  page {page}: through ledger {ledger}, {hits} match(es)");
                    }
                },
            )?;

            if json {
                let out: Vec<Value> = found
                    .iter()
                    .map(
                        |f| json!({ "hash": f.hash, "ledger": f.ledger, "operation": f.operation }),
                    )
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                println!("\nFound {} failed Soroban transaction(s):\n", found.len());
                for f in &found {
                    println!("  {}  ledger {}  {}", f.hash, f.ledger, f.operation);
                }
            }

            Ok(if found.is_empty() {
                std::process::ExitCode::from(1)
            } else {
                std::process::ExitCode::SUCCESS
            })
        }

        Command::Capture {
            rpc,
            tx,
            out,
            purpose,
            failure_category,
            timeout,
        } => {
            let client = RpcClient::new(&rpc, Duration::from_secs(timeout));
            let result = client.get_transaction(&tx)?;

            std::fs::create_dir_all(&out)?;
            let path = out.join("rpc-response.json");
            let mut f = std::fs::File::create(&path)?;
            // Store the RPC `result` object verbatim. Fixtures must be faithful
            // recordings; reshaping them here would make later analysis a test of
            // this tool rather than of real network data.
            writeln!(f, "{}", serde_json::to_string_pretty(&result)?)?;

            let passphrase = network_passphrase(&client);
            let report = probe_transaction(&rpc, passphrase.clone(), &tx, &result);
            let meta_path = out.join("probe.json");
            std::fs::write(&meta_path, serde_json::to_string_pretty(&report)?)?;

            let metadata = FixtureMetadata {
                transaction_hash: tx.clone(),
                network: passphrase.ok_or("RPC response did not provide a network passphrase")?,
                ledger: report
                    .ledger
                    .ok_or("probe report did not provide a ledger")?,
                captured_at: chrono::Utc::now().to_rfc3339(),
                rpc_provider: rpc.clone(),
                failure_category,
                fee_bumped: report
                    .failure_signal
                    .as_ref()
                    .map(|signal| signal.fee_bumped)
                    .unwrap_or(false),
                diagnostic_event_count: report.diagnostic_events.count,
                purpose,
            };

            let metadata_path = out.join("metadata.json");
            std::fs::write(&metadata_path, serde_json::to_string_pretty(&metadata)?)?;
            println!("wrote {}", path.display());
            println!("wrote {}", meta_path.display());
            println!("wrote {}", metadata_path.display());
            Ok(std::process::ExitCode::SUCCESS)
        }
    }
}

/// Best-effort network identification. Not fatal if the endpoint declines.
fn network_passphrase(client: &RpcClient) -> Option<String> {
    client
        .get_network()
        .ok()?
        .get("passphrase")
        .and_then(Value::as_str)
        .map(str::to_string)
}
