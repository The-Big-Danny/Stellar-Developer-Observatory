//! The rule interface.
//!
//! A *rule* looks at one failed transaction and decides whether it can explain
//! it. Rules are the unit of contribution in this project: adding support for a
//! new failure mode should mean adding one rule file, one fixture, and one test
//! — nothing else.
//!
//! Rules must be:
//!
//! * **Pure.** No I/O, no clock, no randomness, no global state.
//! * **Independent.** A rule never inspects or depends on another rule's output.
//! * **Honest.** Return `None` rather than a low-confidence guess, and never
//!   conclude anything from absent diagnostic events when
//!   [`AnalysisInput::diagnostics_enabled`] is `false`.
//!
//! No rules ship in this milestone. The registry below is deliberately empty;
//! populating it is milestone M4.
//!
//! [`AnalysisInput::diagnostics_enabled`]: crate::AnalysisInput::diagnostics_enabled

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
}

/// A single explanation strategy.
///
/// Implementations live in `crates/soroban-failure-analysis/src/rules/` and are
/// registered in [`RuleRegistry::builtin`].
pub trait Rule: Send + Sync {
    /// Stable identifier, e.g. `"missing_auth_entry"`.
    ///
    /// Appears in output as `CandidateCause::rule_id` and must not change once
    /// released — consumers and tests match on it.
    fn id(&self) -> &'static str;

    /// One-line description of what this rule detects.
    fn description(&self) -> &'static str;

    /// Decide whether this rule explains the failure.
    ///
    /// Return `None` when it does not apply, or when the evidence needed to
    /// decide is unavailable.
    fn evaluate(&self, ctx: &FailureContext<'_>) -> Option<CandidateCause>;
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

    /// The rules shipped with this crate.
    ///
    /// **Currently empty.** Failure rules are milestone M4; this crate is at M1
    /// and deliberately ships the engine without them rather than shipping
    /// guesses. See `ROADMAP.md`.
    pub fn builtin() -> Self {
        Self::new()
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
        fn evaluate(&self, _ctx: &FailureContext<'_>) -> Option<CandidateCause> {
            Some(CandidateCause {
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
    fn builtin_registry_is_empty_until_m4() {
        // This test is a tripwire. When M4 lands the first rule, update it
        // deliberately -- do not delete it.
        assert!(
            RuleRegistry::builtin().is_empty(),
            "no failure rules ship before M4"
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
