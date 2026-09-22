//! M5.2: deduplication cluster keys, derived from recorded mainnet data.
//!
//! The unit tests in `soroban-failure-rpc` cover the derivation rules in
//! isolation. These check the same code against real recorded transactions and
//! real recorded contract instances, so the keys the population pilot writes
//! are known to come out of actual mainnet bytes and not only out of
//! hand-built values.

mod common;

use common::*;
use soroban_failure_analysis::TransactionModel;
use soroban_failure_rpc::cluster::{
    code_cluster, root_contract_id, submitter_cluster, CodeIdentity, InstanceLookup, PilotRecord,
    UnresolvedCode,
};
use soroban_failure_rpc::{ContractSource, FixtureContractSource};
use stellar_xdr::{ContractExecutable, ContractId, Hash};

/// The recorded instances under `fixtures/contracts/`.
fn recorded_instances() -> FixtureContractSource {
    FixtureContractSource::new(root().join("contracts"))
}

fn executable(contract: &str) -> ContractExecutable {
    let id: ContractId = contract.parse().expect("a contract strkey");
    recorded_instances()
        .contract_executable(&id)
        .expect("a recorded instance")
}

fn footprint_hashes(model: &TransactionModel) -> Vec<[u8; 32]> {
    model
        .soroban
        .as_ref()
        .expect("Soroban transaction data")
        .footprint
        .contract_code_hashes()
}

#[test]
fn the_root_contracts_recorded_instance_resolves_to_the_wasm_it_ran() {
    let model = model(FEE_BUMP_49);
    assert_eq!(root_contract_id(&model).as_deref(), Some(HARVESTER));

    let identity = code_cluster(Some(&executable(HARVESTER)), &footprint_hashes(&model));
    assert_eq!(
        identity,
        CodeIdentity::Resolved(
            "wasm:70fe44694c9fe6b0abc69a6da4858fc2aaba04fa10492a466a1d426d04ca8560".into()
        )
    );
}

#[test]
fn a_missing_instance_recording_never_becomes_a_contract_id() {
    let model = model(FEE_BUMP_49);
    let identity = code_cluster(None, &footprint_hashes(&model));
    assert_eq!(
        identity,
        CodeIdentity::Unresolved(UnresolvedCode::InstanceUnavailable)
    );
    // The point of the rule: the contract ID is right there, and is still not
    // used.
    match identity {
        CodeIdentity::Resolved(key) => panic!("fell back to {key}"),
        CodeIdentity::Unresolved(reason) => {
            assert_eq!(reason.reason(), "code_identity_instance_unavailable")
        }
    }
}

#[test]
fn a_contract_upgraded_since_the_transaction_ran_is_not_clustered_with_it() {
    // The inner farm contract's WASM is in this transaction's footprint; the
    // harvester's is not the same hash. Using the wrong instance must not
    // produce a key.
    let model = model(FEE_BUMP_49);
    let farm_only: Vec<[u8; 32]> = footprint_hashes(&model)
        .into_iter()
        .filter(|h| h != &executable_hash(HARVESTER))
        .collect();
    assert_eq!(
        code_cluster(Some(&executable(HARVESTER)), &farm_only),
        CodeIdentity::Unresolved(UnresolvedCode::NotInFootprint)
    );
}

fn executable_hash(contract: &str) -> [u8; 32] {
    match executable(contract) {
        ContractExecutable::Wasm(Hash(h)) => h,
        other => panic!("{contract} is not WASM backed: {other:?}"),
    }
}

#[test]
fn a_fee_bumped_transaction_clusters_by_its_fee_source_not_its_inner_source() {
    let model = model(FEE_BUMP_49);
    assert!(model.is_fee_bumped());
    assert_eq!(
        submitter_cluster(&model),
        "GA2JRQOF6EA3HQWDCEDBPPMLYPJCFLDDGYZLEQGMS5SOBQIB3BAFHVAW"
    );
    assert_ne!(submitter_cluster(&model), model.source_account);
}

#[test]
fn a_plain_transaction_clusters_by_its_own_source_account() {
    let model = model(AUTH_NONCE);
    assert!(!model.is_fee_bumped());
    assert_eq!(submitter_cluster(&model), model.source_account);
}

#[test]
fn a_pilot_record_from_a_real_transaction_carries_observations_only() {
    let model = model(FEE_BUMP_49);
    let record = PilotRecord::new(
        "0".repeat(64).as_str(),
        1,
        &model,
        Some(&InstanceLookup::Found(executable(HARVESTER))),
    );

    assert!(record.fee_bumped);
    assert_eq!(
        record.inner_transaction_hash.as_deref(),
        Some(
            hex(&model
                .outcome
                .fee_bump
                .as_ref()
                .unwrap()
                .inner_transaction_hash)
            .as_str()
        ),
        "a fee bump's inner hash is recorded so it can be excluded too"
    );
    assert_eq!(record.operation_count, 1);
    assert_eq!(record.diagnostic_event_count, 49);
    assert_eq!(record.instance_lookup.as_deref(), Some("found"));
    assert_eq!(record.root_contract_id.as_deref(), Some(HARVESTER));
    assert!(record.code_cluster.as_deref().unwrap().starts_with("wasm:"));
    assert_eq!(record.code_unresolved_reason, None);
    assert_eq!(record.terminal_error.as_deref(), Some("Contract"));

    // Nothing SDO concluded may appear in the serialised line.
    let line = serde_json::to_string(&record).unwrap();
    for word in ["cause", "verdict", "confidence", "rule_id", "Explained"] {
        assert!(!line.contains(word), "{word} leaked into {line}");
    }
}

#[test]
fn a_failed_or_empty_instance_lookup_is_unresolved_in_the_record_too() {
    // Whether the provider said "no such entry" or did not answer, the record
    // has no cluster key, keeps the contract ID only as the root contract, and
    // says which of the two happened.
    let model = model(FEE_BUMP_49);
    for (lookup, status) in [
        (InstanceLookup::NotFound, "not_found"),
        (InstanceLookup::Failed, "failed"),
    ] {
        let record = PilotRecord::new("0".repeat(64).as_str(), 1, &model, Some(&lookup));
        assert_eq!(record.code_cluster, None, "{status}");
        assert_eq!(
            record.code_unresolved_reason,
            Some(UnresolvedCode::InstanceUnavailable),
            "{status}"
        );
        assert_eq!(record.instance_lookup.as_deref(), Some(status));
        assert_eq!(record.root_contract_id.as_deref(), Some(HARVESTER));
    }
}

#[test]
fn a_plain_transaction_has_no_inner_hash() {
    let model = model(AUTH_NONCE);
    let record = PilotRecord::new("0".repeat(64).as_str(), 1, &model, None);
    assert!(!record.fee_bumped);
    assert_eq!(record.inner_transaction_hash, None);
}
