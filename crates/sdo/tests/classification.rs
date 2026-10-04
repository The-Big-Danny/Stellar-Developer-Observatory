//! M4: failure classification against **real** mainnet fixtures.
//!
//! Every Soroban fixture in `fixtures/failed/` reports the same opaque result,
//! `Trapped`. These tests pin down that the rules separate them by evidence —
//! and that none of them is turned into a cause the evidence does not support.
//!
//! Categories with no real fixture (archived entry, resource limit, resource
//! fee) are covered only by synthetic unit tests inside
//! `soroban-failure-analysis`, and are deliberately not pretended here. The
//! contract-trap and missing-authorization cases use deliberately caused
//! testnet fixtures, which are labelled as such in their own READMEs.

mod common;

use common::*;
use soroban_failure_analysis::{
    analyze, AnalysisInput, CauseClass, Confidence, Diagnosis, EvidenceSource, FailureStage,
    RuleStatus, TransactionModel, Verdict,
};

fn top(d: &Diagnosis) -> (CauseClass, Confidence) {
    let c = d.top_cause().expect("expected a candidate cause");
    (c.class, c.confidence)
}

fn status(d: &Diagnosis, rule: &str) -> RuleStatus {
    d.rule_reports
        .iter()
        .find(|r| r.rule_id == rule)
        .unwrap_or_else(|| panic!("no report for {rule}"))
        .status
}

// ---- contract-defined error (M3 resolution) -----------------------------------

#[test]
fn a_resolved_contract_error_is_confirmed_with_its_declared_name() {
    for name in [FEE_BUMP_49, FEE_BUMP_49_ALT] {
        let d = diagnose_with_specs(name);
        assert_eq!(
            d.candidate_causes.len(),
            1,
            "{name}: exactly one explanation"
        );
        assert_eq!(
            top(&d),
            (CauseClass::ContractDefinedError, Confidence::Confirmed),
            "{name}"
        );
        assert_eq!(d.verdict(), Verdict::Explained(Confidence::Confirmed));

        let c = d.top_cause().unwrap();
        assert!(c.summary.contains(HARVESTER), "{name}: {}", c.summary);
        assert!(
            c.summary.contains("Error::NoHarvestablePails"),
            "{}",
            c.summary
        );
        let text: Vec<&str> = c.evidence.iter().map(|e| e.observation.as_str()).collect();
        assert!(
            text.iter()
                .any(|t| t
                    .contains("70fe44694c9fe6b0abc69a6da4858fc2aaba04fa10492a466a1d426d04ca8560")),
            "WASM hash provenance missing: {text:?}"
        );
        assert!(
            text.iter()
                .any(|t| t.contains("Harvesting all pails results in 0 reward")),
            "the spec's documentation should be cited"
        );
        // The five caught inner failures are evidence, not a second cause.
        assert!(text.iter().any(
            |t| t.contains("Error(Contract, #9) (PailMissing)") && t.contains("without ending")
        ));
    }
}

#[test]
fn without_specs_the_contract_error_is_likely_and_unnamed() {
    let d = analyze(&input(FEE_BUMP_49));
    assert_eq!(
        top(&d),
        (CauseClass::ContractDefinedError, Confidence::Likely)
    );
    assert!(!d
        .top_cause()
        .unwrap()
        .summary
        .contains("NoHarvestablePails"));
}

// ---- footprint -----------------------------------------------------------------

#[test]
fn a_real_footprint_violation_is_confirmed_by_checking_the_key_against_the_footprint() {
    let d = analyze(&input(FEE_BUMP_24));
    assert_eq!(d.candidate_causes.len(), 1);
    assert_eq!(
        top(&d),
        (CauseClass::FootprintEntryMissing, Confidence::Confirmed)
    );
    let c = d.top_cause().unwrap();
    assert!(c.summary.contains(FARM));
    assert!(c.summary.contains("vec[Block, 183188]"), "{}", c.summary);
    assert!(c
        .evidence
        .iter()
        .any(|e| e.source == EvidenceSource::Footprint
            && e.observation
                .contains("is not among the 3 contract-data keys")));
    assert!(c.remediation.as_deref().unwrap().contains("Re-simulate"));
}

