//! The M5.2 pilot's exclusion list must name every transaction the pilot saw.
//!
//! `evaluation/exclusions/pilot-v1.txt` is what stops a transaction the pilot
//! observed from later becoming an evaluation sample (protocol §13.1). It is
//! generated from the committed census files, and this test regenerates it
//! independently — parsing the JSONL here rather than calling the generator —
//! so the list cannot drift from the data it claims to cover.
//!
//! Offline: it reads only committed files.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// Every hash, outer and inner, recorded in one gzipped census file.
fn hashes_in(path: &Path) -> BTreeSet<String> {
    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let reader: Box<dyn Read> = if path.extension().is_some_and(|e| e == "gz") {
        Box::new(flate2::read::GzDecoder::new(file))
    } else {
        Box::new(file)
    };
    let mut hashes = BTreeSet::new();
    for line in BufReader::new(reader).lines() {
        let line = line.unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        if line.trim().is_empty() {
            continue;
        }
        let record: serde_json::Value = serde_json::from_str(&line)
            .unwrap_or_else(|e| panic!("{}: {e}: {line}", path.display()));
        for field in ["transaction_hash", "inner_transaction_hash"] {
            if let Some(hash) = record[field].as_str() {
                hashes.insert(hash.to_ascii_lowercase());
            }
        }
    }
    hashes
}

/// The census files under `evaluation/pilot/`, whatever they are called.
fn census_files() -> Vec<PathBuf> {
    let dir = repo_root().join("evaluation").join("pilot");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|p| p.to_string_lossy().ends_with(".jsonl.gz"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no census files in {}", dir.display());
    files
}

fn committed_list() -> (BTreeSet<String>, usize) {
    let path = repo_root()
        .join("evaluation")
        .join("exclusions")
        .join("pilot-v1.txt");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let entries: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    let set: BTreeSet<String> = entries.iter().map(|l| l.to_string()).collect();
    assert_eq!(set.len(), entries.len(), "the list repeats a hash");
    (set, entries.len())
}

#[test]
fn the_exclusion_list_is_exactly_the_hashes_the_census_recorded() {
    let mut expected = BTreeSet::new();
    for file in census_files() {
        expected.extend(hashes_in(&file));
    }
    let (listed, _) = committed_list();

    let missing: Vec<&String> = expected.difference(&listed).take(5).collect();
    assert!(
        missing.is_empty(),
        "{} observed transactions are not excluded, e.g. {missing:?}",
        expected.difference(&listed).count()
    );
    let extra: Vec<&String> = listed.difference(&expected).take(5).collect();
    assert!(
        extra.is_empty(),
        "{} excluded hashes are in no census file, e.g. {extra:?}",
        listed.difference(&expected).count()
    );
}

#[test]
fn every_excluded_line_is_a_lowercase_hash() {
    let (listed, count) = committed_list();
    assert!(count > 0, "the exclusion list is empty");
    for hash in &listed {
        assert!(
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "not a lowercase 64-character hash: {hash}"
        );
    }
}

#[test]
fn no_development_fixture_was_observed_by_the_pilot() {
    // A fixture transaction in an evaluation dataset would be development data
    // leaking into the measurement (protocol §13.1). The pilot's windows are
    // newer than every fixture, so this must hold -- and if a future census
    // ever covers an older window, this test is where that shows up.
    let dir = repo_root().join("fixtures").join("failed");
    let mut fixture_hashes = BTreeSet::new();
    for entry in std::fs::read_dir(&dir).expect("fixtures/failed") {
        let metadata = entry
            .expect("a directory entry")
            .path()
            .join("metadata.json");
        if !metadata.exists() {
            continue;
        }
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&metadata).expect("metadata.json"))
                .expect("metadata.json is JSON");
        if let Some(hash) = value["transaction_hash"].as_str() {
            fixture_hashes.insert(hash.to_ascii_lowercase());
        }
    }
    assert!(!fixture_hashes.is_empty(), "no fixture hashes were read");

    let (listed, _) = committed_list();
    let overlap: Vec<&String> = fixture_hashes.intersection(&listed).collect();
    assert!(
        overlap.is_empty(),
        "the pilot observed development fixtures: {overlap:?}"
    );
}
