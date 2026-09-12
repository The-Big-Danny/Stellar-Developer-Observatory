//! Rule: the invocation ended with the contract's own declared error.
//!
//! Builds on M3 contract error resolution ([`crate::contract`]).
//!
//! **Fires on:** a `host_fn_failed` diagnostic event whose error is
//! `Error(Contract, #N)`. Contracts can only raise the `Contract` error type
//! (the host rejects `fail_with_error` with any other), so the error type is
//! structured evidence that the contract itself chose to fail.
//!
//! **Confidence:**
//!
//! | Evidence | Confidence |
//! |---|---|
//! | contract uniquely identified, name resolved, spec WASM in the footprint | `Confirmed` |
//! | contract uniquely identified, name resolved, spec not verifiable | `Likely` |
//! | contract uniquely identified, name not established | `Likely` |
//! | raising contract not uniquely identified | `Likely` |
//!
//! **Does not fire on:** a terminal host error (`Error(Storage, …)`,
//! `Error(Auth, …)` and so on); contract errors that were raised and caught
//! without ending the invocation — those are reported as evidence, never as the
//! cause; or anything without diagnostic events.
//!
//! **Deliberately does not claim:** *why* the contract raised the error. A
//! resolved name says what the contract reported, not that the contract, the
//! caller or anyone else is at fault.
//!
//! **Fixtures:** `soroban-trapped-feebump-49ev` and `-49ev-alt` (real mainnet),
//! with specs from `fixtures/contracts/`.

use stellar_xdr::ScError;

use crate::contract::{
    ContractIdentification, ErrorResolution, IdentificationBasis, SpecProvenance,
};
use crate::diagnosis::{CandidateCause, Confidence, Evidence, EvidenceSource};
use crate::model::{error_label, CallOutcome, DiagnosticAvailability};
use crate::rule::{FailureContext, Rule, RuleOutcome};
use crate::taxonomy::{CauseClass, FailureStage};

use super::hex;

/// See the [module documentation](self).
pub struct ContractDefinedError;

