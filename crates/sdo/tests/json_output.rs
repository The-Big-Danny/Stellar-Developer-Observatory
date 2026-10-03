//! Offline integration tests for `sdo explain --json`.
//!
//! **These tests must never touch the network.** They run the compiled `sdo`
//! binary as a subprocess against a committed fixture and compare the
//! resulting JSON against the in-process `analyze` result for the same
//! fixture, so the test cannot pass by accident if the two pipelines diverge.

use std::path::{Path, PathBuf};
use std::process::Command;

use soroban_failure_analysis::contract::contracts_needing_specs;
use soroban_failure_analysis::{analyze, TransactionModel};
use soroban_failure_rpc::{fetch_specs, fixture, FixtureContractSource};

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
}

/// Build the same diagnosis the CLI builds, in-process, for comparison.
fn diagnose(fixture_name: &str) -> soroban_failure_analysis::Diagnosis {
    let dir = fixtures_root().join("failed").join(fixture_name);
    let decoded = fixture::load(&dir).unwrap_or_else(|e| panic!("{fixture_name}: {e}"));
    let model = TransactionModel::from_input(&decoded.input);
    let needed = contracts_needing_specs(&model);
    let source = FixtureContractSource::new(fixtures_root().join("contracts"));
    let specs = fetch_specs(&source, &needed);
    analyze(&decoded.input.with_contract_specs(specs))
}

/// Run `sdo explain --fixture <name> --contracts fixtures/contracts --json`
/// and return stdout, parsed as JSON.
fn run_json(fixture_name: &str) -> serde_json::Value {
    let dir = fixtures_root().join("failed").join(fixture_name);
    let output = Command::new(env!("CARGO_BIN_EXE_sdo"))
        .arg("explain")
        .arg("--fixture")
        .arg(&dir)
        .arg("--contracts")
        .arg(fixtures_root().join("contracts"))
        .arg("--json")
        .output()
        .expect("failed to run the sdo binary");

    assert!(
        output.status.success(),
        "sdo explain --json exited with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout was not valid JSON: {e}\n{}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

/// A 64-character lowercase hex string, as a hash should be serialised.
fn is_lowercase_hex_64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[test]
fn json_output_matches_the_in_process_diagnosis() {
    for fixture_name in [
        "soroban-trapped-feebump-49ev",
        "soroban-auth-signature-expired",
    ] {
        let value = run_json(fixture_name);
        let expected = diagnose(fixture_name);

        // Verdict.
        let expected_verdict = expected.verdict();
        let verdict = &value["verdict"];
        match expected_verdict {
            soroban_failure_analysis::Verdict::Explained(confidence) => {
                assert_eq!(verdict["kind"], "EXPLAINED", "{fixture_name}");
                assert_eq!(
                    verdict["confidence"],
                    confidence.id().to_uppercase(),
                    "{fixture_name}"
                );
            }
            soroban_failure_analysis::Verdict::NotAFailure => {
                assert_eq!(verdict["kind"], "NOT_A_FAILURE", "{fixture_name}");
            }
            soroban_failure_analysis::Verdict::InsufficientEvidence => {
                assert_eq!(verdict["kind"], "INSUFFICIENT_EVIDENCE", "{fixture_name}");
            }
            soroban_failure_analysis::Verdict::Unsupported => {
                assert_eq!(verdict["kind"], "UNSUPPORTED", "{fixture_name}");
            }
        }

        // Top cause, when there is one.
        match expected.top_cause() {
            Some(top) => {
                let causes = value["candidate_causes"]
                    .as_array()
                    .unwrap_or_else(|| panic!("{fixture_name}: candidate_causes is not an array"));
                let first = causes
                    .first()
                    .unwrap_or_else(|| panic!("{fixture_name}: candidate_causes is empty"));
                assert_eq!(
                    first["class"],
                    top.class.id().to_uppercase(),
                    "{fixture_name}"
                );
                assert_eq!(first["rule_id"], top.rule_id, "{fixture_name}");
                assert_eq!(
                    first["confidence"],
                    top.confidence.id().to_uppercase(),
                    "{fixture_name}"
                );
            }
            None => {
                assert!(
                    value["candidate_causes"]
                        .as_array()
                        .is_some_and(Vec::is_empty),
                    "{fixture_name}: expected no candidate causes"
                );
            }
        }

        // Transaction hash and stage line up with the in-process diagnosis.
        let expected_hash = expected
            .transaction_hash
            .clone()
            .map_or(serde_json::Value::Null, serde_json::Value::String);
        assert_eq!(value["transaction_hash"], expected_hash, "{fixture_name}");

        let expected_stage = expected
            .stage
            .map_or(serde_json::Value::Null, |s| s.id().to_uppercase().into());
        assert_eq!(value["stage"], expected_stage, "{fixture_name}");

        // Determinism: parsing twice from the same stdout bytes is trivially
        // equal, so instead run the binary again and compare values.
        let second = run_json(fixture_name);
        assert_eq!(
            value, second,
            "{fixture_name}: JSON output is not deterministic"
        );
    }
}

#[test]
fn wasm_hashes_are_lowercase_hex_not_byte_arrays() {
    let value = run_json("soroban-trapped-feebump-49ev");
    let errors = value["contract_errors"]
        .as_array()
        .expect("contract_errors should be an array");
    assert!(!errors.is_empty(), "fixture should have contract errors");

    let mut saw_a_hash = false;
    for error in errors {
        let Some(provenance) = error.pointer("/resolution/provenance") else {
            continue;
        };
        if provenance["kind"] == "matches_footprint" {
            let hash = provenance["wasm_hash"]
                .as_str()
                .expect("wasm_hash should be a JSON string, not an array of numbers");
            assert!(
                is_lowercase_hex_64(hash),
                "wasm_hash {hash:?} is not 64 lowercase hex characters"
            );
            saw_a_hash = true;
        }
    }
    assert!(
        saw_a_hash,
        "fixture should exercise at least one MatchesFootprint provenance"
    );
}

#[test]
fn human_output_is_unchanged_without_the_json_flag() {
    let dir = fixtures_root()
        .join("failed")
        .join("soroban-trapped-feebump-49ev");
    let output = Command::new(env!("CARGO_BIN_EXE_sdo"))
        .arg("explain")
        .arg("--fixture")
        .arg(&dir)
        .arg("--contracts")
        .arg(fixtures_root().join("contracts"))
        .output()
        .expect("failed to run the sdo binary");

    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.starts_with("Transaction     "), "{text}");
    assert!(
        !text.trim_start().starts_with('{'),
        "looks like JSON leaked into the human path"
    );
}
