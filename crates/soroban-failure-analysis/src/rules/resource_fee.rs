//! Rule: the declared Soroban resource fee was insufficient.
//!
//! **Fires on:** the operation result `InsufficientRefundableFee` (stage
//! `resource_fee`). The result code states the cause directly.
//!
//! **Does not fire on:** `TxInsufficientFee` (stage `fee`). That is a
//! *transaction-level* failure to pay the inclusion fee, rejected before any
//! operation ran. It is a different failure with a different fix, and the rule
//! says so explicitly rather than just declining.
//!
//! **Fixtures:** none real yet — validated only by synthetic tests. Tracked in
//! issue #1.

use crate::diagnosis::{CandidateCause, Confidence, Evidence, EvidenceSource};
use crate::rule::{FailureContext, Rule, RuleOutcome};
use crate::taxonomy::{CauseClass, FailureStage};

/// See the [module documentation](self).
pub struct InsufficientResourceFee;

impl Rule for InsufficientResourceFee {
    fn id(&self) -> &'static str {
        "insufficient_resource_fee"
    }

    fn description(&self) -> &'static str {
        "The declared resource fee did not cover refundable charges (InsufficientRefundableFee)"
    }

    fn evaluate(&self, ctx: &FailureContext<'_>) -> RuleOutcome {
        if ctx.model.stage() == Some(FailureStage::Fee) {
            return RuleOutcome::not_applicable(
                "the transaction was rejected for its inclusion fee — a transaction-level fee \
                 failure, not a Soroban resource fee failure",
            );
        }
        let op = match super::result_code_gate(
            ctx.model,
            FailureStage::ResourceFee,
            "InsufficientRefundableFee",
        ) {
            Ok(e) => e,
            Err(outcome) => return outcome,
        };

        let mut evidence = vec![op];
        if let Some(soroban) = &ctx.model.soroban {
            evidence.push(Evidence::new(
                EvidenceSource::SorobanResources,
                format!(
                    "declared maximum resource fee: {} stroops",
                    soroban.declared.resource_fee
                ),
            ));
        }
        if let Some(f) = ctx.model.observed.fees_charged {
            evidence.push(Evidence::new(
                EvidenceSource::ObservedResources,
                format!(
                    "charged: {} non-refundable, {} refundable, {} rent (stroops)",
                    f.non_refundable, f.refundable, f.rent
                ),
            ));
        }

        RuleOutcome::Match(CandidateCause {
            class: CauseClass::InsufficientResourceFee,
            confidence: Confidence::Confirmed,
            summary: "The declared resource fee did not cover the transaction's refundable \
                      resource charges."
                .into(),
            evidence,
            remediation: Some(
                "Re-simulate to get a current resource fee and raise the declared resource fee. \
                 Refundable charges such as rent and event bytes can grow between simulation \
                 and execution, so leave headroom."
                    .into(),
            ),
            rule_id: self.id().into(),
        })
    }
}

#[cfg(test)]
mod tests {
    //! Synthetic: no real `InsufficientRefundableFee` fixture exists yet (issue #1).

    use super::*;
    use crate::testutil::{analyze_ctx, soroban_failure, tx_level_failure};
    use stellar_xdr::{InvokeHostFunctionResult as R, TransactionResultResult};

    #[test]
    fn insufficient_refundable_fee_is_confirmed() {
        let input = soroban_failure(R::InsufficientRefundableFee, Vec::new(), None);
        let outcome = analyze_ctx(&input, |ctx| InsufficientResourceFee.evaluate(ctx));
        let c = outcome.candidate().expect("should match");
        assert_eq!(c.class, CauseClass::InsufficientResourceFee);
        assert_eq!(c.confidence, Confidence::Confirmed);
    }

    #[test]
    fn transaction_level_insufficient_fee_is_explicitly_not_a_resource_fee_failure() {
        let input = tx_level_failure(TransactionResultResult::TxInsufficientFee);
        match analyze_ctx(&input, |ctx| InsufficientResourceFee.evaluate(ctx)) {
            RuleOutcome::NotApplicable { reason } => {
                assert!(reason.contains("inclusion fee"), "{reason}")
            }
            other => panic!("expected NotApplicable, got {other:?}"),
        }
    }
}
