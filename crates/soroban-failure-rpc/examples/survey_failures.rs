//! Survey failed Soroban transactions on a network, bucketed by how they failed.
//!
//! ```bash
//! # Bucket survey (issue #1): sample pages spread across the retention window.
//! cargo run -p soroban-failure-rpc --example survey_failures -- \
//!     [--rpc <URL>] [--pages <N>] [--page-size <N>]
//!
//! # Population census (issue #19): read whole ledgers and write one JSONL
//! # record per failed Soroban transaction.
//! cargo run -p soroban-failure-rpc --example survey_failures -- \
//!     --ledgers <N> --jsonl <FILE> [--rpc <URL>] [--fallback <URL>] \
//!     [--seed <TEXT>] [--window <LO>..<HI>] [--ledger-log <FILE>]
//! ```
//!
//! RPC has no "failed transactions" filter, so this samples the retention
//! window. Every failed Soroban transaction is decoded with the same code the
//! analyzer uses and bucketed by:
//!
//! * the effective (inner) result code and failing operation code, and
//! * the terminal diagnostic error type, e.g. `Storage/ExceededLimit`.
//!
//! Contract error codes are collapsed to `Contract` so buckets group by *kind*
//! of failure. Example hashes are printed per bucket so a transaction can be
//! captured as a fixture with `sdo-probe capture`.
//!
//! # Modes
//!
//! **Page mode** (the default) samples `--pages` pages of `--page-size`
//! transactions spread evenly across the window. It finds examples quickly, but
//! a page starts at a ledger boundary and stops after one page, so it
//! under-samples busy ledgers and the transactions applied late in them.
//!
//! **Ledger mode** (`--ledgers <N>`) reads **every** transaction of `N` sampled
//! ledgers, following the `getTransactions` cursor to the end of each ledger.
//! This is the sampling shape the M5 evaluation protocol uses (§6.3), so the
//! counts it produces describe the population the protocol would actually draw
//! from. Ledgers are chosen by stratified sampling: one ledger from each of `N`
//! equal strata across the window, at a deterministic pseudorandom offset
//! within its stratum. The generator is a SplitMix64 seeded from `--seed`, and
//! is deliberately *not* the protocol's SHA-256 selection: the pilot measures a
//! population, it does not select evaluation samples.
//!
//! With `--jsonl <FILE>`, ledger mode writes one [`PilotRecord`] per failed
//! Soroban transaction: its hashes, its cluster keys, the raw facts the
//! protocol's eligibility checks read (operation count, diagnostic event count,
//! how the instance lookup went) and its raw terminal error type and code, and
//! **nothing SDO concluded** — no cause class, verdict, confidence or rule id.
//! Failed transactions that are not Soroban are not written; their count is
//! reported.
//!
//! # Long runs
//!
//! A census takes hours, and public endpoints drop connections. Three options
//! make a run reproducible and resumable rather than something to start over:
//!
//! * `--window <LO>..<HI>` fixes the ledger range instead of deriving it from
//!   `getHealth` at start-up, so a restarted run chooses the same ledgers.
//! * `--ledger-log <FILE>` appends one JSON line per ledger attempt: its
//!   sequence, `read` or `unreadable`, the transactions read and the final
//!   cursor. On restart, ledgers already `read` are skipped and the JSONL is
//!   appended to; ledgers recorded `unreadable` are attempted again, and both
//!   attempts stay in the log.
//! * `--fallback <URL>` is tried once after four failed attempts on `--rpc`
//!   (waiting 1, 2 and 4 seconds between them), the retry procedure of protocol
//!   §7.1. Every failed attempt is printed.
//!
//! A ledger is only ever used if it was read completely.
//!
//! This exists to find real examples of failure categories the fixture corpus
//! does not yet contain (issue #1) and to measure the population available to
//! the evaluation (issue #19). It makes many network requests and is a research
//! tool, not a test.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, Write};
use std::time::Duration;

