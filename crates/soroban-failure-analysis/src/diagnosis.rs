//! The output type of the analysis engine.
//!
//! Design rule: a [`Diagnosis`] never asserts a single verdict. It carries a
//! *ranked list* of [`CandidateCause`]s, each with a [`Confidence`] and the
//! [`Evidence`] that supports it. Diagnostic events are unmetered and are not
//! part of consensus, so certainty we do not have must never be implied.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::contract::ContractErrorReport;
use crate::taxonomy::{CauseClass, FailureStage};

/// How strongly the evidence supports a candidate cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum Confidence {
    /// Consistent with the evidence, but other causes explain it equally well.
    Possible,
    /// The evidence points here, but a confirming signal is absent.
    Likely,
    /// The result code or diagnostic events state this cause directly.
    Confirmed,
}

impl Confidence {
    /// A stable machine-friendly identifier.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Possible => "possible",
            Self::Likely => "likely",
            Self::Confirmed => "confirmed",
        }
    }
}

/// Where a piece of supporting evidence was found.
///
/// Evidence is a *pointer into the transaction*, not prose. A consumer must be
/// able to follow it back to the exact artifact and check the claim.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "source", rename_all = "snake_case"))]
#[non_exhaustive]
pub enum EvidenceSource {
    /// The `TransactionResult` result code.
    TransactionResult,
    /// The operation result at this index within the transaction result.
    OperationResult {
        /// Zero-based index of the operation.
        index: u32,
    },
    /// A diagnostic event at this index in the diagnostic event list.
    DiagnosticEvent {
        /// Zero-based index into the diagnostic event list.
        index: u32,
    },
    /// A `SorobanAuthorizationEntry` at this index in the envelope.
    AuthorizationEntry {
        /// Zero-based index into the operation's auth entries.
        index: u32,
    },
    /// An entry in the declared read-only or read-write footprint.
    FootprintEntry {
        /// `true` for the read-write footprint, `false` for read-only.
        read_write: bool,
        /// Zero-based index within that footprint list.
        index: u32,
    },
    /// The declared Soroban resources on the transaction.
    SorobanResources,
    /// The contract spec fetched for the invoked contract.
    ContractSpec,
}

/// A single piece of evidence supporting (or qualifying) a candidate cause.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Evidence {
    /// Where in the transaction this evidence was found.
    pub source: EvidenceSource,
    /// A short factual statement of what was observed.
    ///
    /// Must describe an observation, not a conclusion. Good: "auth entry list is
    /// empty". Bad: "the developer forgot to sign".
    pub observation: String,
}

impl Evidence {
    /// Construct a piece of evidence.
    pub fn new(source: EvidenceSource, observation: impl Into<String>) -> Self {
        Self {
            source,
            observation: observation.into(),
        }
    }
}

/// One candidate explanation for the failure.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CandidateCause {
    /// The class of cause being claimed.
    pub class: CauseClass,
    /// How strongly the evidence supports it.
    pub confidence: Confidence,
    /// Human-readable summary of the claim. One sentence.
    pub summary: String,
    /// The evidence supporting the claim. Must not be empty for anything above
    /// [`Confidence::Possible`].
    pub evidence: Vec<Evidence>,
    /// What the developer should check or change next, if known.
    pub remediation: Option<String>,
    /// Identifier of the rule that produced this candidate, for traceability.
    pub rule_id: String,
}

/// The result of analysing a failed transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Diagnosis {
    /// The transaction hash, hex-encoded, if it was supplied.
    pub transaction_hash: Option<String>,
    /// Where execution stopped, as observed from the result with fee bumps
    /// unwrapped. `None` means the transaction succeeded.
    pub stage: Option<FailureStage>,
    /// Candidate causes, ranked most-plausible first.
    pub candidate_causes: Vec<CandidateCause>,
    /// Every contract error code seen in the diagnostic events, with whether
    /// it could be named (M3). Terminal error first.
    ///
    /// This is evidence, not a cause: knowing a contract raised
    /// `NoHarvestablePails` says *what* it reported, and turning that into a
    /// ranked explanation is the job of rules.
    pub contract_errors: Vec<ContractErrorReport>,
    /// Facts the engine could not establish, stated plainly.
    ///
    /// Populated when required inputs were absent — most commonly when
    /// diagnostic events were not returned by the RPC node. Consumers should
    /// surface these; silently degrading is how a diagnostic tool loses trust.
    pub limitations: Vec<String>,
    /// Number of rules that were evaluated to produce this diagnosis.
    pub rules_evaluated: usize,
}

impl Diagnosis {
    /// The highest-ranked candidate cause, if any rule produced one.
    pub fn top_cause(&self) -> Option<&CandidateCause> {
        self.candidate_causes.first()
    }

    /// Whether any cause was identified at all.
    pub fn is_undetermined(&self) -> bool {
        self.candidate_causes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confidence_orders_from_weakest_to_strongest() {
        assert!(Confidence::Possible < Confidence::Likely);
        assert!(Confidence::Likely < Confidence::Confirmed);
    }

    #[test]
    fn empty_diagnosis_is_undetermined_and_has_no_top_cause() {
        let d = Diagnosis {
            transaction_hash: None,
            stage: Some(FailureStage::Unknown),
            candidate_causes: Vec::new(),
            contract_errors: Vec::new(),
            limitations: vec!["no diagnostic events available".into()],
            rules_evaluated: 0,
        };
        assert!(d.is_undetermined());
        assert!(d.top_cause().is_none());
    }
}
