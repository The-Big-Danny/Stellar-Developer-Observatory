//! Check, against live RPC, the assumptions the M5 evaluation protocol makes.
//!
//! ```bash
//! cargo run -p soroban-failure-rpc --example verify_protocol_assumptions -- \
//!     [--ledger <SEQ>] [--providers <URL,URL,...>] [--samples <N>] [--archive <URL>]
//! ```
//!
//! The protocol (`docs/evaluation/protocol.md`) rests on four things being true
//! of mainnet RPC. Each is stated in the protocol; each is checked here, and
//! the check either passes, fails, or says it could not be run. Nothing is
//! assumed because it is documented elsewhere.
//!
//! 1. **Retention** (§6.1). Every provider used must retain at least 120,000
//!    ledgers, or a round's window can age out before the round is scanned.
//! 2. **Seed ledgers** (§6.2). `getLedgers` reports a `hash` per ledger, and
//!    that hash really is the ledger header hash: the next ledger's header
//!    names it as its `previousLedgerHash`. This is what makes a future
//!    ledger's hash an unpredictable, unchooseable seed.
//! 3. **Independent agreement** (§6.2). Two providers report the same hash for
//!    the same ledger, so the required cross-check can actually be performed.
//! 4. **Whole-ledger reads** (§6.3). `getTransactions` with a `startLedger`,
//!    followed by its cursor, returns every transaction of that ledger, and
//!    two independent reads agree on the set. "Every" is checked against the
//!    ledger itself: the set read must equal the transaction hashes in that
//!    ledger's own `LedgerCloseMeta` (`getLedgers` `metadataXdr`), and a read
//!    in pages of 7, which forces the cursor across page boundaries inside
//!    the ledger, must give the same set as a read in pages of 200. This runs
//!    on `--samples` ledgers spread across the window (default 6).
//! 5. **History archive** (§6.2). The seed ledger's hash, as a Stellar history
//!    archive publishes it in its checkpoint's `ledger-*.xdr.gz`, equals the
//!    hash RPC reports. The archive is written by stellar-core, independently
//!    of any RPC provider, so it can serve as the second source.
//!
//! This is a research tool that makes live network calls. It exits non-zero if
//! any check fails, so it is usable from a script.

use std::collections::BTreeSet;
use std::io::Read;
use std::time::Duration;

use serde_json::{json, Value};
use soroban_failure_rpc::RpcClient;
use stellar_xdr::{LedgerCloseMeta, LedgerHeaderHistoryEntry, Limits, ReadXdr};

const DEFAULT_PROVIDERS: &str = "https://mainnet.sorobanrpc.com,https://rpc.lightsail.network";

/// SDF's first mainnet history archive.
const DEFAULT_ARCHIVE: &str = "https://history.stellar.org/prd/core-live/core_live_001";

/// The protocol's minimum usable retention, in ledgers (§6.1).
const REQUIRED_RETENTION: u64 = 120_000;

struct Checks {
    passed: usize,
    failed: usize,
}

impl Checks {
    fn record(&mut self, ok: bool, what: &str, detail: &str) {
        if ok {
            self.passed += 1;
            println!("  PASS  {what}: {detail}");
        } else {
            self.failed += 1;
            println!("  FAIL  {what}: {detail}");
        }
    }
}

fn arg(args: &[String], flag: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone())
}

fn call(client: &RpcClient, method: &str, params: Value) -> Option<Value> {
    match client.call(method, params) {
        Ok(v) => Some(v),
        Err(e) => {
            println!("  ----  {method} failed: {e}");
            None
        }
    }
}

