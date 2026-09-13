//! Rule: a ledger entry the transaction needed was archived.
//!
//! **Fires on:** the operation result `EntryArchived` (stage `state_archival`).
//! The protocol defines that code for exactly this condition, so the result
//! code alone states the cause, and the rule reports it as confirmed.
//!
//! **Does not fire on:** any other result, including storage errors raised in
//! diagnostic events. An access outside the footprint is a different failure
//! with a different fix ([`super::FootprintEntryMissing`]).
//!
//! **Cannot say:** which entry was archived. The result code does not carry it,
//! and the rule does not guess.
//!
//! **Fixtures:** none real yet — validated only by synthetic tests. Tracked in
//! issue #1.

use crate::diagnosis::{CandidateCause, Confidence, Evidence, EvidenceSource};
use crate::rule::{FailureContext, Rule, RuleOutcome};
use crate::taxonomy::{CauseClass, FailureStage};

/// See the [module documentation](self).
pub struct ArchivedEntry;

impl Rule for ArchivedEntry {
    fn id(&self) -> &'static str {
        "archived_entry"
    }

    fn description(&self) -> &'static str {
        "A required ledger entry was archived and not restored (result code EntryArchived)"
    }

    fn evaluate(&self, ctx: &FailureContext<'_>) -> RuleOutcome {
        let op = match super::result_code_gate(
            ctx.model,
            FailureStage::StateArchival,
            "EntryArchived",
        ) {
            Ok(e) => e,
            Err(outcome) => return outcome,
        };

        let mut evidence = vec![op];
        if let Some(soroban) = &ctx.model.soroban {
            let marked = &soroban.declared.archived_entry_indexes;
            evidence.push(Evidence::new(
                EvidenceSource::SorobanResources,
                if marked.is_empty() {
                    "the transaction did not mark any footprint entry for automatic restoration"
                        .to_string()
                } else {
                    format!(
                        "the transaction marked read-write footprint entries {marked:?} for \
                         automatic restoration"
                    )
                },
            ));
        }

        RuleOutcome::Match(CandidateCause {
            class: CauseClass::ArchivedEntryRequiresRestore,
            confidence: Confidence::Confirmed,
            summary: "A ledger entry the transaction needed was archived and had not been \
                      restored. The result code does not identify which entry."
                .into(),
            evidence,
            remediation: Some(
                "Re-simulate the transaction. Simulation reports archived entries and can list \
                 them in the transaction's resource extension for automatic restoration \
                 (protocol 23+); alternatively restore them first with a RestoreFootprint \
                 operation."
                    .into(),
            ),
            rule_id: self.id().into(),
        })
    }
}

#[cfg(test)]
mod tests {
    //! Synthetic: no real `EntryArchived` fixture exists yet (issue #1).

    use super::*;
    use crate::testutil::{analyze_ctx, soroban_failure};
    use stellar_xdr::InvokeHostFunctionResult as R;

    #[test]
    fn entry_archived_is_confirmed() {
        let input = soroban_failure(R::EntryArchived, Vec::new(), None);
        let outcome = analyze_ctx(&input, |ctx| ArchivedEntry.evaluate(ctx));
        let c = outcome.candidate().expect("should match");
        assert_eq!(c.class, CauseClass::ArchivedEntryRequiresRestore);
        assert_eq!(c.confidence, Confidence::Confirmed);
        assert!(c.summary.contains("does not identify which entry"));
    }

    #[test]
    fn a_trap_is_not_an_archived_entry() {
        let input = soroban_failure(R::Trapped, Vec::new(), None);
        assert!(matches!(
            analyze_ctx(&input, |ctx| ArchivedEntry.evaluate(ctx)),
            RuleOutcome::NotApplicable { .. }
        ));
    }
}