// ---- authorization ---------------------------------------------------------------

#[test]
fn a_real_expired_signature_is_confirmed_by_the_envelopes_own_expiration_ledger() {
    let d = analyze(&input(AUTH_EXPIRED));
    assert_eq!(d.candidate_causes.len(), 1);
    assert_eq!(
        top(&d),
        (CauseClass::InvalidAuthorizationEntry, Confidence::Confirmed)
    );
    let c = d.top_cause().unwrap();
    assert!(
        c.summary
            .contains("valid until ledger 64392366, checked at ledger 64392368"),
        "{}",
        c.summary
    );
    assert!(c
        .evidence
        .iter()
        .any(|e| e.source == EvidenceSource::AuthorizationEntry { index: 0 }));
}

#[test]
fn a_real_reused_nonce_is_likely_because_prior_use_cannot_be_proven_from_the_transaction() {
    let d = analyze(&input(AUTH_NONCE));
    assert_eq!(
        top(&d),
        (CauseClass::InvalidAuthorizationEntry, Confidence::Likely)
    );
    assert!(d.top_cause().unwrap().summary.contains("already consumed"));
}

#[test]
fn auth_is_classified_on_both_fee_bumped_and_plain_transactions() {
    assert!(model(AUTH_EXPIRED).is_fee_bumped());
    assert!(!model(AUTH_NONCE).is_fee_bumped());
    for name in [AUTH_EXPIRED, AUTH_NONCE] {
        assert_eq!(
            model(name).stage(),
            Some(FailureStage::ContractExecution),
            "{name}"
        );
    }
}

// ---- one result code, four different causes ----------------------------------------

#[test]
fn identical_trapped_results_are_separated_by_evidence_not_merged() {
    let cases = [
        (FEE_BUMP_24, CauseClass::FootprintEntryMissing),
        (FEE_BUMP_49, CauseClass::ContractDefinedError),
        (AUTH_EXPIRED, CauseClass::InvalidAuthorizationEntry),
        (AUTH_NONCE, CauseClass::InvalidAuthorizationEntry),
    ];
    for (name, expected) in cases {
        let m = model(name);
        assert_eq!(m.outcome.failed_operation.as_ref().unwrap().code, "Trapped");
        let d = analyze(&input(name));
        let classes: Vec<_> = d.candidate_causes.iter().map(|c| c.class).collect();
        assert_eq!(
            classes,
            vec![expected],
            "{name}: exactly the supported cause"
        );
    }
}

#[test]
fn each_rule_stays_silent_on_fixtures_that_belong_to_other_rules() {
    let d = analyze(&input(FEE_BUMP_24));
    assert_eq!(
        status(&d, "contract_defined_error"),
        RuleStatus::NotApplicable
    );
    assert_eq!(
        status(&d, "invalid_authorization_entry"),
        RuleStatus::NotApplicable
    );

    let d = analyze(&input(FEE_BUMP_49));
    assert_eq!(
        status(&d, "footprint_entry_missing"),
        RuleStatus::NoEvidence
    );
    assert_eq!(
        status(&d, "invalid_authorization_entry"),
        RuleStatus::NotApplicable
    );

    for name in [AUTH_EXPIRED, AUTH_NONCE] {
        let d = analyze(&input(name));
        assert_eq!(
            status(&d, "footprint_entry_missing"),
            RuleStatus::NoEvidence,
            "{name}"
        );
        assert_eq!(
            status(&d, "contract_defined_error"),
            RuleStatus::NotApplicable,
            "{name}"
        );
    }

    for name in [FEE_BUMP_24, FEE_BUMP_49, AUTH_EXPIRED, AUTH_NONCE] {
        let d = analyze(&input(name));
        for rule in [
            "archived_entry",
            "resource_limit_exceeded",
            "insufficient_resource_fee",
        ] {
            assert_eq!(
                status(&d, rule),
                RuleStatus::NotApplicable,
                "{name}: {rule}"
            );
        }
    }
}

