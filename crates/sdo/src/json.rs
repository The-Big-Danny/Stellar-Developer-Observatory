//! Machine-readable rendering of a [`Diagnosis`].
//!
//! This is presentation only, exactly like `render.rs`: it adds no analysis
//! and classifies nothing. It serialises the same [`Diagnosis`] the human
//! report is built from, plus the one value that diagnosis does not store —
//! [`Verdict`], which `Diagnosis::verdict()` computes on demand.

use serde::Serialize;
use soroban_failure_analysis::{Diagnosis, Verdict};

/// The full JSON document produced by `sdo explain --json`.
///
/// `#[serde(flatten)]` merges `Diagnosis`'s own fields (`transaction_hash`,
/// `stage`, `candidate_causes`, `contract_errors`, `rule_reports`,
/// `limitations`, `rules_evaluated`) into the same top-level object as
/// `verdict`, so the document is one flat record rather than a nested
/// wrapper.
#[derive(Serialize)]
struct ExplainOutput<'a> {
    verdict: Verdict,
    #[serde(flatten)]
    diagnosis: &'a Diagnosis,
}

/// Render a diagnosis as a pretty-printed, deterministic JSON document.
///
/// Field order is stable because it falls out of `Diagnosis`'s declaration
/// order plus `serde_json`'s preserve-order-of-struct-fields behaviour; no
/// map with nondeterministic key order is involved.
pub fn report(diagnosis: &Diagnosis) -> String {
    let output = ExplainOutput {
        verdict: diagnosis.verdict(),
        diagnosis,
    };
    // `Diagnosis` and `Verdict` are plain data with no map/float types that
    // could fail to serialise; a failure here would be a bug in this crate,
    // not a runtime condition to handle.
    serde_json::to_string_pretty(&output).expect("Diagnosis and Verdict always serialise")
}
