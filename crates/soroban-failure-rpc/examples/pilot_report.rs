//! Summarise an M5.2 population census, offline, from its recorded files.
//!
//! ```bash
//! cargo run -p soroban-failure-rpc --example pilot_report -- \
//!     --jsonl <FILE> --ledger-log <FILE> [--split <LEDGER>] \
//!     [--prior-jsonl <FILE>] [--trials <N>] [--only <LO>..<HI>]
//!
//! # Print the exclusion list: every hash in the given census files, sorted.
//! cargo run -p soroban-failure-rpc --example pilot_report -- \
//!     --exclusions <FILE> [<FILE> ...]
//! ```
//!
//! Reads the JSONL and ledger log that `survey_failures --ledgers` wrote and
//! prints, deterministically:
//!
//! * the eligibility funnel of protocol §4.2 for the failed Soroban
//!   transactions (checks 3, 4, 6 and 7; checks 1, 2 and 5 are counted by the
//!   census itself and by the research note);
//! * distinct `code_cluster`s and `submitter_cluster`s, and the share of the
//!   ten largest of each;
//! * unresolved code identities by reason, separating a lookup that *failed*
//!   (an artefact of the run) from an instance that was *not found* (a fact
//!   about the ledger);
//! * the **yield**: how many samples the protocol's selection (§6.4: one per
//!   `code_cluster`, three per `submitter_cluster`) accepts from `K` sampled
//!   ledgers, for several `K`, over `--trials` random subsets of the census
//!   ledgers;
//! * an incidence-based estimate of how many eligible code clusters the
//!   whole window holds, including those no sampled ledger contained: the
//!   Chao2 estimator and its extrapolation to more sampled ledgers (Chao et al.
//!   2014, *Ecological Monographs* 84(1)), with ledgers as sampling units. The
//!   extrapolation is only reliable to about twice the ledgers read, and Chao2
//!   is a lower bound, not a point estimate;
//! * with `--split`, a two-round projection: ledgers before the split as
//!   round 1 and from it as round 2, with caps cumulative across rounds;
//! * with `--prior-jsonl`, how many of an earlier, disjoint window's clusters
//!   recur in this one.
//!
//! Nothing here reads a cause class, verdict, confidence or rule id; the
//! records do not contain one. Selection order uses the transaction hash mixed
//! with a trial number, not the protocol's seeded SHA-256 order, because this
//! projects a yield and selects nothing.
//!
//! `--only <LO>..<HI>` restricts every figure to ledgers in that range. A
//! census stopped before it finished has covered only the start of its window;
//! restricting both it and another census to the covered range compares them
//! on equal ground, at whatever densities they sampled it.
//!
//! Any input may be gzipped (`.gz`). No network access: this runs on
//! committed files only.

use std::collections::{BTreeMap, BTreeSet};
use std::io::BufRead;

use serde_json::Value;
use soroban_failure_rpc::cluster::{PilotRecord, UnresolvedCode};

fn arg(args: &[String], flag: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone())
}

/// The non-empty lines of a file, gunzipped first if its name ends in `.gz`.
fn lines(path: &str) -> Vec<String> {
    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let reader: Box<dyn std::io::Read> = if path.ends_with(".gz") {
        Box::new(flate2::read::GzDecoder::new(file))
    } else {
        Box::new(file)
    };
    std::io::BufReader::new(reader)
        .lines()
        .map(|l| l.unwrap_or_else(|e| panic!("{path}: {e}")))
        .filter(|l| !l.trim().is_empty())
        .collect()
}

fn records(path: &str) -> Vec<PilotRecord> {
    lines(path)
        .iter()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{path}: {e}: {l}")))
        .collect()
}

/// The protocol's eligibility checks that a pilot record can answer.
fn exclusion(r: &PilotRecord) -> Option<&'static str> {
    if r.operation_count != 1 {
        return Some("unexpected_operation_count");
    }
    if r.diagnostic_event_count == 0 {
        return Some("no_diagnostic_events");
    }
    if r.root_contract_id.is_none() {
        return Some("no_root_contract_invocation");
    }
    r.code_unresolved_reason.map(UnresolvedCode::reason)
}

/// SplitMix64, for reproducible subsets and orders without a dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

fn order_key(hash: &str, trial: u64) -> u64 {
    let prefix = u64::from_str_radix(&hash[..16], 16).unwrap_or_default();
    let mut rng = Rng(prefix ^ trial.wrapping_mul(0xa076_1d64_78bd_642f));
    rng.next()
}

/// Caps carried across rounds.
#[derive(Default)]
struct Caps {
    codes: BTreeSet<String>,
    submitters: BTreeMap<String, usize>,
}