// ---- classic negative control -------------------------------------------------------

#[test]
fn the_classic_negative_control_is_unsupported_not_forced_into_a_cause() {
    let d = analyze(&input(CLASSIC));
    assert_eq!(d.stage, Some(FailureStage::Operation));
    assert!(d.is_undetermined());
    assert_eq!(d.verdict(), Verdict::Unsupported);
    assert!(d
        .rule_reports
        .iter()
        .all(|r| r.status == RuleStatus::NotApplicable && r.reason.is_some()));
}

// ---- corpus-wide invariants ---------------------------------------------------------

#[test]
fn every_candidate_on_every_fixture_carries_evidence_and_a_remediation() {
    for name in [
        FEE_BUMP_24,
        FEE_BUMP_49,
        FEE_BUMP_49_ALT,
        AUTH_EXPIRED,
        AUTH_NONCE,
        CLASSIC,
    ] {
        let d = diagnose_with_specs(name);
        for c in &d.candidate_causes {
            assert!(
                !c.evidence.is_empty(),
                "{name}: {} has no evidence",
                c.rule_id
            );
            assert!(
                c.remediation.is_some(),
                "{name}: {} has no remediation",
                c.rule_id
            );
        }
        for r in &d.rule_reports {
            assert_eq!(
                r.reason.is_some(),
                r.status != RuleStatus::Matched,
                "{name}: {} must explain every non-match, and only non-matches",
                r.rule_id
            );
        }
    }
}

#[test]
fn classification_is_deterministic() {
    for name in [FEE_BUMP_24, FEE_BUMP_49, AUTH_EXPIRED, AUTH_NONCE] {
        assert_eq!(
            diagnose_with_specs(name),
            diagnose_with_specs(name),
            "{name}"
        );
    }
}

// ---- contract trap (real testnet fixture) -------------------------------------

/// Replace one host-message string inside the recorded diagnostic events,
/// leaving every other field of the real event untouched.
fn replace_host_message(input: &mut AnalysisInput, from: &str, to: &str) -> bool {
    use stellar_xdr::{ContractEventBody, ScString, ScVal};

    let mut changed = false;
    for ev in &mut input.diagnostic_events {
        let ContractEventBody::V0(body) = &mut ev.event.body;
        let ScVal::Vec(Some(items)) = &mut body.data else {
            continue;
        };
        let mut rebuilt: Vec<ScVal> = items.to_vec();
        for item in rebuilt.iter_mut() {
            let ScVal::String(s) = item else { continue };
            if std::str::from_utf8(s.as_slice()).is_ok_and(|text| text.contains(from)) {
                let text = std::str::from_utf8(s.as_slice()).unwrap().replace(from, to);
                *s = ScString(text.as_bytes().to_vec().try_into().unwrap());
                changed = true;
            }
        }
        if changed {
            *items = rebuilt.try_into().unwrap();
        }
    }
    changed
}

#[test]
fn a_real_testnet_trap_is_a_likely_contract_trap_never_confirmed() {
    let d = analyze(&input(TESTNET_TRAP));

    assert_eq!(top(&d), (CauseClass::ContractTrap, Confidence::Likely));
    assert_eq!(d.candidate_causes.len(), 1, "exactly one explanation");
    assert_eq!(d.verdict(), Verdict::Explained(Confidence::Likely));

    let c = d.top_cause().unwrap();
    assert!(
        c.summary.contains("function `trigger`") && c.summary.contains(TESTNET_TRAP_CONTRACT),
        "the candidate must name the frame that trapped: {}",
        c.summary
    );
    assert!(
        c.evidence
            .iter()
            .any(|e| e.observation.contains("UnreachableCodeReached")),
        "the host message must be cited: {:?}",
        c.evidence
    );
    assert!(
        c.evidence
            .iter()
            .any(|e| e.observation.contains("Error(WasmVm, InvalidAction)")),
        "the terminal error must be cited"
    );
}

