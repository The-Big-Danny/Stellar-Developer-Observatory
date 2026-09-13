//! Integration tests over the committed fixture corpus.
//!
//! **These tests must never touch the network.** Everything they need is in
//! `fixtures/`. If a change to this project makes a test here require an RPC
//! endpoint, that change is wrong.
//!
//! They serve two purposes:
//!
//! 1. Prove the decode path works against real recorded mainnet data.
//! 2. Guard the fixture corpus itself — structure, metadata, and the rule that
//!    no fixture may ever contain a secret.

use std::path::{Path, PathBuf};

use soroban_failure_analysis::{analyze, FailureStage};
use soroban_failure_rpc::{fixture, TransactionStatus};

fn fixtures_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is crates/sdo; the corpus lives at the workspace root.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
}

/// Every fixture directory under `fixtures/failed/`.
fn failed_fixtures() -> Vec<PathBuf> {
    let dir = fixtures_root().join("failed");
    let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    out.sort();
    out
}

#[test]
fn the_corpus_is_not_empty() {
    // A silently empty corpus would make every test below vacuously pass.
    assert!(
        !failed_fixtures().is_empty(),
        "no fixtures found under fixtures/failed/"
    );
}

#[test]
fn every_fixture_has_the_required_files() {
    for dir in failed_fixtures() {
        for required in [
            "rpc-response.json",
            "probe.json",
            "README.md",
            "metadata.json",
        ] {
            assert!(
                dir.join(required).is_file(),
                "{} is missing {required}",
                dir.display()
            );
        }
    }
}

#[test]
fn every_fixture_decodes() {
    for dir in failed_fixtures() {
        let decoded = fixture::load(&dir)
            .unwrap_or_else(|e| panic!("fixture {} failed to decode: {e}", dir.display()));
        assert_eq!(
            decoded.status,
            TransactionStatus::Failed,
            "{} should record a failed transaction",
            dir.display()
        );
    }
}

#[test]
fn every_fixture_can_be_analysed_without_panicking() {
    // The engine has no rules yet, so this asserts the pipeline is sound rather
    // than that any particular cause is found.
    for dir in failed_fixtures() {
        let decoded = fixture::load(&dir).unwrap();
        let diagnosis = analyze(&decoded.input);

        assert!(
            matches!(diagnosis.stage, Some(s) if s != FailureStage::Unknown),
            "M2 classifies every fixture's stage"
        );
        assert!(
            diagnosis
                .candidate_causes
                .iter()
                .all(|c| !c.evidence.is_empty()),
            "{}: M4 must never produce a cause without evidence",
            dir.display()
        );
        assert!(
            !diagnosis.is_undetermined() || !diagnosis.limitations.is_empty(),
            "{} is unexplained yet produced no limitations; an unexplained failure must say why",
            dir.display()
        );
    }
}

#[test]
fn analysis_of_a_fixture_is_reproducible() {
    for dir in failed_fixtures() {
        let a = analyze(&fixture::load(&dir).unwrap().input);
        let b = analyze(&fixture::load(&dir).unwrap().input);
        assert_eq!(a, b, "{} did not analyse deterministically", dir.display());
    }
}

#[test]
fn fixture_hash_matches_the_recorded_response() {
    for dir in failed_fixtures() {
        let raw = fixture::load_raw(&dir).unwrap();
        let recorded = raw
            .get("txHash")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("{} has no txHash", dir.display()));

        let probe: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("probe.json")).unwrap()).unwrap();
        let probed = probe.get("transaction").and_then(|v| v.as_str()).unwrap();

        assert_eq!(
            recorded,
            probed,
            "{}: probe.json and rpc-response.json disagree about the transaction",
            dir.display()
        );
    }
}