impl Caps {
    /// Walk candidates in order under protocol §6.4; return how many were accepted.
    fn select(&mut self, candidates: &[&PilotRecord], trial: u64) -> usize {
        let mut ordered: Vec<&&PilotRecord> = candidates.iter().collect();
        ordered.sort_by_key(|r| (order_key(&r.transaction_hash, trial), &r.transaction_hash));
        let mut accepted = 0;
        for r in ordered {
            let code = r.code_cluster.as_ref().expect("eligible");
            if self.codes.contains(code) {
                continue;
            }
            let used = self
                .submitters
                .entry(r.submitter_cluster.clone())
                .or_default();
            if *used >= 3 {
                continue;
            }
            *used += 1;
            self.codes.insert(code.clone());
            accepted += 1;
        }
        accepted
    }
}

fn share(counts: &BTreeMap<String, usize>) -> (usize, f64, Vec<(String, usize)>) {
    let total: usize = counts.values().sum();
    let mut sorted: Vec<(String, usize)> = counts.iter().map(|(k, v)| (k.clone(), *v)).collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let top: usize = sorted.iter().take(10).map(|(_, v)| v).sum();
    let ratio = if total == 0 {
        0.0
    } else {
        top as f64 / total as f64
    };
    (counts.len(), ratio, sorted.into_iter().take(10).collect())
}

fn percentile(sorted: &[usize], p: f64) -> usize {
    if sorted.is_empty() {
        return 0;
    }
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i]
}

