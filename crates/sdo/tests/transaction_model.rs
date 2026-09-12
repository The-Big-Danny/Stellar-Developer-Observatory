//! M2: the canonical transaction model, against real mainnet fixtures.
//!
//! Every expected value below was read off the recorded responses in
//! `fixtures/failed/`, not assumed. Where the real corpus lacks a case (a
//! non-fee-bumped Soroban transaction, one with address auth entries) it is
//! covered by synthetic unit tests inside the analysis crate instead, and not
//! pretended here.

mod common;

use common::*;
use soroban_failure_analysis::model::{
    CallOutcome, DiagnosticAvailability, EventKind, OperationKind,
};
use soroban_failure_analysis::{analyze, FailureStage};
use stellar_xdr::{ScAddress, ScError, ScErrorCode};

const SOROBAN: [&str; 3] = [FEE_BUMP_24, FEE_BUMP_49, FEE_BUMP_49_ALT];

// ---- fee bumps -------------------------------------------------------------

#[test]
fn every_soroban_fixture_is_detected_as_fee_bumped() {
    for name in SOROBAN {
        let m = model(name);
        assert!(m.is_fee_bumped(), "{name}");
        let bump = m.fee_bump.as_ref().unwrap();
        assert_eq!(
            bump.fee_source, "GA2JRQOF6EA3HQWDCEDBPPMLYPJCFLDDGYZLEQGMS5SOBQIB3BAFHVAW",
            "{name}"
        );
        assert!(m.notes.is_empty(), "{name}: {:?}", m.notes);
    }
}

#[test]
fn the_inner_transaction_is_unwrapped_not_the_wrapper() {
    let m = model(FEE_BUMP_24);
    // The inner source differs from the fee source: proof we read the inner tx.
    assert_eq!(
        m.source_account,
        "GC3KTJPQKQZ3L55RWL2K5GCJUMFPREBTZQTFZKQOTPPSAWP5SFAZOGPC"
    );
    assert_ne!(m.source_account, m.fee_bump.as_ref().unwrap().fee_source);
    assert_eq!(m.fee, 237_240, "inner fee, not the wrapper's 237,441");
    assert_eq!(m.fee_bump.as_ref().unwrap().fee, 237_441);
}

#[test]
fn fee_bumped_failure_classifies_by_the_inner_result_not_the_wrapper() {
    for name in SOROBAN {
        let o = model(name).outcome;
        let bump = o.fee_bump.as_ref().expect("result is a fee-bump wrapper");
        assert_eq!(bump.outer_code, "TxFeeBumpInnerFailed", "{name}");
        assert_eq!(o.result_code, "TxFailed", "{name}: must be the inner code");
        let failed = o.failed_operation.as_ref().unwrap();
        assert_eq!(
            (failed.index, failed.operation, failed.code),
            (0, Some("InvokeHostFunction"), "Trapped"),
            "{name}"
        );
        assert_eq!(o.stage, Some(FailureStage::ContractExecution), "{name}");
    }
}

#[test]
fn the_inner_transaction_hash_is_recorded_from_the_result_pair() {
    let bump = model(FEE_BUMP_24).outcome.fee_bump.unwrap();
    assert_eq!(
        hex(&bump.inner_transaction_hash),
        "809dd637086258550ca9e1fa3d7ce494d84843cbe564ad821fdce1a24e86041c"
    );
}

// ---- operations and invocation --------------------------------------------

#[test]
fn invoke_host_function_and_its_contract_call_are_extracted() {
    let cases = [
        (FEE_BUMP_24, FARM, "work", 3),
        (FEE_BUMP_49, HARVESTER, "harvest", 2),
    ];
    for (name, contract_id, function, args) in cases {
        let m = model(name);
        assert_eq!(m.operations.len(), 1, "{name}");
        assert!(matches!(
            m.operations[0].kind,
            OperationKind::InvokeHostFunction {
                host_function: "InvokeContract",
                ..
            }
        ));
        let inv = m.invocation().unwrap();
        assert_eq!(
            inv.contract,
            ScAddress::Contract(contract(contract_id)),
            "{name}"
        );
        assert_eq!(inv.function, function, "{name}");
        assert_eq!(inv.args.len(), args, "{name}");
    }
}

// ---- classic negative control ---------------------------------------------

#[test]
fn classic_transaction_is_a_plain_non_soroban_negative_control() {
    let m = model(CLASSIC);
    assert!(!m.is_fee_bumped());
    assert!(!m.is_soroban());
    assert!(
        m.soroban.is_none(),
        "no Soroban data is not-applicable, not missing"
    );
    assert!(m.invocation().is_none());
    assert_eq!(m.operations.len(), 100);
    assert!(m
        .operations
        .iter()
        .all(|o| matches!(o.kind, OperationKind::Classic { .. })));
}

#[test]
fn classic_operation_failure_is_the_operation_stage_not_unknown() {
    let o = model(CLASSIC).outcome;
    assert_eq!(o.result_code, "TxFailed");
    assert_eq!(o.stage, Some(FailureStage::Operation));
    let failed = o.failed_operation.unwrap();
    assert_eq!(
        (failed.index, failed.operation, failed.code),
        (0, Some("CreateClaimableBalance"), "NoTrust")
    );
}

// ---- Soroban data: resources, footprint, auth ------------------------------

