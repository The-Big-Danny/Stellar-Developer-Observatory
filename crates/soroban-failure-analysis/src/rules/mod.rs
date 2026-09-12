//! The built-in failure rules (milestone M4).
//!
//! One rule per file. Each states, in its module docs, exactly which evidence
//! makes it fire, which evidence stops it, and which fixtures prove it. See
//! `docs/architecture/rules.md` for the overview, including the categories that
//! deliberately have no rule yet.
//!
//! | Rule | Cause | Evidence |
//! |---|---|---|
//! | [`ArchivedEntry`] | `ArchivedEntryRequiresRestore` | result code `EntryArchived` |
//! | [`ResourceLimitExceeded`] | `ResourceLimitExceeded` | result code `ResourceLimitExceeded` |
//! | [`InsufficientResourceFee`] | `InsufficientResourceFee` | result code `InsufficientRefundableFee` |
//! | [`ContractDefinedError`] | `ContractDefinedError` | terminal `Error(Contract, #N)` + M3 resolution |
//! | [`InvalidAuthorizationEntry`] | `InvalidAuthorizationEntry` | host auth error: expired signature or reused nonce |
//! | [`FootprintEntryMissing`] | `FootprintEntryMissing` | host footprint error event + key absent from footprint |
//!
//! *Missing* authorization (`MissingAuthorizationEntry`) has no rule: the
//! fixture corpus contains no example, so its evidence shape is unknown.

// Public so each rule's documented contract — what triggers it, what prevents
// it, its confidence table and its fixtures — is published with the crate.
pub mod archived;
pub mod auth;
pub mod contract_error;
pub mod footprint;
pub mod resource_fee;
pub mod resource_limit;

pub use archived::ArchivedEntry;
pub use auth::{InvalidAuthorizationEntry, NONCE_REUSED_MARKER, SIGNATURE_EXPIRED_MARKER};
pub use contract_error::ContractDefinedError;
pub use footprint::{FootprintEntryMissing, FOOTPRINT_MARKER};
pub use resource_fee::InsufficientResourceFee;
pub use resource_limit::ResourceLimitExceeded;

use crate::diagnosis::{Evidence, EvidenceSource};
use crate::model::TransactionModel;
use crate::rule::RuleOutcome;
use crate::taxonomy::FailureStage;

/// Lowercase hex.
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The common gate for rules that fire on one specific result code.
///
/// Passes only when **both** the stage is `wanted` **and** the failing
/// operation's result code is `result_code`. A stage mismatch is
/// `NotApplicable`; a stage that disagrees with the code is `NoEvidence`.
/// Neither can ever produce a match.
///
/// Returns the evidence pointing at the failing operation, or the outcome to
/// return when the rule does not apply.
pub(crate) fn result_code_gate(
    model: &TransactionModel,
    wanted: FailureStage,
    result_code: &str,
) -> Result<Evidence, RuleOutcome> {
    let Some(stage) = model.stage() else {
        return Err(RuleOutcome::not_applicable("the transaction succeeded"));
    };
    if stage != wanted {
        return Err(RuleOutcome::not_applicable(format!(
            "the transaction failed at stage `{stage}`; this rule only applies to \
             `{wanted}`, which only the `{result_code}` result code produces"
        )));
    }
    let Some(op) = &model.outcome.failed_operation else {
        // Unreachable for a stage derived from an operation result, but a
        // result that disagrees with itself must not produce a confirmed cause.
        return Err(RuleOutcome::no_evidence(
            "the stage implies a failed operation, but no failing operation result was found",
        ));
    };
    // The stage is derived from this same operation result, so today the two
    // always agree. But these rules are *confirmed by the code*, not by the
    // stage: if a future stage mapping ever routed another code to `wanted`,
    // the rule must decline rather than confirm a cause it was never written
    // for. The XDR names these codes identically for `InvokeHostFunction`,
    // `ExtendFootprintTtl` and `RestoreFootprint`, so one comparison covers all.
    if op.code != result_code {
        return Err(RuleOutcome::no_evidence(format!(
            "the stage is `{wanted}`, but the failing operation returned `{}`, not \
             `{result_code}`",
            op.code
        )));
    }
    Ok(Evidence::new(
        EvidenceSource::OperationResult { index: op.index },
        format!(
            "operation {} ({}) returned `{}`",
            op.index,
            op.operation.unwrap_or("unknown type"),
            op.code
        ),
    ))
}

