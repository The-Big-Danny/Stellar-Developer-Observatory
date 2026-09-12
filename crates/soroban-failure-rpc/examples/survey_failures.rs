//! Survey failed Soroban transactions on a network, bucketed by how they failed.
//!
//! ```bash
//! cargo run -p soroban-failure-rpc --example survey_failures -- \
//!     [--rpc <URL>] [--pages <N>] [--page-size <N>]
//! ```
//!
//! RPC has no "failed transactions" filter, so this samples `getTransactions`
//! pages spread evenly across the endpoint's retention window. Every failed
//! Soroban transaction is decoded with the same code the analyzer uses and
//! bucketed by:
//!
//! * the effective (inner) result code and failing operation code, and
//! * the terminal diagnostic error type, e.g. `Storage/ExceededLimit`.
//!
//! Contract error codes are collapsed to `Contract` so buckets group by *kind*
//! of failure. Example hashes are printed per bucket so a transaction can be
//! captured as a fixture with `sdo-probe capture`.
//!
//! This exists to find real examples of failure categories the fixture corpus
//! does not yet contain (issue #1). It makes many network requests and is a
//! research tool, not a test.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{json, Value};
use soroban_failure_analysis::model::{error_label, EventKind};
use soroban_failure_analysis::TransactionModel;
use soroban_failure_rpc::{decode_get_transaction, RpcClient};
use stellar_xdr::ScError;

#[derive(Default)]
struct Bucket {
    count: usize,
    examples: Vec<(String, u64)>,
    message: Option<String>,
}

fn arg(args: &[String], flag: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone())
}

/// One RPC call, retried with backoff on transport failure or rate limiting.
fn call(client: &RpcClient, method: &str, params: Value) -> Option<Value> {
    for attempt in 0..4u32 {
        match client.call(method, params.clone()) {
            Ok(v) => return Some(v),
            Err(e) => {
                eprintln!("  {method} failed (attempt {}): {e}", attempt + 1);
                std::thread::sleep(Duration::from_millis(750 * 2u64.pow(attempt)));
            }
        }
    }
    None
}

fn kind_of(error: &ScError) -> String {
    match error {
        ScError::Contract(_) => "Contract".into(),
        other => error_label(other),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rpc = arg(&args, "--rpc").unwrap_or_else(|| "https://mainnet.sorobanrpc.com".into());
    let pages: u64 = arg(&args, "--pages")
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let page_size: u64 = arg(&args, "--page-size")
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);

    let client = RpcClient::with_timeout(&rpc, Duration::from_secs(60));
    let health = call(&client, "getHealth", json!({})).expect("getHealth failed");
    let oldest = health["oldestLedger"].as_u64().expect("no oldestLedger");
    let latest = health["latestLedger"].as_u64().expect("no latestLedger");
    // Stay clear of both ends: the window slides while we scan.
    let (lo, hi) = (oldest + 500, latest.saturating_sub(20));
    eprintln!("surveying {pages} pages of {page_size} across ledgers {lo}..{hi} on {rpc}");

    let mut buckets: BTreeMap<(String, String), Bucket> = BTreeMap::new();
    let (mut scanned, mut failed, mut soroban_failed, mut undecodable) = (0, 0, 0, 0);

    for p in 0..pages {
        let start = lo + (hi - lo) * p / pages.max(1);
        let Some(result) = call(
            &client,
            "getTransactions",
            json!({ "startLedger": start, "pagination": { "limit": page_size } }),
        ) else {
            continue;
        };
        let txs = result["transactions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        scanned += txs.len();

        for tx in &txs {
            if tx["status"].as_str() != Some("FAILED") {
                continue;
            }
            failed += 1;
            let hash = tx["txHash"].as_str().unwrap_or_default().to_string();
            let decoded = match decode_get_transaction(&hash, tx) {
                Ok(d) => d,
                Err(_) => {
                    undecodable += 1;
                    continue;
                }
            };
            let model = TransactionModel::from_input(&decoded.input);
            if !model.is_soroban() {
                continue;
            }
            soroban_failed += 1;

            let result_key = match &model.outcome.failed_operation {
                Some(op) => format!("{} / {}", model.outcome.result_code, op.code),
                None => model.outcome.result_code.to_string(),
            };
            let terminal = model.diagnostics.terminal_error.as_ref();
            let error_key = match (&model.diagnostics.events.is_empty(), terminal) {
                (true, _) => "(no diagnostic events)".to_string(),
                (false, Some(t)) => kind_of(&t.error),
                (false, None) => "(no host_fn_failed event)".to_string(),
            };
            let message = terminal.and_then(|t| {
                model.diagnostics.events.iter().find_map(|e| match &e.kind {
                    EventKind::Error { error, message } if *error == t.error => message.clone(),
                    _ => None,
                })
            });

            let bucket = buckets.entry((result_key, error_key)).or_default();
            bucket.count += 1;
            if bucket.message.is_none() {
                bucket.message = message;
            }
            if bucket.examples.len() < 3 {
                bucket
                    .examples
                    .push((hash, tx["ledger"].as_u64().unwrap_or_default()));
            }
        }
        eprintln!(
            "  page {:>3}/{pages} ledger {start}: {} txs, {soroban_failed} Soroban failures so far",
            p + 1,
            txs.len()
        );
    }

    println!("\nscanned {scanned} transactions: {failed} failed, {soroban_failed} failed Soroban, {undecodable} undecodable\n");
    let mut rows: Vec<_> = buckets.into_iter().collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.1.count));
    for ((result, error), b) in rows {
        println!("{:>5}  {result}  |  terminal: {error}", b.count);
        if let Some(m) = &b.message {
            println!("       message: \"{m}\"");
        }
        for (h, l) in &b.examples {
            println!("       {h}  ledger {l}");
        }
    }
}