#[test]
fn the_trap_rule_stays_silent_on_every_other_fixture() {
    for name in [
        FEE_BUMP_24,
        FEE_BUMP_49,
        FEE_BUMP_49_ALT,
        AUTH_EXPIRED,
        AUTH_NONCE,
        CLASSIC,
    ] {
        let d = analyze(&input(name));
        assert_eq!(
            status(&d, "contract_trap"),
            RuleStatus::NotApplicable,
            "{name}: the trap rule must not claim a fixture that ended with another error"
        );
    }
}

#[test]
fn a_different_wasm_trap_is_not_mistaken_for_a_panic() {
    // Same real transaction, same terminal error type, but the host reports a
    // different trap. Generic `function-trapped` must not become ContractTrap.
    let mut input = input(TESTNET_TRAP);
    assert!(
        replace_host_message(&mut input, "UnreachableCodeReached", "MemoryOutOfBounds"),
        "the recorded trap message must be present to be replaced"
    );

    let d = analyze(&input);
    assert_eq!(status(&d, "contract_trap"), RuleStatus::NoEvidence);
    assert!(
        d.candidate_causes
            .iter()
            .all(|c| c.class != CauseClass::ContractTrap),
        "a generic WasmVm trap must not be classified as a panic"
    );
}

// ---- missing authorization (real testnet fixture) -----------------------------

/// Replace the address in every authorization-error event with `to`.
fn replace_event_address(input: &mut AnalysisInput, to: &stellar_xdr::ScAddress) -> bool {
    use stellar_xdr::{ScVal, ScVec};

    let mut changed = false;
    for ev in &mut input.diagnostic_events {
        let stellar_xdr::ContractEventBody::V0(body) = &mut ev.event.body;
        let ScVal::Vec(Some(items)) = &mut body.data else {
            continue;
        };
        let mut rebuilt: Vec<ScVal> = items.to_vec();
        for item in rebuilt.iter_mut() {
            if let ScVal::Address(a) = item {
                *a = to.clone();
                changed = true;
            }
        }
        if changed {
            *items = ScVec::try_from(rebuilt).unwrap();
        }
    }
    changed
}

/// The authorization entries of a donor transaction's invocation, unwrapping a
/// fee bump to its inner transaction.
fn donor_invoke_auth(env: &TransactionEnvelope) -> Vec<SorobanAuthorizationEntry> {
    use stellar_xdr::{FeeBumpTransactionInnerTx, OperationBody};

    let inner = match env {
        TransactionEnvelope::Tx(e) => e,
        TransactionEnvelope::TxFeeBump(fb) => match &fb.tx.inner_tx {
            FeeBumpTransactionInnerTx::Tx(e) => e,
        },
        _ => panic!("donor is not a v1 or fee-bump envelope"),
    };
    match &inner.tx.operations[0].body {
        OperationBody::InvokeHostFunction(op) => op.auth.to_vec(),
        _ => panic!("donor operation is not an invoke"),
    }
}

/// Give the input the authorization entries of the donor, as if the transaction
/// had been signed with them.
fn with_auth_of(mut input: AnalysisInput, donor: &AnalysisInput) -> AnalysisInput {
    use stellar_xdr::{OperationBody, VecM};

    let donor_auth = donor_invoke_auth(&donor.envelope);
    let TransactionEnvelope::Tx(e) = &mut input.envelope else {
        panic!("target is not a v1 envelope");
    };
    let mut ops: Vec<stellar_xdr::Operation> = e.tx.operations.to_vec();
    let OperationBody::InvokeHostFunction(op) = &mut ops[0].body else {
        panic!("target operation is not an invoke");
    };
    op.auth = VecM::try_from(donor_auth).unwrap();
    e.tx.operations = ops.try_into().unwrap();
    input
}