#[cfg(test)]
mod tests {
    //! Audit tests for the result-code gate shared by the three rules that are
    //! validated only synthetically. They pin down that a match requires the
    //! stage *and* the code to agree, on every failure shape.

    use super::*;
    use crate::rule::{FailureContext, Rule};
    use crate::testutil::{analyze_ctx, minimal_input, soroban_failure, tx_level_failure};
    use stellar_xdr::{InvokeHostFunctionResult as R, TransactionResultResult as T};

    fn gated() -> [(&'static dyn Rule, FailureStage, &'static str); 3] {
        [
            (&ArchivedEntry, FailureStage::StateArchival, "EntryArchived"),
            (
                &ResourceLimitExceeded,
                FailureStage::ResourceLimit,
                "ResourceLimitExceeded",
            ),
            (
                &InsufficientResourceFee,
                FailureStage::ResourceFee,
                "InsufficientRefundableFee",
            ),
        ]
    }

    #[test]
    fn each_gated_rule_matches_its_own_code_and_nothing_else() {
        let failures = [
            R::Malformed,
            R::Trapped,
            R::ResourceLimitExceeded,
            R::EntryArchived,
            R::InsufficientRefundableFee,
        ];
        for r in failures {
            let code = r.name();
            // With and without diagnostic events: these rules must not depend
            // on them to decide.
            for events in [None, Some(Vec::new())] {
                let input = soroban_failure(r.clone(), Vec::new(), events);
                for (rule, _, wanted) in gated() {
                    let outcome = analyze_ctx(&input, |ctx| rule.evaluate(ctx));
                    assert_eq!(
                        outcome.candidate().is_some(),
                        code == wanted,
                        "{} on result `{code}`: {outcome:?}",
                        rule.id()
                    );
                }
            }
        }
    }

    #[test]
    fn transaction_level_classic_and_unknown_failures_are_never_matched() {
        for input in [
            tx_level_failure(T::TxInsufficientFee),
            tx_level_failure(T::TxBadSeq),
            tx_level_failure(T::TxMalformed),
            tx_level_failure(T::TxInternalError),
            minimal_input(),
        ] {
            for (rule, _, _) in gated() {
                assert!(
                    matches!(
                        analyze_ctx(&input, |ctx| rule.evaluate(ctx)),
                        RuleOutcome::NotApplicable { .. }
                    ),
                    "{}",
                    rule.id()
                );
            }
        }
    }

    #[test]
    fn a_stage_that_disagrees_with_the_result_code_never_confirms_a_cause() {
        // A Trapped result whose stage is then forced to each gated stage, as
        // a faulty future stage mapping would produce. The code still says
        // Trapped, so neither the gate nor any rule may match.
        let input = soroban_failure(R::Trapped, Vec::new(), Some(Vec::new()));
        let contract_errors = Vec::new();
        for (rule, stage, wanted) in gated() {
            let mut model = TransactionModel::from_input(&input);
            model.outcome.stage = Some(stage);

            match result_code_gate(&model, stage, wanted) {
                Err(RuleOutcome::NoEvidence { reason }) => {
                    assert!(reason.contains("`Trapped`"), "{reason}")
                }
                other => panic!("{}: expected NoEvidence, got {other:?}", rule.id()),
            }

            let ctx = FailureContext {
                input: &input,
                model: &model,
                stage,
                contract_errors: &contract_errors,
            };
            assert!(
                matches!(rule.evaluate(&ctx), RuleOutcome::NoEvidence { .. }),
                "{} matched a stage/code disagreement",
                rule.id()
            );
        }
    }
}
