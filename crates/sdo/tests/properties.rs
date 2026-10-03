//! Property tests over the analysis engine, using the real fixture corpus.
//!
//! Each property takes a real, recorded transaction and removes an arbitrary
//! subset of its diagnostic events. Real evidence is recorded as a whole, so
//! partial evidence is the situation an analyser meets when a node or RPC
//! provider drops events. These tests check that the engine stays safe there:
//! it never panics, it is deterministic, every claim it makes is backed by
//! evidence that exists, and it never claims certainty it cannot show.
//!
//! Offline only, like the rest of the integration suite.

mod common;

use common::*;
use proptest::prelude::*;
use soroban_failure_analysis::{analyze, AnalysisInput, Confidence, TransactionModel};

/// The real fixtures the properties run over: every Soroban and classic one.
const FIXTURES: &[&str] = &[
    FEE_BUMP_24,
    FEE_BUMP_49,
    FEE_BUMP_49_ALT,
    AUTH_EXPIRED,
    AUTH_NONCE,
    TESTNET_TRAP,
    TESTNET_MISSING_AUTH,
    CLASSIC,
];

/// Every rule id the engine can emit. A candidate citing any other id is a bug.
const RULE_IDS: &[&str] = &[
    "archived_entry",
    "resource_limit_exceeded",
    "insufficient_resource_fee",
    "contract_defined_error",
    "invalid_authorization_entry",
    "footprint_entry_missing",
    "contract_trap",
    "missing_authorization_entry",
];

/// Keep only the events whose bit is set in `mask`. Events beyond bit 127 are
/// always kept, so no fixture is ever truncated by accident.
fn subset(input: &AnalysisInput, mask: u128) -> AnalysisInput {
    let mut out = input.clone();
    let mut i = 0usize;
    out.diagnostic_events.retain(|_| {
        let keep = if i < 128 { (mask >> i) & 1 == 1 } else { true };
        i += 1;
        keep
    });
    out
}

/// Index of the terminal `host_fn_failed` event in the full, unmodified
/// fixture, if it has one.
fn terminal_index(input: &AnalysisInput) -> Option<u32> {
    TransactionModel::from_input(input)
        .diagnostics
        .terminal_error
        .map(|t| t.event_index)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn analysis_never_panics_on_any_subset_of_events(mask in any::<u128>()) {
        for name in FIXTURES {
            let input = subset(&input(name), mask);
            let _ = analyze(&input);
        }
    }

    #[test]
    fn analysis_is_deterministic_on_any_subset_of_events(mask in any::<u128>()) {
        for name in FIXTURES {
            let input = subset(&input(name), mask);
            prop_assert_eq!(analyze(&input), analyze(&input), "{}", name);
        }
    }

    #[test]
    fn every_candidate_cites_a_known_rule_and_evidence_that_exists(mask in any::<u128>()) {
        for name in FIXTURES {
            let full = input(name);
            let original = full.diagnostic_events.len() as u32;
            let d = analyze(&subset(&full, mask));

            for c in &d.candidate_causes {
                prop_assert!(RULE_IDS.contains(&c.rule_id.as_str()), "{}: unknown rule {}", name, c.rule_id);
                prop_assert!(!c.evidence.is_empty(), "{}: {} has no evidence", name, c.rule_id);
                for e in &c.evidence {
                    if let soroban_failure_analysis::EvidenceSource::DiagnosticEvent { index } = e.source {
                        prop_assert!(
                            index < original,
                            "{}: {} cites event {} but the transaction had {}",
                            name, c.rule_id, index, original
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn no_confirmed_cause_survives_removing_the_terminal_error(mask in any::<u128>()) {
        // Confirmed claims are the strongest the engine makes. Each one rests on
        // the error that ended the invocation, so with that event removed the
        // engine must not still assert certainty.
        for name in FIXTURES {
            let full = input(name);
            let Some(terminal) = terminal_index(&full) else { continue };

            // Keep the same events as `mask` but force the terminal one out.
            let mask = mask & !(1u128 << terminal);
            let d = analyze(&subset(&full, mask));

            for c in &d.candidate_causes {
                prop_assert!(
                    c.confidence != Confidence::Confirmed,
                    "{}: {} is Confirmed although the terminal error event was removed",
                    name, c.rule_id
                );
            }
        }
    }
}