use serde_json::{json, Value};
use soroban_failure_analysis::model::{error_label, EventKind};
use soroban_failure_analysis::TransactionModel;
use soroban_failure_rpc::cluster::{InstanceLookup, PilotRecord};
use soroban_failure_rpc::contract::{decode_instance, instance_key};
use soroban_failure_rpc::{decode_get_transaction, RpcClient, SourceError};
use stellar_xdr::{ContractId, Limits, ScError, WriteXdr};

#[derive(Default)]
struct Bucket {
    count: usize,
    examples: Vec<(String, u64)>,
    message: Option<String>,
}

/// Everything the census counts, so nothing is dropped silently.
#[derive(Default)]
struct Census {
    ledgers_sampled: usize,
    ledgers_unreadable: Vec<u64>,
    transactions_read: usize,
    failed: usize,
    undecodable: usize,
    not_soroban: usize,
    soroban_failed: usize,
    unexpected_operation_count: usize,
    no_diagnostic_events: usize,
    no_root_contract_invocation: usize,
    instances_not_found: usize,
    instance_lookup_failures: usize,
}

fn arg(args: &[String], flag: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone())
}

fn number(args: &[String], flag: &str) -> Option<u64> {
    arg(args, flag).and_then(|v| v.parse().ok())
}

/// The collection provider, and the fallback tried when it gives up.
struct Rpc {
    primary: RpcClient,
    fallback: Option<RpcClient>,
}

impl Rpc {
    /// One RPC call under the protocol's retry procedure (§7.1): four attempts
    /// on the primary, waiting 1, 2 then 4 seconds between them, then one on
    /// the fallback. Every failure is printed; none is hidden.
    fn call(&self, method: &str, params: Value) -> Option<Value> {
        for attempt in 0..4u32 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_secs(1 << (attempt - 1)));
            }
            match self.primary.call(method, params.clone()) {
                Ok(v) => return Some(v),
                Err(e) => eprintln!("  {method} failed (attempt {}): {e}", attempt + 1),
            }
        }
        let fallback = self.fallback.as_ref()?;
        match fallback.call(method, params) {
            Ok(v) => {
                eprintln!("  {method} answered by the fallback provider");
                Some(v)
            }
            Err(e) => {
                eprintln!("  {method} failed on the fallback provider: {e}");
                None
            }
        }
    }
}

fn kind_of(error: &ScError) -> String {
    match error {
        ScError::Contract(_) => "Contract".into(),
        other => error_label(other),
    }
}

/// SplitMix64: a small, well-documented generator, so ledger choice is
/// reproducible from `--seed` alone without adding a dependency.
struct SplitMix64(u64);

