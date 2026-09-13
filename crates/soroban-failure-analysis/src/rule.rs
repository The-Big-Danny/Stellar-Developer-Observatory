//! The rule interface.
//!
//! A *rule* looks at one failed transaction and decides whether it can explain
//! it. Rules are the unit of contribution in this project: adding support for a
//! new failure mode should mean adding one rule file, one fixture, and one test
//! — nothing else. The built-in rules live in [`crate::rules`].
//!
//! Rules must be:
//!
//! * **Pure.** No I/O, no clock, no randomness, no global state.
//! * **Independent.** A rule never inspects or depends on another rule's output.
//! * **Honest.** A rule says *why* it did not match. [`RuleOutcome`] has three
//!   states so that "this rule does not concern this transaction" is never
//!   confused with "this rule concerns it, but the evidence is missing".

use crate::contract::ContractErrorReport;
use crate::diagnosis::CandidateCause;
use crate::input::AnalysisInput;
use crate::model::TransactionModel;
use crate::taxonomy::FailureStage;

/// What a rule is given when it is asked to explain a failure.
#[derive(Debug, Clone, Copy)]
pub struct FailureContext<'a> {
    /// The decoded transaction artifacts, verbatim.
    pub input: &'a AnalysisInput,
    /// The canonical model. Prefer this over `input`: fee bumps are unwrapped,
    /// events classified, and declared and observed resources kept separate,
    /// so a rule need not understand raw XDR layout.
    pub model: &'a TransactionModel,
    /// The stage observed from the result, or `Unknown` if it could not be
    /// determined.
    pub stage: FailureStage,
    /// Contract errors seen in the diagnostic events, with any names resolved
    /// from contract specs.
    pub contract_errors: &'a [ContractErrorReport],
}

/// What a rule concluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleOutcome {
    /// The rule does not concern this transaction — for example a Soroban rule
    /// on a classic transaction, or any rule on a successful one.
    NotApplicable {
        /// Why, in one sentence.
        reason: String,
    },
    /// The rule could concern this transaction, but the evidence it needs is
    /// absent or inconclusive. This is "unknown", stated honestly.
    NoEvidence {
        /// What was missing, in one sentence.
        reason: String,
    },
    /// The evidence supports a candidate cause.
    Match(CandidateCause),
}

impl RuleOutcome {
    /// Shorthand for [`RuleOutcome::NotApplicable`].
    pub fn not_applicable(reason: impl Into<String>) -> Self {
        Self::NotApplicable {
            reason: reason.into(),
        }
    }

    /// Shorthand for [`RuleOutcome::NoEvidence`].
    pub fn no_evidence(reason: impl Into<String>) -> Self {
        Self::NoEvidence {
            reason: reason.into(),
        }
    }

    /// The candidate cause, if the rule matched.
    pub fn candidate(&self) -> Option<&CandidateCause> {
        match self {
            Self::Match(c) => Some(c),
            _ => None,
        }
    }
}

/// A single explanation strategy.
///
/// Implementations live in `crates/soroban-failure-analysis/src/rules/` and are
/// registered in [`RuleRegistry::builtin`].
pub trait Rule: Send + Sync {
    /// Stable identifier, e.g. `"footprint_entry_missing"`.
    ///
    /// Appears in output as `CandidateCause::rule_id` and must not change once
    /// released — consumers and tests match on it.
    fn id(&self) -> &'static str;

    /// One-line description of what this rule detects.
    fn description(&self) -> &'static str;

    /// Decide whether this rule explains the failure.
    fn evaluate(&self, ctx: &FailureContext<'_>) -> RuleOutcome;
}

/// The set of rules an analysis run will evaluate.
#[derive(Default)]
pub struct RuleRegistry {
    rules: Vec<Box<dyn Rule>>,
}

impl RuleRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// The rules shipped with this crate, in evaluation order.
    ///
    /// Order only breaks ties between candidates of equal confidence; see
    /// [`crate::analyze_with`]. What each rule requires, and which categories
    /// have no rule yet, is documented in `docs/architecture/rules.md`.
    pub fn builtin() -> Self {
        let mut r = Self::new();
        r.register(Box::new(crate::rules::ArchivedEntry))
            .register(Box::new(crate::rules::ResourceLimitExceeded))
            .register(Box::new(crate::rules::InsufficientResourceFee))
            .register(Box::new(crate::rules::ContractDefinedError))
            .register(Box::new(crate::rules::InvalidAuthorizationEntry))
            .register(Box::new(crate::rules::FootprintEntryMissing));
        r
    }

    /// Add a rule.
    pub fn register(&mut self, rule: Box<dyn Rule>) -> &mut Self {
        self.rules.push(rule);
        self
    }

    /// Number of registered rules.
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// Whether the registry has no rules.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Iterate the registered rules.
    pub fn iter(&self) -> impl Iterator<Item = &dyn Rule> {
        self.rules.iter().map(|r| r.as_ref())
    }
}

impl core::fmt::Debug for RuleRegistry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RuleRegistry")
            .field(
                "rules",
                &self.rules.iter().map(|r| r.id()).collect::<Vec<_>>(),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnosis::{CandidateCause, Confidence};
    use crate::taxonomy::CauseClass;

    struct AlwaysFires;

    impl Rule for AlwaysFires {
        fn id(&self) -> &'static str {
            "test_always_fires"
        }
        fn description(&self) -> &'static str {
            "test double"
        }
        fn evaluate(&self, _ctx: &FailureContext<'_>) -> RuleOutcome {
            RuleOutcome::Match(CandidateCause {
                class: CauseClass::Undetermined,
                confidence: Confidence::Possible,
                summary: "test".into(),
                evidence: Vec::new(),
                remediation: None,
                rule_id: "test_always_fires".into(),
            })
        }
    }

    #[test]
    fn builtin_registry_ships_exactly_the_m4_rules() {
        // Tripwire. Rule ids are a public contract; adding, removing or
        // renaming one must be a deliberate change to this list.
        let ids: Vec<_> = RuleRegistry::builtin().iter().map(|r| r.id()).collect();
        assert_eq!(
            ids,
            vec![
                "archived_entry",
                "resource_limit_exceeded",
                "insufficient_resource_fee",
                "contract_defined_error",
                "invalid_authorization_entry",
                "footprint_entry_missing",
            ]
        );
    }

    #[test]
    fn registry_registers_and_iterates() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(AlwaysFires));
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.iter().next().unwrap().id(), "test_always_fires");
    }
}