impl Rule for ContractDefinedError {
    fn id(&self) -> &'static str {
        "contract_defined_error"
    }

    fn description(&self) -> &'static str {
        "The invocation ended with the contract's own error, Error(Contract, #N)"
    }

    fn evaluate(&self, ctx: &FailureContext<'_>) -> RuleOutcome {
        let m = ctx.model;
        let Some(stage) = m.stage() else {
            return RuleOutcome::not_applicable("the transaction succeeded");
        };
        if stage != FailureStage::ContractExecution {
            return RuleOutcome::not_applicable(format!(
                "the transaction failed at stage `{stage}`; a contract-defined error ends \
                 contract execution with a trap"
            ));
        }
        if m.diagnostics.availability == DiagnosticAvailability::NotEmitted {
            return RuleOutcome::no_evidence(
                "diagnostic events were not available, and a contract-defined error is only \
                 visible in them",
            );
        }
        let Some(terminal) = &m.diagnostics.terminal_error else {
            return RuleOutcome::no_evidence(
                "no `host_fn_failed` event states which error ended the invocation",
            );
        };
        let ScError::Contract(code) = terminal.error else {
            return RuleOutcome::not_applicable(format!(
                "the invocation ended with {}, a host error rather than a contract-defined one",
                error_label(&terminal.error)
            ));
        };
        let Some(report) = ctx
            .contract_errors
            .iter()
            .find(|r| r.terminal && r.code == code)
        else {
            return RuleOutcome::no_evidence(format!(
                "no resolution report exists for the terminal error #{code}"
            ));
        };

        let mut evidence = vec![Evidence::new(
            EvidenceSource::DiagnosticEvent {
                index: terminal.event_index,
            },
            format!("`host_fn_failed` reports the invocation ended with Error(Contract, #{code})"),
        )];

        let raised_at = report
            .event_indexes
            .iter()
            .copied()
            .find(|&i| i != terminal.event_index)
            .unwrap_or(terminal.event_index);
        let at = EvidenceSource::DiagnosticEvent { index: raised_at };
        evidence.push(match &report.identification {
            ContractIdentification::Unique { contract, basis } => Evidence::new(
                at,
                format!(
                    "raised by contract {contract}: {}",
                    match basis {
                        IdentificationBasis::SoleEmitter =>
                            "the only contract whose error events carry this code",
                        IdentificationBasis::OriginMarker =>
                            "the frame that emitted the host's origin message; other frames only \
                             re-emitted the code",
                    }
                ),
            ),
            ContractIdentification::Ambiguous { candidates } => Evidence::new(
                at,
                format!(
                    "{} contracts emitted error events carrying this code and none is marked as \
                     its origin: {}",
                    candidates.len(),
                    candidates
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ),
            ContractIdentification::Unidentified => {
                Evidence::new(at, "no error event attributes the code to any contract")
            }
        });

        let spec = EvidenceSource::ContractSpec;
        match &report.resolution {
            ErrorResolution::Resolved {
                contract,
                enum_name,
                case_name,
                doc,
                provenance,
            } => {
                let checked = match provenance {
                    SpecProvenance::MatchesFootprint { wasm_hash } => format!(
                        "; the spec's WASM {} is loaded in this transaction's footprint",
                        hex(wasm_hash)
                    ),
                    SpecProvenance::Unverified => {
                        "; the spec could not be checked against the code that ran".to_string()
                    }
                };
                evidence.push(Evidence::new(
                    spec.clone(),
                    format!("the spec of {contract} declares #{code} as {enum_name}::{case_name}{checked}"),
                ));
                if !doc.is_empty() {
                    evidence.push(Evidence::new(spec, format!("documented as: \"{doc}\"")));
                }
            }
            ErrorResolution::CodeNotInSpec { .. } => evidence.push(Evidence::new(
                spec,
                format!("the contract's spec declares no name for #{code}"),
            )),
            ErrorResolution::AmbiguousInSpec { candidates, .. } => evidence.push(Evidence::new(
                spec,
                format!(
                    "the contract's spec declares #{code} under different names: {}",
                    candidates
                        .iter()
                        .map(|(e, c)| format!("{e}::{c}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )),
            ErrorResolution::SpecUnavailable { reason, .. } => evidence.push(Evidence::new(
                spec,
                format!("the contract's spec was unavailable: {reason}"),
            )),
            ErrorResolution::SpecVersionMismatch { spec_wasm_hash, .. } => {
                evidence.push(Evidence::new(
                    spec,
                    format!(
                        "the available spec is for WASM {}, which this transaction did not load, \
                         so its names are not used",
                        hex(spec_wasm_hash)
                    ),
                ))
            }
            ErrorResolution::ContractNotIdentified | ErrorResolution::NotApplicable => {}
        }

        for other in ctx.contract_errors.iter().filter(|r| !r.terminal) {
            let who = other
                .identification
                .contract()
                .map(|c| format!(" by {c}"))
                .unwrap_or_default();
            let name = other
                .resolution
                .name()
                .map(|n| format!(" ({n})"))
                .unwrap_or_default();
            // Count failed calls from the call tree rather than events: one
            // failed call emits several error events carrying the same code.
            let failed_calls = m
                .diagnostics
                .calls
                .iter()
                .filter(|f| {
                    matches!(&f.outcome, CallOutcome::Failed { error: ScError::Contract(c), .. } if *c == other.code)
                        && other.identification.contract() == Some(&f.contract)
                })
                .count();
            if let Some(&first) = other.event_indexes.first() {
                let observation = if failed_calls > 0 {
                    format!(
                        "earlier, {failed_calls} call(s){who} failed with Error(Contract, #{}){name} \
                         without ending the invocation",
                        other.code
                    )
                } else {
                    format!(
                        "earlier, Error(Contract, #{}){name} was raised{who} without ending the \
                         invocation ({} diagnostic events carry it)",
                        other.code,
                        other.event_indexes.len()
                    )
                };
                evidence.push(Evidence::new(
                    EvidenceSource::DiagnosticEvent { index: first },
                    observation,
                ));
            }
        }

        let contract = report.identification.contract();
        let (confidence, summary, remediation) = match (contract, &report.resolution) {
            (
                Some(c),
                ErrorResolution::Resolved {
                    enum_name,
                    case_name,
                    doc,
                    provenance,
                    ..
                },
            ) => {
                let verified = matches!(provenance, SpecProvenance::MatchesFootprint { .. });
                let summary = format!(
                    "Contract {c} ended the invocation with its declared error \
                     {enum_name}::{case_name} (Error(Contract, #{code})).{}",
                    if verified {
                        ""
                    } else {
                        " The spec was not verified against the code that ran."
                    }
                );
                let remediation = if doc.is_empty() {
                    format!(
                        "Find where contract {c} raises {enum_name}::{case_name} in its source, \
                         and check that condition against this invocation's arguments and the \
                         contract's state at the time."
                    )
                } else {
                    format!(
                        "The contract documents {case_name} as \"{doc}\". Check whether that \
                         condition held for this invocation's arguments and the contract's \
                         state at the time."
                    )
                };
                let confidence = if verified {
                    Confidence::Confirmed
                } else {
                    Confidence::Likely
                };
                (confidence, summary, remediation)
            }
            (Some(c), _) => (
                Confidence::Likely,
                format!(
                    "Contract {c} ended the invocation with contract-defined error #{code}, whose \
                     name could not be established."
                ),
                format!(
                    "Name error #{code} from contract {c}'s spec (online analysis fetches it) or \
                     its source, then check the condition that raises it."
                ),
            ),
            (None, _) => (
                Confidence::Likely,
                format!(
                    "The invocation ended with contract-defined error #{code}, but the contract \
                     that raised it could not be identified uniquely."
                ),
                "Inspect the call trace to see which contract raised the error, then look the \
                 code up in that contract's spec or source."
                    .to_string(),
            ),
        };

        RuleOutcome::Match(CandidateCause {
            class: CauseClass::ContractDefinedError,
            confidence,
            summary,
            evidence,
            remediation: Some(remediation),
            rule_id: self.id().into(),
        })
    }
}

#[cfg(test)]
mod tests {
    //! Synthetic. Real resolution against mainnet fixtures is tested in
    //! `crates/sdo/tests/classification.rs`.

    use std::collections::BTreeMap;

    use super::*;
    use crate::contract::{ContractSpec, SpecAvailability, ORIGIN_MARKER};
    use crate::testutil::{
        analyze_ctx, call, cid, code_key, err, error_enum, host_fn_failed, soroban_failure,
    };
    use stellar_xdr::{InvokeHostFunctionResult as R, ScErrorCode};

    fn contract_failure(extra: Vec<stellar_xdr::DiagnosticEvent>) -> crate::AnalysisInput {
        let mut events = vec![call(None, cid(5), "f")];
        events.extend(extra);
        soroban_failure(R::Trapped, vec![code_key([9; 32])], Some(events))
    }

    fn spec(hash: [u8; 32]) -> BTreeMap<stellar_xdr::ContractId, SpecAvailability> {
        BTreeMap::from([(
            cid(5),
            SpecAvailability::Available(
                ContractSpec::from_entries([error_enum("Error", &[("Unauthorized", 3)])])
                    .with_wasm_hash(hash),
            ),
        )])
    }

    fn raised(code: u32) -> Vec<stellar_xdr::DiagnosticEvent> {
        vec![
            err(cid(5), ScError::Contract(code), ORIGIN_MARKER),
            host_fn_failed(ScError::Contract(code)),
        ]
    }

    #[test]
    fn resolved_and_footprint_verified_is_confirmed() {
        let input = contract_failure(raised(3)).with_contract_specs(spec([9; 32]));
        let c = analyze_ctx(&input, |ctx| ContractDefinedError.evaluate(ctx))
            .candidate()
            .cloned()
            .expect("should match");
        assert_eq!(c.class, CauseClass::ContractDefinedError);
        assert_eq!(c.confidence, Confidence::Confirmed);
        assert!(c.summary.contains("Error::Unauthorized"), "{}", c.summary);
        assert!(c
            .evidence
            .iter()
            .any(|e| e.observation.contains(&"09".repeat(32))));
        // What the contract reported, not a verdict on who is at fault.
        assert!(!c.summary.to_lowercase().contains("bug"));
    }

    #[test]
    fn without_a_spec_the_class_is_likely_and_no_name_is_invented() {
        let input = contract_failure(raised(3));
        let c = analyze_ctx(&input, |ctx| ContractDefinedError.evaluate(ctx))
            .candidate()
            .cloned()
            .unwrap();
        assert_eq!(c.confidence, Confidence::Likely);
        assert!(c.summary.contains("could not be established"));
        assert!(!c.summary.contains("Unauthorized"));
    }

    #[test]
    fn a_spec_for_other_wasm_does_not_produce_a_name_or_confirmation() {
        let input = contract_failure(raised(3)).with_contract_specs(spec([7; 32]));
        let c = analyze_ctx(&input, |ctx| ContractDefinedError.evaluate(ctx))
            .candidate()
            .cloned()
            .unwrap();
        assert_eq!(c.confidence, Confidence::Likely);
        assert!(!c.summary.contains("Unauthorized"));
        assert!(c
            .evidence
            .iter()
            .any(|e| e.observation.contains("did not load")));
    }

    #[test]
    fn an_ambiguous_origin_is_not_confirmed() {
        let input = contract_failure(vec![
            err(cid(5), ScError::Contract(3), "x"),
            err(cid(6), ScError::Contract(3), "y"),
            host_fn_failed(ScError::Contract(3)),
        ])
        .with_contract_specs(spec([9; 32]));
        let c = analyze_ctx(&input, |ctx| ContractDefinedError.evaluate(ctx))
            .candidate()
            .cloned()
            .unwrap();
        assert_eq!(c.confidence, Confidence::Likely);
        assert!(c.summary.contains("could not be identified uniquely"));
    }

    #[test]
    fn a_terminal_host_error_is_not_a_contract_defined_error() {
        let input = contract_failure(vec![host_fn_failed(ScError::Storage(
            ScErrorCode::ExceededLimit,
        ))]);
        assert!(matches!(
            analyze_ctx(&input, |ctx| ContractDefinedError.evaluate(ctx)),
            RuleOutcome::NotApplicable { .. }
        ));
    }

    #[test]
    fn a_caught_contract_error_is_evidence_not_the_cause() {
        // #9 was raised and caught; the invocation then ended with a host error.
        let input = contract_failure(vec![
            err(cid(5), ScError::Contract(9), ORIGIN_MARKER),
            host_fn_failed(ScError::Storage(ScErrorCode::ExceededLimit)),
        ]);
        assert!(matches!(
            analyze_ctx(&input, |ctx| ContractDefinedError.evaluate(ctx)),
            RuleOutcome::NotApplicable { .. }
        ));
    }

    #[test]
    fn a_generic_trap_without_diagnostics_is_no_evidence() {
        let input = soroban_failure(R::Trapped, Vec::new(), None);
        assert!(matches!(
            analyze_ctx(&input, |ctx| ContractDefinedError.evaluate(ctx)),
            RuleOutcome::NoEvidence { .. }
        ));
    }

    #[test]
    fn a_trap_with_no_terminal_event_is_no_evidence() {
        let input = contract_failure(vec![err(cid(5), ScError::Contract(3), ORIGIN_MARKER)]);
        assert!(matches!(
            analyze_ctx(&input, |ctx| ContractDefinedError.evaluate(ctx)),
            RuleOutcome::NoEvidence { .. }
        ));
    }
}