/// Every transaction hash, outer and inner, in the given census files.
///
/// Read loosely, so a census that predates `inner_transaction_hash` still
/// contributes its outer hashes.
fn exclusions(paths: &[String]) {
    let mut hashes = BTreeSet::new();
    for path in paths {
        for line in lines(path) {
            let v: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("{path}: {e}"));
            for field in ["transaction_hash", "inner_transaction_hash"] {
                if let Some(h) = v[field].as_str() {
                    assert!(
                        h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()),
                        "{path}: {field} is not a hash: {h}"
                    );
                    hashes.insert(h.to_ascii_lowercase());
                }
            }
        }
    }
    for h in hashes {
        println!("{h}");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--exclusions") {
        exclusions(&args[1..]);
        return;
    }
    let jsonl = arg(&args, "--jsonl").expect("--jsonl is required");
    let log = arg(&args, "--ledger-log").expect("--ledger-log is required");
    let trials: u64 = arg(&args, "--trials")
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);

    let recs = records(&jsonl);
    let mut ledgers: BTreeMap<u64, usize> = BTreeMap::new();
    let mut unreadable: BTreeSet<u64> = BTreeSet::new();
    for line in lines(&log) {
        let v: Value = serde_json::from_str(&line).expect("ledger log line");
        let seq = v["ledger"].as_u64().expect("ledger");
        match v["status"].as_str() {
            Some("read") => {
                ledgers.insert(seq, v["transactions"].as_u64().unwrap_or_default() as usize);
                unreadable.remove(&seq);
            }
            _ => {
                if !ledgers.contains_key(&seq) {
                    unreadable.insert(seq);
                }
            }
        }
    }
    let only = arg(&args, "--only").map(|text| {
        let (lo, hi) = text.split_once("..").expect("--only must be LO..HI");
        (
            lo.parse::<u64>().expect("a ledger"),
            hi.parse::<u64>().expect("a ledger"),
        )
    });
    if let Some((lo, hi)) = only {
        ledgers.retain(|seq, _| (lo..=hi).contains(seq));
        unreadable.retain(|seq| (lo..=hi).contains(seq));
        println!("restricted to ledgers {lo}..={hi}\n");
    }
    let read: BTreeSet<u64> = ledgers.keys().copied().collect();
    // Only ledgers read completely count, and each transaction once: a census
    // interrupted between writing a ledger's records and logging it re-reads
    // that ledger on resume.
    let mut hashes = BTreeSet::new();
    let recs: Vec<PilotRecord> = recs
        .into_iter()
        .filter(|r| read.contains(&u64::from(r.ledger)))
        .filter(|r| hashes.insert(r.transaction_hash.clone()))
        .collect();

    println!("# M5.2 census summary\n");
    println!(
        "ledgers read completely: {} ({}..{}); still unreadable: {}",
        read.len(),
        read.first().copied().unwrap_or_default(),
        read.last().copied().unwrap_or_default(),
        unreadable.len()
    );
    println!("transactions read: {}", ledgers.values().sum::<usize>());
    println!("failed Soroban transactions recorded: {}\n", recs.len());

    // Funnel.
    let mut funnel: BTreeMap<&str, usize> = BTreeMap::new();
    let mut eligible: Vec<&PilotRecord> = Vec::new();
    for r in &recs {
        match exclusion(r) {
            Some(reason) => *funnel.entry(reason).or_default() += 1,
            None => eligible.push(r),
        }
    }
    println!("## Funnel (failed Soroban, checks in protocol order)\n");
    for reason in [
        "unexpected_operation_count",
        "no_diagnostic_events",
        "no_root_contract_invocation",
        "code_identity_instance_unavailable",
        "code_identity_not_in_footprint",
        "code_identity_unsupported_executable",
    ] {
        println!("  {reason}: {}", funnel.get(reason).copied().unwrap_or(0));
    }
    println!(
        "  eligible (before check 5, development data): {}",
        eligible.len()
    );

    let unavailable = recs.iter().filter(|r| {
        r.code_unresolved_reason == Some(UnresolvedCode::InstanceUnavailable)
            && r.operation_count == 1
            && r.diagnostic_event_count > 0
    });
    let (mut failed, mut not_found) = (0, 0);
    for r in unavailable {
        match r.instance_lookup.as_deref() {
            Some("failed") => failed += 1,
            _ => not_found += 1,
        }
    }
    println!(
        "  of instance_unavailable: {not_found} not found (population), {failed} lookup failed (run artefact)\n"
    );

    // Clusters.
    let mut codes: BTreeMap<String, usize> = BTreeMap::new();
    let mut submitters: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_code_submitters: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for r in &eligible {
        let code = r.code_cluster.clone().unwrap();
        *codes.entry(code.clone()).or_default() += 1;
        *submitters.entry(r.submitter_cluster.clone()).or_default() += 1;
        per_code_submitters
            .entry(code)
            .or_default()
            .insert(r.submitter_cluster.clone());
    }
    for (name, counts) in [("code_cluster", &codes), ("submitter_cluster", &submitters)] {
        let (distinct, top10, top) = share(counts);
        println!("## {name}\n");
        println!(
            "  distinct: {distinct}; top 10 hold {:.1}% of eligible",
            top10 * 100.0
        );
        for (key, n) in top {
            println!("    {n:>6}  {key}");
        }
        let singletons = counts.values().filter(|n| **n == 1).count();
        println!("  seen exactly once: {singletons}\n");
    }
    let fee_bumped = eligible.iter().filter(|r| r.fee_bumped).count();
    println!(
        "eligible fee-bumped: {fee_bumped} of {} ({:.1}%)\n",
        eligible.len(),
        100.0 * fee_bumped as f64 / eligible.len().max(1) as f64
    );

    // Terminal errors among eligible, raw only.
    let mut errors: BTreeMap<String, usize> = BTreeMap::new();
    for r in &eligible {
        *errors
            .entry(r.terminal_error.clone().unwrap_or_else(|| "(none)".into()))
            .or_default() += 1;
    }
    println!("## Raw terminal error of eligible transactions\n");
    let mut errs: Vec<_> = errors.into_iter().collect();
    errs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (e, n) in errs {
        println!("    {n:>6}  {e}");
    }
    println!();

    // Yield by K.
    let by_ledger: BTreeMap<u64, Vec<&PilotRecord>> =
        eligible.iter().fold(BTreeMap::new(), |mut m, r| {
            m.entry(u64::from(r.ledger)).or_default().push(*r);
            m
        });
    let all: Vec<u64> = read.iter().copied().collect();
    println!("## Projected single-round yield by K ({trials} random subsets each)\n");
    println!(
        "  {:>6}  {:>6}  {:>6}  {:>6}  {:>6}",
        "K", "mean", "p5", "p50", "p95"
    );
    for k in [50usize, 100, 200, 300, 500, 750, 1000, 1500, 2000] {
        if k > all.len() {
            continue;
        }
        let mut yields = Vec::new();
        for t in 0..trials {
            let mut rng = Rng(0x5eed ^ (k as u64) << 20 ^ t);
            let mut pool = all.clone();
            // Fisher-Yates, first k.
            for i in 0..k {
                let j = i + (rng.next() % (pool.len() - i) as u64) as usize;
                pool.swap(i, j);
            }
            let candidates: Vec<&PilotRecord> = pool[..k]
                .iter()
                .flat_map(|l| by_ledger.get(l).into_iter().flatten().copied())
                .collect();
            yields.push(Caps::default().select(&candidates, t));
        }
        yields.sort_unstable();
        let mean = yields.iter().sum::<usize>() as f64 / yields.len() as f64;
        println!(
            "  {k:>6}  {mean:>6.1}  {:>6}  {:>6}  {:>6}",
            percentile(&yields, 0.05),
            percentile(&yields, 0.5),
            percentile(&yields, 0.95)
        );
    }
    println!();

    // Accumulation of distinct code clusters as ledgers are added, in ledger order.
    println!("## Distinct eligible code clusters vs ledgers read (ledger order)\n");
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let step = (all.len() / 10).max(1);
    for (i, l) in all.iter().enumerate() {
        for r in by_ledger.get(l).into_iter().flatten() {
            seen.insert(r.code_cluster.as_deref().unwrap());
        }
        if (i + 1) % step == 0 || i + 1 == all.len() {
            println!("  after {:>5} ledgers: {:>4} distinct", i + 1, seen.len());
        }
    }
    println!();

    // Richness: how many clusters the window holds that were not seen.
    let m = all.len() as f64;
    let mut incidence: BTreeMap<&str, usize> = BTreeMap::new();
    for records in by_ledger.values() {
        let here: BTreeSet<&str> = records
            .iter()
            .map(|r| r.code_cluster.as_deref().unwrap())
            .collect();
        for code in here {
            *incidence.entry(code).or_default() += 1;
        }
    }
    let s_obs = incidence.len() as f64;
    let q1 = incidence.values().filter(|n| **n == 1).count() as f64;
    let q2 = incidence.values().filter(|n| **n == 2).count() as f64;
    let q0 = if q2 > 0.0 {
        (m - 1.0) / m * q1 * q1 / (2.0 * q2)
    } else {
        (m - 1.0) / m * q1 * (q1 - 1.0).max(0.0) / 2.0
    };
    println!(
        "## Eligible code clusters not yet seen (incidence, ledgers as units)
"
    );
    println!(
        "  observed {s_obs}; in exactly one sampled ledger (Q1) {q1}, in exactly two (Q2) {q2}"
    );
    println!("  Chao2 lower bound for the window: {:.1}", s_obs + q0);
    for factor in [1.5f64, 2.0, 3.0] {
        let t = m * (factor - 1.0);
        let extra = if q0 > 0.0 {
            q0 * (1.0 - (1.0 - q1 / (m * q0 + q1)).powf(t))
        } else {
            0.0
        };
        println!(
            "  expected distinct at {:.0} ledgers: {:.1}",
            m * factor,
            s_obs + extra
        );
    }
    println!();

    // Two-round projection.
    if let Some(split) = arg(&args, "--split").and_then(|v| v.parse::<u64>().ok()) {
        let r1: Vec<&PilotRecord> = eligible
            .iter()
            .filter(|r| u64::from(r.ledger) < split)
            .copied()
            .collect();
        let r2: Vec<&PilotRecord> = eligible
            .iter()
            .filter(|r| u64::from(r.ledger) >= split)
            .copied()
            .collect();
        let l1 = all.iter().filter(|l| **l < split).count();
        let l2 = all.len() - l1;
        let mut totals = (Vec::new(), Vec::new());
        for t in 0..trials {
            let mut caps = Caps::default();
            totals.0.push(caps.select(&r1, t));
            totals.1.push(caps.select(&r2, t));
        }
        let mean = |v: &[usize]| v.iter().sum::<usize>() as f64 / v.len() as f64;
        println!("## Two-round projection, split at ledger {split}, caps cumulative\n");
        println!(
            "  round 1: {l1} ledgers, {} eligible, mean accepted {:.1}",
            r1.len(),
            mean(&totals.0)
        );
        println!(
            "  round 2: {l2} ledgers, {} eligible, mean newly accepted {:.1}",
            r2.len(),
            mean(&totals.1)
        );
        let c1: BTreeSet<&str> = r1
            .iter()
            .map(|r| r.code_cluster.as_deref().unwrap())
            .collect();
        let c2: BTreeSet<&str> = r2
            .iter()
            .map(|r| r.code_cluster.as_deref().unwrap())
            .collect();
        println!(
            "  code clusters: {} in round 1, {} in round 2, {} in both, {} new in round 2\n",
            c1.len(),
            c2.len(),
            c1.intersection(&c2).count(),
            c2.difference(&c1).count()
        );
    }

    // Recurrence from a prior window.
    if let Some(prior) = arg(&args, "--prior-jsonl") {
        // Read loosely: an earlier census may predate fields added since. Only
        // its resolved code clusters are compared.
        let prior_codes: BTreeSet<String> = lines(&prior)
            .iter()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter_map(|v| v["code_cluster"].as_str().map(str::to_string))
            .collect();
        let now: BTreeSet<String> = codes.keys().cloned().collect();
        println!("## Recurrence against a prior window\n");
        println!(
            "  prior eligible code clusters: {}; here: {}; in both: {}; new here: {}\n",
            prior_codes.len(),
            now.len(),
            prior_codes.intersection(&now).count(),
            now.difference(&prior_codes).count()
        );
    }

    // Code clusters with the submitters that could supply them.
    let multi = per_code_submitters.values().filter(|s| s.len() > 1).count();
    println!(
        "code clusters seen from more than one submitter: {multi} of {}",
        per_code_submitters.len()
    );
}
