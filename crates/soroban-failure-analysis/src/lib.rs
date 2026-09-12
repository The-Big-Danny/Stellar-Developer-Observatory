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
//! **No failure rules are implemented yet.** [`analyze`] runs an empty registry
//! and returns a [`Diagnosis`] with a stage and contract error names, but no
//! candidate causes, and a limitation saying so. Rules are milestone M4; see
//! `ROADMAP.md`.
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
pub mod taxonomy;

#[cfg(test)]
pub(crate) mod testutil;

pub use diagnosis::{CandidateCause, Confidence, Diagnosis, Evidence, EvidenceSource};
pub use input::{AnalysisInput, AnalysisInputBuilder};
pub use model::TransactionModel;
pub use rule::{FailureContext, Rule, RuleRegistry};
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

    let mut candidate_causes: Vec<CandidateCause> = registry
        .iter()
        .filter_map(|rule| rule.evaluate(&ctx))
        .collect();

    // Strongest confidence first. `sort_by_key` is stable, so equal-confidence
    // candidates retain registration order and output stays deterministic.
    candidate_causes.sort_by_key(|c| core::cmp::Reverse(c.confidence));

    let mut limitations = Vec::new();

    if registry.is_empty() {
        limitations.push(
            "No failure rules are implemented yet (milestone M4). This build can \
             decode and structure a transaction but cannot yet attribute a cause."
                .to_string(),
        );
    }

    if stage.is_none() {
        limitations.push("The transaction succeeded; there is no failure to analyse.".to_string());
    }

    let unnamed = contract_errors
        .iter()
        .filter(|r| r.resolution.name().is_none())
        .count();
    if unnamed > 0 {
        limitations.push(format!(
            "{unnamed} contract error code(s) could not be named; each report in              `contract_errors` states why."
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
    fn analyze_is_undetermined_with_no_rules() {
        let diagnosis = analyze(&minimal_input());
        assert!(diagnosis.is_undetermined());
        assert_eq!(diagnosis.rules_evaluated, 0);
        // TxFailed with no failing operation: failed, but the stage is not
        // determinable. Not `None`, which would claim success.
        assert_eq!(diagnosis.stage, Some(FailureStage::Unknown));
    }

    #[test]
    fn analyze_states_its_limitations_rather_than_degrading_silently() {
        let diagnosis = analyze(&minimal_input());
        assert!(
            diagnosis.limitations.iter().any(|l| l.contains("M4")),
            "must disclose that no rules exist yet"
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
