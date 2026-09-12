//! M3: contract error resolution against **real** mainnet data.
//!
//! The transactions come from `fixtures/failed/`; the contract instances and
//! WASM from `fixtures/contracts/`. Both are verbatim RPC recordings. The error
//! names asserted below are the ones the contracts' own specs declare.
//!
//! Resolution outcomes the real corpus does not contain (a spec with no name
//! for the code, conflicting names, a version mismatch) are covered by clearly
//! synthetic unit tests in `soroban-failure-analysis`, not faked here.

mod common;

use std::collections::BTreeMap;

use common::*;
use soroban_failure_analysis::analyze;
use soroban_failure_analysis::contract::{
    contracts_needing_specs, resolve, resolve_contract_errors, ContractErrorReport,
    ContractIdentification, ErrorResolution, IdentificationBasis, SpecAvailability, SpecProvenance,
};
use soroban_failure_analysis::model::EventKind;
use soroban_failure_rpc::{fetch_specs, FixtureContractSource};
use stellar_xdr::ScError;

fn contract_source() -> FixtureContractSource {
    FixtureContractSource::new(root().join("contracts"))
}

/// Resolve a fixture's errors the way the CLI does: learn which contracts are
/// needed, fetch their specs from the recorded fixtures, then analyse.
fn reports(name: &str) -> Vec<ContractErrorReport> {
    let input = input(name);
    let needed = contracts_needing_specs(&soroban_failure_analysis::TransactionModel::from_input(
        &input,
    ));
    let specs = fetch_specs(&contract_source(), &needed);
    analyze(&input.with_contract_specs(specs)).contract_errors
}

fn report(reports: &[ContractErrorReport], code: u32) -> &ContractErrorReport {
    reports
        .iter()
        .find(|r| r.code == code)
        .unwrap_or_else(|| panic!("no report for #{code}"))
}

#[test]
fn the_terminal_contract_error_is_named_from_the_contracts_real_spec() {
    for name in [FEE_BUMP_49, FEE_BUMP_49_ALT] {
        let reports = reports(name);
        let terminal = &reports[0];
        assert!(terminal.terminal, "{name}: terminal error is listed first");
        assert_eq!(terminal.code, 2);
        assert_eq!(
            terminal.identification,
            ContractIdentification::Unique {
                contract: contract(HARVESTER),
                basis: IdentificationBasis::SoleEmitter
            },
            "{name}"
        );
        match &terminal.resolution {
            ErrorResolution::Resolved {
                contract: c,
                enum_name,
                case_name,
                provenance,
                ..
            } => {
                assert_eq!(c, &contract(HARVESTER));
                assert_eq!(enum_name, "Error");
                assert_eq!(case_name, "NoHarvestablePails");
                assert!(matches!(
                    provenance,
                    SpecProvenance::MatchesFootprint { .. }
                ));
            }
            other => panic!("{name}: expected Resolved, got {other:?}"),
        }
    }
}

#[test]
fn a_caught_inner_error_is_attributed_to_its_origin_and_named_from_its_spec() {
    // The harvester re-emits #9 when reporting each caught try_call failure,
    // so two contracts emit it. The origin marker picks out the farm contract,
    // whose spec — not the harvester's — defines #9.
    let reports = reports(FEE_BUMP_49);
    let caught = report(&reports, 9);
    assert!(!caught.terminal);
    assert_eq!(
        caught.identification,
        ContractIdentification::Unique {
            contract: contract(FARM),
            basis: IdentificationBasis::OriginMarker
        }
    );
    assert_eq!(caught.resolution.name(), Some("PailMissing"));
}

