//! Rule: a Soroban resource limit was exceeded.
//!
//! **Fires on:** the operation result `ResourceLimitExceeded` (stage
//! `resource_limit`), from `InvokeHostFunction`, `ExtendFootprintTtl` or
//! `RestoreFootprint`. The result code states the cause directly.
//!
//! **Cannot say:** *which* limit. The result code does not carry it. The rule
//! attaches declared limits and any values the host reported, as observations,
//! and only calls a value "above the limit" when it strictly is — it never
//! names a dimension as the cause.
//!
//! **Fixtures:** none real yet — validated only by synthetic tests. Tracked in
//! issue #1.

use crate::diagnosis::{CandidateCause, Confidence, Evidence, EvidenceSource};
use crate::rule::{FailureContext, Rule, RuleOutcome};
use crate::taxonomy::{CauseClass, FailureStage};

/// See the [module documentation](self).
pub struct ResourceLimitExceeded;

impl Rule for ResourceLimitExceeded {
    fn id(&self) -> &'static str {
        "resource_limit_exceeded"
    }

    fn description(&self) -> &'static str {
        "A Soroban resource limit was exceeded (result code ResourceLimitExceeded)"
    }

    fn evaluate(&self, ctx: &FailureContext<'_>) -> RuleOutcome {
        let op = match super::result_code_gate(
            ctx.model,
            FailureStage::ResourceLimit,
            "ResourceLimitExceeded",
        ) {
            Ok(e) => e,
            Err(outcome) => return outcome,
        };

        let mut evidence = vec![op];
        if let Some(soroban) = &ctx.model.soroban {
            let d = &soroban.declared;
            evidence.push(Evidence::new(
                EvidenceSource::SorobanResources,
                format!(
                    "declared limits: {} CPU instructions, {} disk read bytes, {} write bytes",
                    d.instructions, d.disk_read_bytes, d.write_bytes
                ),
            ));
            if let Some(cpu) = ctx.model.observed.cpu_instructions() {
                let over = cpu > u64::from(d.instructions);
                evidence.push(Evidence::new(
                    EvidenceSource::ObservedResources,
                    format!(
                        "the host reported {cpu} CPU instructions (`cpu_insn`) against a \
                         declared limit of {}{}",
                        d.instructions,
                        if over { ", above the limit" } else { "" }
                    ),
                ));
            }
        }
        if let Some(mem) = ctx.model.observed.memory_bytes() {
            evidence.push(Evidence::new(
                EvidenceSource::ObservedResources,
                format!(
                    "the host reported {mem} memory bytes (`mem_byte`); memory is limited by \
                     the network, not declared on the transaction"
                ),
            ));
        }

        RuleOutcome::Match(CandidateCause {
            class: CauseClass::ResourceLimitExceeded,
            confidence: Confidence::Confirmed,
            summary: "The transaction exceeded a Soroban resource limit. The result code does \
                      not say which one."
                .into(),
            evidence,
            remediation: Some(
                "Re-simulate to get current resource estimates and declare them with headroom. \
                 Compare the declared limits with any observed values above; if a network-wide \
                 limit (such as memory) was hit, the invocation itself must do less work."
                    .into(),
            ),
            rule_id: self.id().into(),
        })
    }
}

#[cfg(test)]
mod tests {
    //! Synthetic: no real `ResourceLimitExceeded` fixture exists yet (issue #1).

    use super::*;
    use crate::testutil::{analyze_ctx, core_metric, soroban_failure};
    use stellar_xdr::InvokeHostFunctionResult as R;

    #[test]
    fn resource_limit_exceeded_is_confirmed_without_naming_a_dimension() {
        let input = soroban_failure(R::ResourceLimitExceeded, Vec::new(), None);
        let outcome = analyze_ctx(&input, |ctx| ResourceLimitExceeded.evaluate(ctx));
        let c = outcome.candidate().expect("should match");
        assert_eq!(c.class, CauseClass::ResourceLimitExceeded);
        assert_eq!(c.confidence, Confidence::Confirmed);
        assert!(!c
            .evidence
            .iter()
            .any(|e| e.observation.contains("above the limit")));
    }

    #[test]
    fn observed_cpu_above_the_declared_limit_is_stated_as_an_observation() {
        // testutil declares 1000 instructions.
        let input = soroban_failure(
            R::ResourceLimitExceeded,
            Vec::new(),
            Some(vec![core_metric("cpu_insn", 1_500)]),
        );
        let outcome = analyze_ctx(&input, |ctx| ResourceLimitExceeded.evaluate(ctx));
        let c = outcome.candidate().unwrap();
        assert!(c
            .evidence
            .iter()
            .any(|e| e.observation.contains("1500 CPU instructions")
                && e.observation.contains("above the limit")));
        // Still not promoted into a claim about the cause.
        assert!(c.summary.contains("does not say which one"));
    }

    #[test]
    fn a_resource_fee_failure_is_not_a_resource_limit_failure() {
        let input = soroban_failure(R::InsufficientRefundableFee, Vec::new(), None);
        assert!(matches!(
            analyze_ctx(&input, |ctx| ResourceLimitExceeded.evaluate(ctx)),
            RuleOutcome::NotApplicable { .. }
        ));
    }
}