#[test]
fn declared_resources_are_extracted() {
    let d = model(FEE_BUMP_24).soroban.unwrap();
    assert_eq!(d.data_version, 0);
    assert_eq!(d.declared.instructions, 1_820_106);
    assert_eq!(d.declared.resource_fee, 237_240);
    assert!(d.declared.archived_entry_indexes.is_empty());

    let d = model(FEE_BUMP_49).soroban.unwrap();
    assert_eq!(d.declared.instructions, 6_732_347);
    assert_eq!(d.declared.resource_fee, 45_692);
}

#[test]
fn observed_consumption_is_kept_separate_from_declared_limits() {
    let m = model(FEE_BUMP_24);
    let declared = m.soroban.as_ref().unwrap().declared.instructions;
    // From the `core_metrics` diagnostic events: what the host actually used.
    assert_eq!(m.observed.cpu_instructions(), Some(1_354_586));
    assert_eq!(m.observed.memory_bytes(), Some(1_310_672));
    assert_ne!(m.observed.cpu_instructions(), Some(u64::from(declared)));

    let fees = m.observed.fees_charged.unwrap();
    assert_eq!(
        (fees.non_refundable, fees.refundable, fees.rent),
        (14_160, 0, 0)
    );
}

#[test]
fn footprint_is_extracted_with_read_only_and_read_write_separated() {
    let fp = model(FEE_BUMP_24).soroban.unwrap().footprint;
    assert_eq!((fp.read_only.len(), fp.read_write.len()), (1, 3));

    let fp = model(FEE_BUMP_49).soroban.unwrap().footprint;
    assert_eq!((fp.read_only.len(), fp.read_write.len()), (10, 6));
}

#[test]
fn footprint_names_the_wasm_each_transaction_loaded() {
    let farm_wasm = "db2c14290d4964e3805f2527dd132939ba5fb3fccac56b30bfab8fd091011627";
    let hashes: Vec<String> = model(FEE_BUMP_24)
        .soroban
        .unwrap()
        .footprint
        .contract_code_hashes()
        .iter()
        .map(|h| hex(h))
        .collect();
    assert_eq!(hashes, vec![farm_wasm]);
}

#[test]
fn real_fixtures_authorize_via_source_account_so_carry_no_auth_entries() {
    // A true fact about this corpus, not a gap in extraction: all three
    // transactions rely on the source-account signature. Address-credential
    // extraction is covered by synthetic tests in the analysis crate.
    for name in SOROBAN {
        assert!(model(name).soroban.unwrap().auth.is_empty(), "{name}");
    }
}

// ---- diagnostic events ------------------------------------------------------

#[test]
fn diagnostic_events_are_extracted_and_classified() {
    let m = model(FEE_BUMP_24);
    let d = &m.diagnostics;
    assert_eq!(d.availability, DiagnosticAvailability::Emitted);
    assert_eq!(d.events.len(), 24);
    assert_eq!(
        d.execution_event_count(),
        5,
        "19 of the 24 are core_metrics"
    );
    assert!(matches!(&d.events[0].kind, EventKind::FnCall { function, .. } if function == "work"));
}

#[test]
fn a_host_error_is_recorded_with_the_hosts_own_message() {
    // Observation only. The message names the footprint, but concluding that
    // the footprint was the *cause* is an M4 rule's job, and is not done here.
    let d = model(FEE_BUMP_24).diagnostics;
    let terminal = d.terminal_error.as_ref().unwrap();
    assert_eq!(terminal.error, ScError::Storage(ScErrorCode::ExceededLimit));

    let (_, _, message) = d.errors().next().unwrap();
    assert_eq!(
        message,
        Some("trying to access contract data key outside of the footprint")
    );
}

#[test]
fn the_call_tree_shows_caught_inner_failures_and_the_terminal_outer_failure() {
    let d = model(FEE_BUMP_49).diagnostics;
    let outer = &d.calls[0];
    assert_eq!((outer.depth, &outer.contract), (0, &contract(HARVESTER)));
    assert!(matches!(
        outer.outcome,
        CallOutcome::Failed {
            error: ScError::Contract(2),
            ..
        }
    ));

    let inner: Vec<_> = d.calls[1..].iter().collect();
    assert_eq!(inner.len(), 5, "five pails, five inner harvest calls");
    for frame in inner {
        assert_eq!((frame.depth, &frame.contract), (1, &contract(FARM)));
        assert_eq!(frame.function, "harvest");
        assert!(matches!(
            frame.outcome,
            CallOutcome::Failed {
                error: ScError::Contract(9),
                ..
            }
        ));
    }
}

#[test]
fn a_transaction_with_no_diagnostic_events_is_not_read_as_having_none() {
    // The classic response carries no `diagnosticEventsXdr` at all, so nothing
    // can be concluded from the silence.
    let m = model(CLASSIC);
    assert_eq!(
        m.diagnostics.availability,
        DiagnosticAvailability::NotEmitted
    );
    assert!(m.diagnostics.events.is_empty());
    assert!(m.diagnostics.terminal_error.is_none());
    assert!(m.observed.cpu_instructions().is_none(), "absent, not zero");
}

// ---- end to end --------------------------------------------------------------

#[test]
fn analyze_reports_the_stage_from_the_model() {
    // Causes are M4's job; see `classification.rs`.
    for (name, stage) in [
        (FEE_BUMP_24, FailureStage::ContractExecution),
        (FEE_BUMP_49, FailureStage::ContractExecution),
        (CLASSIC, FailureStage::Operation),
    ] {
        assert_eq!(analyze(&input(name)).stage, Some(stage), "{name}");
    }
}