#[test]
fn a_resolution_cites_the_events_carrying_the_error() {
    let input = input(FEE_BUMP_49);
    let model = soroban_failure_analysis::TransactionModel::from_input(&input);
    let reports = reports(FEE_BUMP_49);
    for r in &reports {
        assert!(!r.event_indexes.is_empty(), "#{} cites no evidence", r.code);
        for &i in &r.event_indexes {
            let carries = match &model.diagnostics.events[i as usize].kind {
                EventKind::Error { error, .. } | EventKind::HostFnFailed { error } => {
                    *error == ScError::Contract(r.code)
                }
                _ => false,
            };
            assert!(carries, "event {i} cited for #{} does not carry it", r.code);
        }
    }
}

#[test]
fn only_the_contracts_that_raised_errors_are_fetched() {
    let model = model(FEE_BUMP_49);
    let mut needed = contracts_needing_specs(&model);
    needed.sort();
    let mut expected = vec![contract(HARVESTER), contract(FARM)];
    expected.sort();
    assert_eq!(needed, expected);
}

#[test]
fn each_recorded_spec_matches_the_wasm_its_transactions_loaded() {
    // Specs were captured after the transactions; contracts can be upgraded.
    // The footprint proves these specs describe the code that actually ran.
    let specs = fetch_specs(&contract_source(), &[contract(HARVESTER), contract(FARM)]);
    for (name, ids) in [
        (FEE_BUMP_24, vec![FARM]),
        (FEE_BUMP_49, vec![HARVESTER, FARM]),
        (FEE_BUMP_49_ALT, vec![HARVESTER, FARM]),
    ] {
        let loaded = model(name)
            .soroban
            .unwrap()
            .footprint
            .contract_code_hashes();
        for id in ids {
            let SpecAvailability::Available(spec) = &specs[&contract(id)] else {
                panic!("{id}: spec fixture did not load");
            };
            assert!(
                loaded.contains(&spec.wasm_hash.unwrap()),
                "{name}: footprint does not contain {id}'s recorded WASM"
            );
        }
    }
}

#[test]
fn a_host_error_is_not_applicable_to_contract_resolution() {
    // The 24-event fixture fails with Error(Storage, ExceededLimit): a host
    // error with no contract-defined name to look up.
    let model = model(FEE_BUMP_24);
    let terminal = model.diagnostics.terminal_error.clone().unwrap();
    assert!(!matches!(terminal.error, ScError::Contract(_)));

    let r = resolve(
        &terminal.error,
        &ContractIdentification::Unidentified,
        &BTreeMap::new(),
        None,
    );
    assert_eq!(r, ErrorResolution::NotApplicable);
    assert!(
        reports(FEE_BUMP_24).is_empty(),
        "no contract error codes appear"
    );
}

#[test]
fn without_specs_errors_are_reported_unavailable_never_guessed() {
    // Offline analysis with no contract fixtures supplied.
    let d = analyze(&input(FEE_BUMP_49));
    assert_eq!(d.contract_errors.len(), 2);
    for r in &d.contract_errors {
        assert!(
            matches!(&r.resolution, ErrorResolution::SpecUnavailable { reason, .. }
                if reason.contains("no contract spec was supplied")),
            "#{}: {:?}",
            r.code,
            r.resolution
        );
        assert_eq!(r.resolution.name(), None);
    }
    assert!(d
        .limitations
        .iter()
        .any(|l| l.contains("could not be named")));
}

#[test]
fn a_contract_with_no_recorded_fixture_is_unavailable_with_a_reason() {
    // A syntactically valid ID with no recorded fixture.
    let unknown = stellar_xdr::ContractId(stellar_xdr::Hash([0; 32]));
    let specs = fetch_specs(&contract_source(), std::slice::from_ref(&unknown));
    assert!(matches!(
        &specs[&unknown],
        SpecAvailability::Unavailable { reason } if reason.contains("not found")
    ));
}

#[test]
fn a_classic_transaction_has_no_contract_errors() {
    assert!(resolve_contract_errors(&model(CLASSIC), &BTreeMap::new()).is_empty());
}

#[test]
fn resolution_is_deterministic() {
    assert_eq!(reports(FEE_BUMP_49), reports(FEE_BUMP_49));
}