/// A fixture must never carry a Stellar secret seed.
///
/// `getTransaction` returns only public keys and signatures, so a match here
/// means something was pasted in by hand and must not be committed.
///
/// The check is deliberately strict — case-sensitive, anchored on a word
/// boundary, and exactly 56 characters — because a loose pattern matches
/// coincidental substrings inside base64 XDR and trains people to ignore it.
#[test]
fn no_fixture_contains_a_secret() {
    fn is_strkey_seed(candidate: &str) -> bool {
        candidate.len() == 56
            && candidate.starts_with('S')
            && candidate
                .bytes()
                .all(|b| b.is_ascii_uppercase() || (b'2'..=b'7').contains(&b))
    }

    let forbidden_keys = [
        "secret",
        "secretKey",
        "privateKey",
        "apiKey",
        "api_key",
        "password",
        "authToken",
    ];

    for dir in failed_fixtures() {
        for file in ["rpc-response.json", "probe.json"] {
            let path = dir.join(file);
            let text = std::fs::read_to_string(&path).unwrap();

            for key in forbidden_keys {
                assert!(
                    !text.contains(&format!("\"{key}\"")),
                    "{}: contains a `{key}` field",
                    path.display()
                );
            }

            // Split on characters that cannot appear in a strkey so that base64
            // blobs are examined as whole tokens rather than sliding windows.
            for token in text.split(|c: char| !c.is_ascii_alphanumeric()) {
                assert!(
                    !is_strkey_seed(token),
                    "{}: contains what looks like a Stellar secret seed",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn probe_reports_agree_with_a_fresh_decode() {
    // Guards against a fixture being edited by hand so that its recorded probe
    // summary no longer describes its own response.
    for dir in failed_fixtures() {
        let probe: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("probe.json")).unwrap()).unwrap();
        let claimed_count = probe
            .get("diagnostic_events")
            .and_then(|d| d.get("count"))
            .and_then(|c| c.as_u64())
            .unwrap() as usize;

        let decoded = fixture::load(&dir).unwrap();
        assert_eq!(
            decoded.input.diagnostic_events.len(),
            claimed_count,
            "{}: probe.json claims {claimed_count} diagnostic events",
            dir.display()
        );
    }
}

#[test]
fn every_fixture_has_valid_metadata() {
    for dir in failed_fixtures() {
        let path = dir.join("metadata.json");
        let metadata: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap())
                .unwrap_or_else(|e| panic!("{} is invalid JSON: {e}", path.display()));

        let object = metadata
            .as_object()
            .unwrap_or_else(|| panic!("{} must contain a JSON object", path.display()));

        let required = [
            "transaction_hash",
            "network",
            "ledger",
            "captured_at",
            "rpc_provider",
            "failure_category",
            "fee_bumped",
            "diagnostic_event_count",
            "purpose",
        ];

        for field in required {
            assert!(
                object.contains_key(field),
                "{} is missing metadata field {field}",
                path.display()
            );
        }

        assert!(object["transaction_hash"].is_string());
        assert!(object["network"].is_string());
        assert!(object["ledger"].is_u64());
        assert!(object["captured_at"].is_string());
        assert!(object["rpc_provider"].is_string());
        assert!(object["failure_category"].is_string());
        assert!(object["fee_bumped"].is_boolean());
        assert!(object["diagnostic_event_count"].is_u64());
        assert!(object["purpose"].is_string());
    }
}

fn validate_metadata_claims(
    metadata: &serde_json::Value,
    probe: &serde_json::Value,
    rpc: &serde_json::Value,
) -> Result<(), String> {
    if metadata["transaction_hash"] != probe["transaction"] {
        return Err("transaction_hash does not match probe transaction".to_string());
    }

    if metadata["transaction_hash"] != rpc["txHash"] {
        return Err("transaction_hash does not match recorded RPC response".to_string());
    }

    if metadata["ledger"] != probe["ledger"] {
        return Err("ledger does not match probe ledger".to_string());
    }

    if metadata["ledger"] != rpc["ledger"] {
        return Err("ledger does not match recorded RPC response".to_string());
    }

    if metadata["diagnostic_event_count"] != probe["diagnostic_events"]["count"] {
        return Err("diagnostic_event_count does not match probe report".to_string());
    }

    Ok(())
}

#[test]
fn metadata_claims_match_recorded_response() {
    for dir in failed_fixtures() {
        let metadata: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("metadata.json")).unwrap())
                .unwrap();

        let probe: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("probe.json")).unwrap())
                .unwrap();

        let rpc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("rpc-response.json")).unwrap())
                .unwrap();
        validate_metadata_claims(&metadata, &probe, &rpc)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    }
}

#[test]
fn deliberately_mismatched_metadata_fails_validation() {
    let dir = failed_fixtures()
        .into_iter()
        .next()
        .expect("fixture corpus should not be empty");

    let mut metadata: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("metadata.json")).unwrap()).unwrap();

    metadata["transaction_hash"] = serde_json::Value::String("deliberately-wrong-hash".to_string());

    let probe: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("probe.json")).unwrap()).unwrap();

    let rpc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("rpc-response.json")).unwrap())
            .unwrap();

    let error = validate_metadata_claims(&metadata, &probe, &rpc)
        .expect_err("deliberately mismatched metadata should fail validation");

    assert!(
        error.contains("transaction_hash"),
        "validation error should explain the mismatched field: {error}"
    );

    let probe: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("probe.json")).unwrap()).unwrap();

    assert_ne!(
        metadata["transaction_hash"], probe["transaction"],
        "deliberately mismatched metadata must not match the recorded transaction"
    );
}