use stellar_xdr::{SorobanAuthorizationEntry, SorobanCredentials, TransactionEnvelope};

fn source_address(input: &AnalysisInput) -> stellar_xdr::ScAddress {
    use std::str::FromStr;
    let key = TransactionModel::from_input(input).source_account;
    stellar_xdr::ScAddress::Account(stellar_xdr::AccountId::from_str(&key).unwrap())
}

#[test]
fn a_real_missing_authorization_is_confirmed_and_names_the_address() {
    let d = analyze(&input(TESTNET_MISSING_AUTH));

    assert_eq!(
        top(&d),
        (CauseClass::MissingAuthorizationEntry, Confidence::Confirmed)
    );
    assert_eq!(d.candidate_causes.len(), 1, "exactly one explanation");
    assert_eq!(d.verdict(), Verdict::Explained(Confidence::Confirmed));

    let c = d.top_cause().unwrap();
    assert_eq!(c.rule_id, "missing_authorization_entry");
    assert!(
        c.summary
            .contains("GCPT5IKE74AUKY5ZZIZISOG4YJJ6ABYGAU2I5HTUFDJSIDQVQLKD7FXF"),
        "the candidate must name the unauthorized address: {}",
        c.summary
    );
    assert!(
        c.evidence
            .iter()
            .any(|e| e.observation.contains("Unauthorized function call")),
        "the host message must be cited"
    );
    assert!(
        c.evidence
            .iter()
            .any(|e| e.observation.contains("0 authorization entries")),
        "the absence of an entry must be cited"
    );
}

#[test]
fn the_missing_authorization_rule_stays_silent_on_every_other_fixture() {
    for name in [
        FEE_BUMP_24,
        FEE_BUMP_49,
        FEE_BUMP_49_ALT,
        AUTH_EXPIRED,
        AUTH_NONCE,
        TESTNET_TRAP,
        CLASSIC,
    ] {
        let d = analyze(&input(name));
        assert_ne!(
            status(&d, "missing_authorization_entry"),
            RuleStatus::Matched,
            "{name}"
        );
        assert!(
            d.candidate_causes
                .iter()
                .all(|c| c.class != CauseClass::MissingAuthorizationEntry),
            "{name}: must not claim a missing entry"
        );
    }
}

#[test]
fn a_rejection_naming_the_source_account_is_not_a_missing_entry() {
    // The source account authorizes its own invocations implicitly, so a
    // rejection naming it cannot be a missing entry.
    let mut input = input(TESTNET_MISSING_AUTH);
    let source = source_address(&input);
    assert!(replace_event_address(&mut input, &source));

    let d = analyze(&input);
    assert_eq!(
        status(&d, "missing_authorization_entry"),
        RuleStatus::NotApplicable
    );
    assert!(d.candidate_causes.is_empty(), "no cause may be claimed");
}

#[test]
fn an_address_that_has_an_entry_is_an_invalid_entry_not_a_missing_one() {
    // Give the transaction a real authorization entry for an address, and point
    // the rejection at that address. The entry is present but not accepted, so
    // the missing-entry rule must decline rather than call it missing.
    let donor = input(AUTH_EXPIRED);
    let wallet = match &donor_invoke_auth(&donor.envelope)[0].credentials {
        SorobanCredentials::Address(c) => c.address.clone(),
        _ => panic!("donor entry has no address credentials"),
    };

    let mut input = with_auth_of(input(TESTNET_MISSING_AUTH), &donor);
    assert!(replace_event_address(&mut input, &wallet));

    let d = analyze(&input);
    assert_eq!(
        status(&d, "missing_authorization_entry"),
        RuleStatus::NoEvidence,
        "an entry for the address exists, so the missing-entry rule must not fire"
    );
    assert!(d
        .candidate_causes
        .iter()
        .all(|c| c.class != CauseClass::MissingAuthorizationEntry));
}
