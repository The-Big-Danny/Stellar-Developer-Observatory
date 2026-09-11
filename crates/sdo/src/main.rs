//! `sdo` — the Stellar Developer Observatory command line interface.
//!
//! # Status
//!
//! **Milestones M2–M3.** `sdo explain` shows where a transaction failed, the
//! contract call trace, declared versus observed resources, and the names of
//! contract-defined errors resolved from the contracts' own specs.
//!
//! It **cannot yet say why** a transaction failed: no failure rules exist
//! (milestone M4). It prints what it observed and states plainly what it does
//! not know. See `ROADMAP.md`.

mod render;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use soroban_failure_analysis::contract::contracts_needing_specs;
use soroban_failure_analysis::{analyze, TransactionModel};
use soroban_failure_rpc::{fetch_specs, fixture, FixtureContractSource, RpcClient};

/// Default public Stellar mainnet RPC endpoint.
const DEFAULT_RPC: &str = "https://mainnet.sorobanrpc.com";

#[derive(Parser)]
#[command(
    name = "sdo",
    version,
    about = "Explain why a Soroban transaction failed",
    long_about = "Stellar Developer Observatory.\n\n\
                  EARLY DEVELOPMENT: this build shows where a transaction failed and \
                  names contract errors, but cannot yet attribute a cause. See ROADMAP.md."
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
        fixture: Option<PathBuf>,

        /// Directory of recorded contract fixtures to resolve contract error
        /// names from when using `--fixture` (e.g. `fixtures/contracts`).
        /// Without it, fixture mode makes no network requests and reports
        /// contract error names as unavailable.
        #[arg(long, requires = "fixture")]
        contracts: Option<PathBuf>,
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
    let Command::Explain {
        tx,
        rpc,
        fixture,
        contracts,
    } = Cli::parse().command;

    let (decoded, client) = match (&fixture, &tx) {
        (Some(dir), _) => (fixture::load(dir)?, None),
        (None, Some(hash)) => {
            let client = RpcClient::new(&rpc);
            (client.fetch_transaction(hash)?, Some(client))
        }
        (None, None) => return Err("provide a transaction hash, or --fixture <dir>".into()),
    };

    let model = TransactionModel::from_input(&decoded.input);

    // Fetch only the specs this transaction's contract errors need.
    let needed = contracts_needing_specs(&model);
    let specs = match (&client, &contracts) {
        (Some(client), _) => fetch_specs(client, &needed),
        (None, Some(dir)) => fetch_specs(&FixtureContractSource::new(dir), &needed),
        (None, None) => Default::default(),
    };

    let diagnosis = analyze(&decoded.input.with_contract_specs(specs));
    print!("{}", render::report(&model, &diagnosis));
    Ok(())
}
