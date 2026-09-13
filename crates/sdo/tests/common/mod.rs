//! Shared helpers for the M2/M3 integration tests. Offline only.

#![allow(dead_code)] // each test binary uses a different subset

use std::path::PathBuf;
use std::str::FromStr;

use soroban_failure_analysis::{AnalysisInput, TransactionModel};
use soroban_failure_rpc::fixture;
use stellar_xdr::ContractId;

/// The outer contract in the 49-event fixtures (spec: `Error`).
pub const HARVESTER: &str = "CBGSBKYMYO6OMGHQXXNOBRGVUDFUDVC2XLC3SXON5R2SNXILR7XCKKY3";
/// The inner farm contract, invoked by every Soroban fixture (spec: `Errors`).
pub const FARM: &str = "CDL74RF5BLYR2YBLCCI7F5FB6TPSCLKEJUBSD2RSVWZ4YHF3VMFAIGWA";

pub const FEE_BUMP_24: &str = "soroban-trapped-feebump-24ev";
pub const FEE_BUMP_49: &str = "soroban-trapped-feebump-49ev";
pub const FEE_BUMP_49_ALT: &str = "soroban-trapped-feebump-49ev-alt";
pub const CLASSIC: &str = "classic-failed-no-diagnostics";
pub const AUTH_EXPIRED: &str = "soroban-auth-signature-expired";
pub const AUTH_NONCE: &str = "soroban-auth-nonce-reused";

/// Analyse a fixture with contract specs resolved from `fixtures/contracts/`,
/// the way `sdo explain --contracts` does.
pub fn diagnose_with_specs(name: &str) -> soroban_failure_analysis::Diagnosis {
    let input = input(name);
    let needed = soroban_failure_analysis::contract::contracts_needing_specs(
        &TransactionModel::from_input(&input),
    );
    let source = soroban_failure_rpc::FixtureContractSource::new(root().join("contracts"));
    let specs = soroban_failure_rpc::fetch_specs(&source, &needed);
    soroban_failure_analysis::analyze(&input.with_contract_specs(specs))
}

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
}

pub fn input(name: &str) -> AnalysisInput {
    fixture::load(root().join("failed").join(name))
        .unwrap_or_else(|e| panic!("fixture {name}: {e}"))
        .input
}

pub fn model(name: &str) -> TransactionModel {
    TransactionModel::from_input(&input(name))
}

pub fn contract(id: &str) -> ContractId {
    ContractId::from_str(id).unwrap()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
