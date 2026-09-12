//! Pure, deterministic analysis of failed Soroban transactions.
//!
//! # Contract of this crate
//!
//! This crate performs **no I/O**. No network, no filesystem, no clock, no
//! environment variables, no global state. Given the same [`AnalysisInput`] it
//! always produces the same [`Diagnosis`]. That is what makes the whole rule
//! corpus testable from committed fixtures without mainnet access, which is in
//! turn what makes outside contribution possible.
//!
//! Fetching and decoding live under [`soroban-failure-rpc`][rpc]; presentation
//! lives in the `sdo` CLI.
//!
//! # Status
//!
//! **Milestones M2–M3.**
//!
//! * [`TransactionModel`] (M2) is the canonical view of a transaction: fee
//!   bumps unwrapped, [`FailureStage`] classified, diagnostic events and the
//!   call tree reconstructed, declared and observed resources kept apart.
//! * [`contract`] (M3) names contract-defined errors from specs the caller
//!   supplies, and reports explicitly when it cannot.
//!
//! * [`rules`] (M4) turns that evidence into ranked candidate causes. Each rule
//!   returns a match, "no evidence", or "not applicable", and every outcome is
//!   kept in [`Diagnosis::rule_reports`], so [`Diagnosis::verdict`] can
//!   distinguish an explained failure from an unknown or unsupported one.
//!
//! Not every cause category has a rule; see `docs/architecture/rules.md`.
//!
//! This crate would rather return "undetermined" than a guess.
//!
//! # Example
//!
//! ```no_run
//! use soroban_failure_analysis::{analyze, AnalysisInput};
//! use stellar_xdr::{Limits, ReadXdr, TransactionEnvelope, TransactionResult};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let (envelope_b64, result_b64) = ("", "");
//! let envelope = TransactionEnvelope::from_xdr_base64(envelope_b64, Limits::none())?;
//! let result = TransactionResult::from_xdr_base64(result_b64, Limits::none())?;
//!
//! let input = AnalysisInput::builder(envelope, result).build();
//! let diagnosis = analyze(&input);
//!
//! for limitation in &diagnosis.limitations {
//!     eprintln!("note: {limitation}");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! [rpc]: https://docs.rs/soroban-failure-rpc

#![doc(html_root_url = "https://docs.rs/soroban-failure-analysis")]

pub mod contract;
pub mod diagnosis;
pub mod input;
pub mod model;
pub mod rule;
pub mod rules;
pub mod taxonomy;

#[cfg(test)]
pub(crate) mod testutil;

pub use diagnosis::{
    CandidateCause, Confidence, Diagnosis, Evidence, EvidenceSource, RuleReport, RuleStatus,
    Verdict,
};
pub use input::{AnalysisInput, AnalysisInputBuilder};
pub use model::TransactionModel;
pub use rule::{FailureContext, Rule, RuleOutcome, RuleRegistry};
pub use taxonomy::{CauseClass, FailureStage};

/// Analyse a failed transaction with the built-in rules.
///
/// See [`analyze_with`] to supply your own registry.
pub fn analyze(input: &AnalysisInput) -> Diagnosis {
    analyze_with(input, &RuleRegistry::builtin())
}