impl SplitMix64 {
    fn seeded(text: &str) -> Self {
        // FNV-1a over the seed text, so any string gives a 64-bit state.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in text.as_bytes() {
            h ^= u64::from(*byte);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        Self(h)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

/// One ledger from each of `count` equal strata across `lo..=hi`.
fn stratified_ledgers(lo: u64, hi: u64, count: u64, seed: &str) -> Vec<u64> {
    let mut rng = SplitMix64::seeded(seed);
    let span = hi.saturating_sub(lo).max(1);
    let stratum = (span / count.max(1)).max(1);
    (0..count)
        .map(|i| {
            let base = lo + i * stratum;
            (base + rng.next() % stratum).min(hi)
        })
        .collect()
}

/// Every transaction of one ledger, and the cursor the read ended on.
struct LedgerRead {
    transactions: Vec<Value>,
    final_cursor: Option<String>,
}

/// Read every transaction of one ledger, following the cursor to its end.
///
/// Returns `None` if any page of the ledger could not be read: a partially read
/// ledger is never used, because the transactions it is missing are not missing
/// at random.
fn whole_ledger(client: &Rpc, ledger: u64) -> Option<LedgerRead> {
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let params = match &cursor {
            None => json!({ "startLedger": ledger, "pagination": { "limit": 200 } }),
            Some(c) => json!({ "pagination": { "cursor": c, "limit": 200 } }),
        };
        let result = client.call("getTransactions", params)?;
        let page = result["transactions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let final_cursor = result["cursor"].as_str().map(str::to_string);
        let reached_next_ledger = page
            .iter()
            .any(|t| t["ledger"].as_u64().unwrap_or_default() > ledger);
        out.extend(
            page.iter()
                .filter(|t| t["ledger"].as_u64() == Some(ledger))
                .cloned(),
        );
        if reached_next_ledger || page.is_empty() {
            return Some(LedgerRead {
                transactions: out,
                final_cursor,
            });
        }
        cursor = final_cursor;
        // No cursor and no transaction from a later ledger: the window ends
        // here, and we cannot prove the ledger was read completely.
        cursor.as_ref()?;
    }
}

/// Look up the root contract's instance, caching answers per contract.
///
/// Anything but `Found` is recorded by the protocol as
/// `code_identity_instance_unavailable` — never as a contract-ID fallback.
/// `NotFound` is a fact about the ledger and is cached; `Failed` is a fact
/// about this run, so it is not cached and the next transaction asks again.
fn instance(
    client: &Rpc,
    cache: &mut BTreeMap<String, InstanceLookup>,
    census: &mut Census,
    contract: &str,
) -> InstanceLookup {
    if let Some(cached) = cache.get(contract) {
        return cached.clone();
    }
    let lookup = match contract.parse::<ContractId>() {
        Err(_) => InstanceLookup::Failed,
        Ok(id) => {
            let key = instance_key(&id)
                .to_xdr_base64(Limits::none())
                .expect("a ledger key always encodes");
            match client.call("getLedgerEntries", json!({ "keys": [key] })) {
                None => InstanceLookup::Failed,
                Some(entry) => match decode_instance(&entry) {
                    Ok(executable) => InstanceLookup::Found(executable),
                    Err(SourceError::NotFound(_)) => InstanceLookup::NotFound,
                    Err(_) => InstanceLookup::Failed,
                },
            }
        }
    };
    match lookup {
        InstanceLookup::Failed => census.instance_lookup_failures += 1,
        InstanceLookup::NotFound => {
            census.instances_not_found += 1;
            cache.insert(contract.to_string(), lookup.clone());
        }
        InstanceLookup::Found(_) => {
            cache.insert(contract.to_string(), lookup.clone());
        }
    }
    lookup
}

/// Parse `LO..HI`.
fn window(text: &str) -> Option<(u64, u64)> {
    let (lo, hi) = text.split_once("..")?;
    let (lo, hi) = (lo.parse().ok()?, hi.parse().ok()?);
    (lo < hi).then_some((lo, hi))
}

/// Ledgers a previous run of the same census already read completely.
fn already_read(log: &str) -> BTreeSet<u64> {
    let Ok(file) = std::fs::File::open(log) else {
        return BTreeSet::new();
    };
    std::io::BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
        .filter(|v| v["status"] == "read")
        .filter_map(|v| v["ledger"].as_u64())
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rpc = arg(&args, "--rpc").unwrap_or_else(|| "https://mainnet.sorobanrpc.com".into());
    let pages = number(&args, "--pages").unwrap_or(60);
    let page_size = number(&args, "--page-size").unwrap_or(200);
    let ledgers = number(&args, "--ledgers");
    let seed = arg(&args, "--seed").unwrap_or_else(|| "m5.2-pilot".into());
    let jsonl = arg(&args, "--jsonl");
    let ledger_log = arg(&args, "--ledger-log");

    let client = Rpc {
        primary: RpcClient::with_timeout(&rpc, Duration::from_secs(60)),
        fallback: arg(&args, "--fallback")
            .map(|url| RpcClient::with_timeout(&url, Duration::from_secs(60))),
    };
    let health = client
        .call("getHealth", json!({}))
        .expect("getHealth failed");
    let oldest = health["oldestLedger"].as_u64().expect("no oldestLedger");
    let latest = health["latestLedger"].as_u64().expect("no latestLedger");
    eprintln!("provider {rpc}: ledgers {oldest}..{latest}");
    let (lo, hi) = match arg(&args, "--window") {
        Some(text) => {
            let (lo, hi) = window(&text).expect("--window must be LO..HI with LO < HI");
            assert!(
                lo >= oldest && hi <= latest,
                "--window {lo}..{hi} is not inside the provider's {oldest}..{latest}"
            );
            (lo, hi)
        }
        // Stay clear of both ends: the window slides while we scan.
        None => (oldest + 500, latest.saturating_sub(20)),
    };

    let mut buckets: BTreeMap<(String, String), Bucket> = BTreeMap::new();
    let mut census = Census::default();

    match ledgers {
        Some(count) => ledger_mode(
            &client,
            lo,
            hi,
            count,
            &seed,
            jsonl.as_deref(),
            ledger_log.as_deref(),
            &mut buckets,
            &mut census,
        ),
        None => page_mode(&client, lo, hi, pages, page_size, &mut buckets, &mut census),
    }

    report(&rpc, lo, hi, &census, buckets);
}

#[allow(clippy::too_many_arguments)]
fn ledger_mode(
    client: &Rpc,
    lo: u64,
    hi: u64,
    count: u64,
    seed: &str,
    jsonl: Option<&str>,
    ledger_log: Option<&str>,
    buckets: &mut BTreeMap<(String, String), Bucket>,
    census: &mut Census,
) {
    let chosen = stratified_ledgers(lo, hi, count, seed);
    let unique: BTreeSet<u64> = chosen.iter().copied().collect();
    let done = ledger_log.map(already_read).unwrap_or_default();
    eprintln!(
        "census: {} ledgers ({} distinct) across {lo}..{hi}, seed {seed:?}; {} already read",
        chosen.len(),
        unique.len(),
        done.intersection(&unique).count()
    );

    // Resuming appends; a fresh census (no log, or an empty one) starts clean.
    let append = !done.is_empty();
    let open = |path: &str| {
        std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(path)
            .map(std::io::BufWriter::new)
            .unwrap_or_else(|e| panic!("could not open {path}: {e}"))
    };
    let mut out = jsonl.map(open);
    let mut log = ledger_log.map(|path| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap_or_else(|e| panic!("could not open {path}: {e}"))
    });
    let mut instances: BTreeMap<String, InstanceLookup> = BTreeMap::new();

    for (n, ledger) in unique.iter().enumerate() {
        if done.contains(ledger) {
            continue;
        }
        let Some(read) = whole_ledger(client, *ledger) else {
            eprintln!("  ledger {ledger}: UNREADABLE, skipped whole");
            census.ledgers_unreadable.push(*ledger);
            if let Some(log) = log.as_mut() {
                let line = json!({ "ledger": ledger, "status": "unreadable" });
                writeln!(log, "{line}").expect("could not write the ledger log");
            }
            continue;
        };
        let transactions = read.transactions;
        census.ledgers_sampled += 1;
        census.transactions_read += transactions.len();

        let mut in_ledger = 0;
        for tx in &transactions {
            let Some(model_and_hash) = failed_soroban(tx, census, buckets) else {
                continue;
            };
            let (hash, model) = model_and_hash;
            in_ledger += 1;

            let lookup = match soroban_failure_rpc::cluster::root_contract_id(&model) {
                Some(root) => Some(instance(client, &mut instances, census, &root)),
                None => {
                    census.no_root_contract_invocation += 1;
                    None
                }
            };
            if let Some(out) = out.as_mut() {
                let record = PilotRecord::new(&hash, *ledger as u32, &model, lookup.as_ref());
                writeln!(out, "{}", serde_json::to_string(&record).unwrap())
                    .expect("could not write JSONL");
            }
        }
        // The JSONL is flushed before the ledger is logged as read, so a
        // resumed run never skips a ledger whose records were lost.
        if let Some(out) = out.as_mut() {
            out.flush().expect("could not flush JSONL");
        }
        if let Some(log) = log.as_mut() {
            let line = json!({
                "ledger": ledger,
                "status": "read",
                "transactions": transactions.len(),
                "failed_soroban": in_ledger,
                "final_cursor": read.final_cursor,
            });
            writeln!(log, "{line}").expect("could not write the ledger log");
        }
        eprintln!(
            "  [{}/{}] ledger {ledger}: {} txs, {in_ledger} failed Soroban ({} so far)",
            n + 1,
            unique.len(),
            transactions.len(),
            census.soroban_failed
        );
    }
    if let Some(out) = out.as_mut() {
        out.flush().expect("could not flush JSONL");
    }
}

fn page_mode(
    client: &Rpc,
    lo: u64,
    hi: u64,
    pages: u64,
    page_size: u64,
    buckets: &mut BTreeMap<(String, String), Bucket>,
    census: &mut Census,
) {
    eprintln!("surveying {pages} pages of {page_size} across ledgers {lo}..{hi}");
    for p in 0..pages {
        let start = lo + (hi - lo) * p / pages.max(1);
        let Some(result) = client.call(
            "getTransactions",
            json!({ "startLedger": start, "pagination": { "limit": page_size } }),
        ) else {
            continue;
        };
        let txs = result["transactions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        census.transactions_read += txs.len();
        for tx in &txs {
            let _ = failed_soroban(tx, census, buckets);
        }
        eprintln!(
            "  page {:>3}/{pages} ledger {start}: {} txs, {} Soroban failures so far",
            p + 1,
            txs.len(),
            census.soroban_failed
        );
    }
}

/// Decode a transaction, count it, and bucket it if it is a Soroban failure.
fn failed_soroban(
    tx: &Value,
    census: &mut Census,
    buckets: &mut BTreeMap<(String, String), Bucket>,
) -> Option<(String, TransactionModel)> {
    if tx["status"].as_str() != Some("FAILED") {
        return None;
    }
    census.failed += 1;
    let hash = tx["txHash"].as_str().unwrap_or_default().to_string();
    let Ok(decoded) = decode_get_transaction(&hash, tx) else {
        census.undecodable += 1;
        return None;
    };
    let model = TransactionModel::from_input(&decoded.input);
    if !model.is_soroban() {
        census.not_soroban += 1;
        return None;
    }
    census.soroban_failed += 1;
    if model.operations.len() != 1 {
        census.unexpected_operation_count += 1;
    }
    if model.diagnostics.events.is_empty() {
        census.no_diagnostic_events += 1;
    }

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
            .push((hash.clone(), tx["ledger"].as_u64().unwrap_or_default()));
    }
    Some((hash, model))
}

fn report(
    rpc: &str,
    lo: u64,
    hi: u64,
    census: &Census,
    buckets: BTreeMap<(String, String), Bucket>,
) {
    println!("\nendpoint {rpc}, ledgers {lo}..{hi}");
    if census.ledgers_sampled > 0 {
        println!(
            "ledgers: {} read completely, {} unreadable {:?}",
            census.ledgers_sampled,
            census.ledgers_unreadable.len(),
            census.ledgers_unreadable
        );
    }
    println!(
        "scanned {} transactions: {} failed, {} failed Soroban, {} undecodable, {} not Soroban",
        census.transactions_read,
        census.failed,
        census.soroban_failed,
        census.undecodable,
        census.not_soroban
    );
    println!(
        "of the failed Soroban: {} not a single operation, {} without diagnostic events, \
         {} not an InvokeContract call, {} instances not found, {} instance lookups failed",
        census.unexpected_operation_count,
        census.no_diagnostic_events,
        census.no_root_contract_invocation,
        census.instances_not_found,
        census.instance_lookup_failures
    );
    println!();

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