/// The decoded `LedgerHeaderHistoryEntry` RPC returns as `headerXdr`.
fn header_entry(ledger: &Value) -> Option<LedgerHeaderHistoryEntry> {
    let xdr = ledger.get("headerXdr")?.as_str()?;
    LedgerHeaderHistoryEntry::from_xdr_base64(xdr, Limits::none()).ok()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Read every transaction hash of one ledger, following the cursor.
fn ledger_transaction_hashes(
    client: &RpcClient,
    ledger: u64,
    limit: u32,
) -> Option<BTreeSet<String>> {
    let mut hashes = BTreeSet::new();
    let mut cursor: Option<String> = None;
    loop {
        let params = match &cursor {
            None => json!({ "startLedger": ledger, "pagination": { "limit": limit } }),
            Some(c) => json!({ "pagination": { "cursor": c, "limit": limit } }),
        };
        let result = call(client, "getTransactions", params)?;
        let page = result["transactions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let past_ledger = page
            .iter()
            .any(|t| t["ledger"].as_u64().unwrap_or_default() > ledger);
        for tx in page.iter().filter(|t| t["ledger"].as_u64() == Some(ledger)) {
            hashes.insert(tx["txHash"].as_str().unwrap_or_default().to_string());
        }
        if past_ledger || page.is_empty() {
            return Some(hashes);
        }
        cursor = result["cursor"].as_str().map(str::to_string);
        cursor.as_ref()?;
    }
}

/// The transaction hashes a ledger's own close meta lists.
fn close_meta_hashes(client: &RpcClient, ledger: u64) -> Option<BTreeSet<String>> {
    let result = call(
        client,
        "getLedgers",
        json!({ "startLedger": ledger, "pagination": { "limit": 1 } }),
    )?;
    let entry = result["ledgers"].as_array()?.first()?;
    if entry["sequence"].as_u64() != Some(ledger) {
        return None;
    }
    let meta = LedgerCloseMeta::from_xdr_base64(entry["metadataXdr"].as_str()?, Limits::none())
        .map_err(|e| println!("  ----  metadataXdr did not decode: {e}"))
        .ok()?;
    let hashes = match meta {
        LedgerCloseMeta::V0(m) => m
            .tx_processing
            .iter()
            .map(|t| hex(&t.result.transaction_hash.0))
            .collect(),
        LedgerCloseMeta::V1(m) => m
            .tx_processing
            .iter()
            .map(|t| hex(&t.result.transaction_hash.0))
            .collect(),
        LedgerCloseMeta::V2(m) => m
            .tx_processing
            .iter()
            .map(|t| hex(&t.result.transaction_hash.0))
            .collect(),
    };
    Some(hashes)
}

/// The hash a history archive publishes for `ledger`, from its checkpoint file.
///
/// Checkpoints close every 64 ledgers, on sequences of the form `64k − 1`. The
/// file is a gzipped stream of XDR records, each preceded by a 4-byte record
/// mark whose low 31 bits are the record's length.
fn archive_hash(archive: &str, ledger: u64) -> Result<String, String> {
    let checkpoint = (ledger / 64) * 64 + 63;
    let name = format!("{checkpoint:08x}");
    let url = format!(
        "{archive}/ledger/{}/{}/{}/ledger-{name}.xdr.gz",
        &name[0..2],
        &name[2..4],
        &name[4..6]
    );
    let mut gz = Vec::new();
    ureq::get(&url)
        .call()
        .map_err(|e| format!("{url}: {e}"))?
        .into_body()
        .into_reader()
        .read_to_end(&mut gz)
        .map_err(|e| format!("{url}: {e}"))?;
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(gz.as_slice())
        .read_to_end(&mut raw)
        .map_err(|e| format!("{url}: not gzip: {e}"))?;

    let mut at = 0;
    while at + 4 <= raw.len() {
        let mark = u32::from_be_bytes(raw[at..at + 4].try_into().unwrap());
        let len = (mark & 0x7fff_ffff) as usize;
        let body = raw
            .get(at + 4..at + 4 + len)
            .ok_or_else(|| format!("{url}: truncated record"))?;
        let entry = LedgerHeaderHistoryEntry::from_xdr(body, Limits::none())
            .map_err(|e| format!("{url}: record did not decode: {e}"))?;
        if u64::from(entry.header.ledger_seq) == ledger {
            return Ok(hex(&entry.hash.0));
        }
        at += 4 + len;
    }
    Err(format!(
        "{url}: ledger {ledger} is not in its checkpoint file"
    ))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let providers: Vec<String> = arg(&args, "--providers")
        .unwrap_or_else(|| DEFAULT_PROVIDERS.into())
        .split(',')
        .map(str::to_string)
        .collect();
    let clients: Vec<(String, RpcClient)> = providers
        .iter()
        .map(|url| {
            (
                url.clone(),
                RpcClient::with_timeout(url, Duration::from_secs(60)),
            )
        })
        .collect();

    let mut checks = Checks {
        passed: 0,
        failed: 0,
    };

    // 1. Retention.
    println!("\n1. Retention (protocol §6.1): at least {REQUIRED_RETENTION} ledgers");
    let mut latest_seen = 0u64;
    let mut oldest_seen = u64::MAX;
    for (url, client) in &clients {
        match call(client, "getHealth", json!({})) {
            Some(h) => {
                let oldest = h["oldestLedger"].as_u64().unwrap_or_default();
                let latest = h["latestLedger"].as_u64().unwrap_or_default();
                let window = h["ledgerRetentionWindow"].as_u64().unwrap_or_default();
                latest_seen = latest_seen.max(latest);
                oldest_seen = oldest_seen.min(oldest);
                checks.record(
                    window >= REQUIRED_RETENTION && latest > oldest,
                    url,
                    &format!("retains {window} ledgers, {oldest}..{latest}"),
                );
            }
            None => checks.record(false, url, "getHealth did not answer"),
        }
    }
    if oldest_seen == u64::MAX {
        println!("\nno provider answered; nothing further can be checked");
        std::process::exit(1);
    }

    // A ledger comfortably inside every provider's window.
    let ledger: u64 = arg(&args, "--ledger")
        .and_then(|v| v.parse().ok())
        .unwrap_or((oldest_seen + latest_seen) / 2);

    // 2. Seed ledgers, and 3. agreement between providers.
    println!("\n2. Seed ledger {ledger} (§6.2): `hash` is the ledger header hash");
    let mut hashes: Vec<(String, String)> = Vec::new();
    for (url, client) in &clients {
        let Some(result) = call(
            client,
            "getLedgers",
            json!({ "startLedger": ledger, "pagination": { "limit": 2 } }),
        ) else {
            checks.record(false, url, "getLedgers did not answer");
            continue;
        };
        let empty = Vec::new();
        let ledgers = result["ledgers"].as_array().unwrap_or(&empty);
        let (Some(first), Some(second)) = (ledgers.first(), ledgers.get(1)) else {
            checks.record(false, url, "getLedgers returned fewer than two ledgers");
            continue;
        };
        let reported = first["hash"].as_str().unwrap_or_default().to_string();
        checks.record(
            reported.len() == 64 && reported.chars().all(|c| c.is_ascii_hexdigit()),
            &format!("{url}: hash format"),
            &reported,
        );

        match (header_entry(first), header_entry(second)) {
            (Some(this), Some(next)) => {
                checks.record(
                    hex(&this.hash.0) == reported,
                    &format!("{url}: header entry agrees with the reported hash"),
                    &hex(&this.hash.0),
                );
                // The decisive check: the *next* ledger's signed header names
                // this hash as its predecessor, so the hash is the real ledger
                // header hash and not a value RPC made up.
                checks.record(
                    hex(&next.header.previous_ledger_hash.0) == reported,
                    &format!(
                        "{url}: ledger {} names it as previousLedgerHash",
                        ledger + 1
                    ),
                    &hex(&next.header.previous_ledger_hash.0),
                );
            }
            _ => checks.record(false, &format!("{url}: headerXdr"), "did not decode"),
        }
        hashes.push((url.clone(), reported));
    }

    println!("\n3. Independent agreement (§6.2): providers report the same hash");
    match hashes.split_first() {
        Some(((first_url, first_hash), rest)) if !rest.is_empty() => {
            for (url, hash) in rest {
                checks.record(
                    hash == first_hash,
                    &format!("{url} vs {first_url}"),
                    &format!("{hash} vs {first_hash}"),
                );
            }
        }
        _ => println!("  ----  fewer than two providers answered; cross-check not run"),
    }

    // 4. Whole-ledger reads, on several ledgers spread across the window.
    let samples: u64 = arg(&args, "--samples")
        .and_then(|v| v.parse().ok())
        .unwrap_or(6);
    let (lo, hi) = (oldest_seen + 500, latest_seen.saturating_sub(100));
    let mut targets: Vec<u64> = vec![ledger];
    targets.extend((0..samples.saturating_sub(1)).map(|i| lo + (hi - lo) * i / samples.max(1)));
    println!(
        "\n4. Whole-ledger reads (§6.3): the cursor reaches the end of a ledger, on {} ledgers",
        targets.len()
    );
    for target in &targets {
        let mut sets: Vec<(String, BTreeSet<String>)> = Vec::new();
        for (url, client) in &clients {
            match ledger_transaction_hashes(client, *target, 200) {
                Some(set) => {
                    checks.record(
                        !set.is_empty(),
                        &format!("{url}: read ledger {target} to its end"),
                        &format!("{} transactions", set.len()),
                    );
                    sets.push((url.clone(), set));
                }
                None => checks.record(false, url, "could not read the ledger completely"),
            }
        }
        let Some(((first_url, first_set), rest)) = sets.split_first() else {
            continue;
        };
        for (url, set) in rest {
            checks.record(
                set == first_set,
                &format!("{url} vs {first_url}: same transactions"),
                &format!("{} vs {} transactions", set.len(), first_set.len()),
            );
        }
        match close_meta_hashes(&clients[0].1, *target) {
            Some(meta) => checks.record(
                &meta == first_set,
                &format!("ledger {target}: read set equals the ledger's own close meta"),
                &format!(
                    "{} read, {} in LedgerCloseMeta, {} missing, {} extra",
                    first_set.len(),
                    meta.len(),
                    meta.difference(first_set).count(),
                    first_set.difference(&meta).count()
                ),
            ),
            None => checks.record(false, &format!("ledger {target}"), "close meta unavailable"),
        }
        match ledger_transaction_hashes(&clients[0].1, *target, 7) {
            Some(small) => checks.record(
                &small == first_set,
                &format!("ledger {target}: pages of 7 give the same set as pages of 200"),
                &format!("{} vs {} transactions", small.len(), first_set.len()),
            ),
            None => checks.record(false, &format!("ledger {target}"), "paged read failed"),
        }
    }

    // 5. History archive.
    let archive = arg(&args, "--archive").unwrap_or_else(|| DEFAULT_ARCHIVE.into());
    println!("\n5. History archive (§6.2): {archive} publishes the same hash for ledger {ledger}");
    match (archive_hash(&archive, ledger), hashes.first()) {
        (Ok(published), Some((url, reported))) => checks.record(
            &published == reported,
            &format!("archive vs {url}"),
            &format!("{published} vs {reported}"),
        ),
        (Err(e), _) => checks.record(false, "archive", &e),
        (_, None) => println!("  ----  no provider reported a hash; archive check not run"),
    }

    println!(
        "\n{} checks passed, {} failed",
        checks.passed, checks.failed
    );
    if checks.failed > 0 {
        std::process::exit(1);
    }
}