/// Analyse a failed transaction against a specific rule registry.
///
/// Candidate causes are returned ranked by [`Confidence`], strongest first.
/// Ranking is stable: rules of equal confidence keep their registration order,
/// so output does not shift between runs.
pub fn analyze_with(input: &AnalysisInput, registry: &RuleRegistry) -> Diagnosis {
    let model = TransactionModel::from_input(input);
    let stage = model.stage();
    let contract_errors = contract::resolve_contract_errors(&model, &input.contract_specs);

    let ctx = FailureContext {
        input,
        model: &model,
        stage: stage.unwrap_or(FailureStage::Unknown),
        contract_errors: &contract_errors,
    };

    let mut candidate_causes: Vec<CandidateCause> = Vec::new();
    let mut rule_reports: Vec<RuleReport> = Vec::new();
    for rule in registry.iter() {
        let (status, reason) = match rule.evaluate(&ctx) {
            RuleOutcome::Match(cause) => {
                candidate_causes.push(cause);
                (RuleStatus::Matched, None)
            }
            RuleOutcome::NoEvidence { reason } => (RuleStatus::NoEvidence, Some(reason)),
            RuleOutcome::NotApplicable { reason } => (RuleStatus::NotApplicable, Some(reason)),
        };
        rule_reports.push(RuleReport {
            rule_id: rule.id().to_string(),
            status,
            reason,
        });
    }

    // Strongest confidence first. `sort_by_key` is stable, so equal-confidence
    // candidates retain registration order and output stays deterministic.
    candidate_causes.sort_by_key(|c| core::cmp::Reverse(c.confidence));

    let mut limitations = Vec::new();

    match stage {
        None => limitations
            .push("The transaction succeeded; there is no failure to analyse.".to_string()),
        Some(stage) if candidate_causes.is_empty() => {
            if rule_reports
                .iter()
                .any(|r| r.status == RuleStatus::NoEvidence)
            {
                limitations.push(
                    "No rule could establish a cause: the evidence each applicable rule needs \
                     is absent or inconclusive. `rule_reports` states what each one lacked."
                        .to_string(),
                );
            } else {
                limitations.push(format!(
                    "No implemented rule covers failures at stage `{stage}`; the cause is \
                     reported as unsupported rather than guessed."
                ));
            }
        }
        Some(_) => {}
    }

    let unnamed = contract_errors
        .iter()
        .filter(|r| r.resolution.name().is_none())
        .count();
    if unnamed > 0 {
        limitations.push(format!(
            "{unnamed} contract error code(s) could not be named; each report in \
             `contract_errors` states why."
        ));
    }

    if !input.diagnostics_enabled {
        limitations.push(
            "Diagnostic events were not available from the source. Their absence \
             carries no information: the node may simply not emit them."
                .to_string(),
        );
    }

    Diagnosis {
        transaction_hash: input.transaction_hash.clone(),
        stage,
        candidate_causes,
        contract_errors,
        rule_reports,
        limitations,
        rules_evaluated: registry.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::minimal_input;
    use stellar_xdr::{Limits, ReadXdr, TransactionEnvelope, WriteXdr};

    #[test]
    fn a_failure_no_rule_covers_is_unsupported_not_guessed() {
        let diagnosis = analyze(&minimal_input());
        // TxFailed with no failing operation: failed, but the stage is not
        // determinable. Not `None`, which would claim success.
        assert_eq!(diagnosis.stage, Some(FailureStage::Unknown));
        assert!(diagnosis.is_undetermined());
        assert_eq!(diagnosis.rules_evaluated, RuleRegistry::builtin().len());
        assert_eq!(diagnosis.rule_reports.len(), diagnosis.rules_evaluated);
        assert!(diagnosis
            .rule_reports
            .iter()
            .all(|r| r.status == RuleStatus::NotApplicable && r.reason.is_some()));
        assert_eq!(diagnosis.verdict(), Verdict::Unsupported);
    }

    #[test]
    fn a_generic_trap_without_evidence_is_insufficient_evidence_not_a_cause() {
        use crate::testutil::soroban_failure;
        use stellar_xdr::InvokeHostFunctionResult as R;

        // Diagnostics emitted, but none for this transaction.
        let diagnosis = analyze(&soroban_failure(R::Trapped, Vec::new(), Some(Vec::new())));
        assert_eq!(diagnosis.stage, Some(FailureStage::ContractExecution));
        assert!(diagnosis.is_undetermined());
        assert_eq!(diagnosis.verdict(), Verdict::InsufficientEvidence);
        assert!(diagnosis
            .limitations
            .iter()
            .any(|l| l.contains("No rule could establish a cause")));
    }

    #[test]
    fn several_candidates_are_ranked_by_confidence_then_registration_order() {
        use crate::contract::ORIGIN_MARKER;
        use crate::testutil::{
            call, cid, err, footprint_violation, host_fn_failed, soroban_failure, sym,
        };
        use stellar_xdr::{InvokeHostFunctionResult as R, ScError, ScVal, ScVec};

        // A footprint violation is raised but does not end the invocation;
        // the contract then fails with its own error. Two rules match, and the
        // one supported by the terminal error must rank first.
        let key = ScVal::Vec(Some(ScVec(vec![sym("K")].try_into().unwrap())));
        let input = soroban_failure(
            R::Trapped,
            Vec::new(),
            Some(vec![
                call(None, cid(5), "f"),
                footprint_violation(cid(5), key),
                err(cid(5), ScError::Contract(4), ORIGIN_MARKER),
                host_fn_failed(ScError::Contract(4)),
            ]),
        );
        let d = analyze(&input);
        let ranked: Vec<_> = d
            .candidate_causes
            .iter()
            .map(|c| (c.class, c.confidence))
            .collect();
        assert_eq!(
            ranked,
            vec![
                (CauseClass::ContractDefinedError, Confidence::Likely),
                (CauseClass::FootprintEntryMissing, Confidence::Possible),
            ]
        );
        assert_eq!(d.verdict(), Verdict::Explained(Confidence::Likely));
    }

    #[test]
    fn equal_confidence_keeps_registration_order() {
        struct Fixed(&'static str);
        impl Rule for Fixed {
            fn id(&self) -> &'static str {
                self.0
            }
            fn description(&self) -> &'static str {
                "test"
            }
            fn evaluate(&self, _: &FailureContext<'_>) -> RuleOutcome {
                RuleOutcome::Match(CandidateCause {
                    class: CauseClass::Undetermined,
                    confidence: Confidence::Likely,
                    summary: self.0.into(),
                    evidence: Vec::new(),
                    remediation: None,
                    rule_id: self.0.into(),
                })
            }
        }
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(Fixed("first")))
            .register(Box::new(Fixed("second")));
        let d = analyze_with(&minimal_input(), &reg);
        let ids: Vec<_> = d
            .candidate_causes
            .iter()
            .map(|c| c.rule_id.as_str())
            .collect();
        assert_eq!(ids, vec!["first", "second"]);
    }

    #[test]
    fn analyze_states_its_limitations_rather_than_degrading_silently() {
        let diagnosis = analyze(&minimal_input());
        assert!(
            diagnosis
                .limitations
                .iter()
                .any(|l| l.contains("rather than guessed")),
            "must disclose that no rule covers this failure"
        );
        assert!(
            diagnosis
                .limitations
                .iter()
                .any(|l| l.contains("Diagnostic events were not available")),
            "must disclose missing diagnostic events"
        );
    }

    #[test]
    fn diagnostics_flag_distinguishes_absent_from_empty() {
        let base = minimal_input();
        assert!(!base.diagnostics_enabled);
        assert!(!base.has_diagnostic_evidence());

        let with_empty = AnalysisInput::builder(base.envelope.clone(), base.result.clone())
            .diagnostic_events(Vec::new())
            .build();
        // Node emits diagnostics, but this transaction had none. That is a real,
        // usable observation -- distinct from the node not emitting any.
        assert!(with_empty.diagnostics_enabled);
        assert!(!with_empty.has_diagnostic_evidence());
    }

    #[test]
    fn analysis_is_deterministic() {
        let input = minimal_input();
        assert_eq!(analyze(&input), analyze(&input));
    }

    #[test]
    fn minimal_fixture_round_trips_through_xdr() {
        // Guards against the hand-built test envelope drifting out of shape.
        let input = minimal_input();
        let b64 = input.envelope.to_xdr_base64(Limits::none()).unwrap();
        let decoded = TransactionEnvelope::from_xdr_base64(&b64, Limits::none()).unwrap();
        assert_eq!(decoded, input.envelope);
    }
}
