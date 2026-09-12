//! M4: failure classification against **real** mainnet fixtures.
//!
//! Every Soroban fixture in `fixtures/failed/` reports the same opaque result,
//! `Trapped`. These tests pin down that the rules separate them by evidence —
//! and that none of them is turned into a cause the evidence does not support.
//!
//! Categories with no real fixture (archived entry, resource limit, resource
//! fee, missing authorization) are covered only by synthetic unit tests inside
//! `soroban-failure-analysis`, and are deliberately not pretended here.

mod common;

use common::*;
use soroban_failure_analysis::{
    analyze, CauseClass, Confidence, Diagnosis, EvidenceSource, FailureStage, RuleStatus, Verdict,
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
