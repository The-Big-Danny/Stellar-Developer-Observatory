//! Failure taxonomy: *where* a transaction died and *what class* of cause explains it.
//!
//! These two enums are deliberately separate.
//!
//! * [`FailureStage`] is an **observation**. It is derived mechanically from the
//!   transaction result and metadata, and it is either right or wrong — there is
//!   no judgement involved.
//! * [`CauseClass`] is an **interpretation**. It is what a rule concludes from
//!   evidence, and it is always accompanied by a confidence and supporting
//!   evidence (see [`crate::CandidateCause`]).
//!
//! Conflating the two is the mistake that turns a diagnostic tool into a
//! guessing machine, so the type system keeps them apart.
//!
//! The stage variants are grounded in the XDR result types rather than invented:
//! `InvokeHostFunctionResult` has exactly the variants `Success`, `Malformed`,
//! `Trapped`, `ResourceLimitExceeded`, `EntryArchived` and
//! `InsufficientRefundableFee`, and those map onto stages below.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use core::fmt;

/// Where in the transaction pipeline execution stopped.
///
/// This is an observation derived from `TransactionResult` and
/// `TransactionMeta`, not an interpretation. When the available data does not
/// support a determination, [`FailureStage::Unknown`] is correct and must be
/// used — never guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
#[non_exhaustive]
pub enum FailureStage {
    /// The transaction was rejected before any operation ran: malformed
    /// envelope, bad auth on the classic envelope, missing source account.
    Validation,
    /// The sequence number did not match the source account.
    Sequence,
    /// The inclusion fee (not the Soroban resource fee) was insufficient.
    Fee,
    /// A classic (non-Soroban) operation ran and returned a failure code, such
    /// as `payment: underfunded` or `create_claimable_balance: no_trust`.
    ///
    /// Added in M2. Before it existed a classic operation failure could only be
    /// reported as `Unknown`, which was inaccurate: the result *does* identify
    /// the stage, it just is not a Soroban one.
    Operation,
    /// The host function itself was malformed — it never entered contract code.
    HostFunction,
    /// Contract code ran and trapped.
    ContractExecution,
    /// Execution stopped because a required authorization was missing or invalid.
    Auth,
    /// A ledger entry was touched that the declared footprint did not cover.
    Footprint,
    /// A required ledger entry was archived and needed restoring first.
    StateArchival,
    /// A metered resource limit (CPU, memory, read/write bytes, tx size) was exceeded.
    ResourceLimit,
    /// The declared refundable resource fee did not cover actual consumption.
    ResourceFee,
    /// The transaction failed but the available data does not identify a stage.
    ///
    /// This is a legitimate, expected outcome — not an error. Diagnostic events
    /// are not part of consensus and may simply be absent.
    Unknown,
}

impl FailureStage {
    /// A short, stable, machine-friendly identifier.
    ///
    /// Stable across releases: downstream consumers may match on these strings.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Validation => "validation",
            Self::Sequence => "sequence",
            Self::Fee => "fee",
            Self::Operation => "operation",
            Self::HostFunction => "host_function",
            Self::ContractExecution => "contract_execution",
            Self::Auth => "auth",
            Self::Footprint => "footprint",
            Self::StateArchival => "state_archival",
            Self::ResourceLimit => "resource_limit",
            Self::ResourceFee => "resource_fee",
            Self::Unknown => "unknown",
        }
    }

    /// A one-line human description, for presentation.
    pub const fn description(self) -> &'static str {
        match self {
            Self::Validation => "rejected before any operation ran",
            Self::Sequence => "sequence number mismatch",
            Self::Fee => "inclusion fee could not be paid",
            Self::Operation => "a classic (non-Soroban) operation failed",
            Self::HostFunction => "Soroban host function was malformed",
            Self::ContractExecution => "Soroban contract execution trapped",
            Self::Auth => "Soroban authorization failed",
            Self::Footprint => "Soroban footprint did not cover an accessed entry",
            Self::StateArchival => "a required ledger entry was archived",
            Self::ResourceLimit => "a Soroban resource limit was exceeded",
            Self::ResourceFee => "the refundable resource fee was insufficient",
            Self::Unknown => "not determinable from the available data",
        }
    }

    /// Whether this stage implies contract code actually began executing.
    pub const fn reached_contract_code(self) -> bool {
        matches!(
            self,
            Self::ContractExecution | Self::Auth | Self::Footprint | Self::StateArchival
        )
    }
}

impl fmt::Display for FailureStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// The class of root cause a rule concluded.
///
/// Unlike [`FailureStage`] this is always a *claim*, and a claim must carry
/// evidence. Never construct a [`CauseClass`] without attaching the evidence
/// that supports it — see [`crate::CandidateCause`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
#[non_exhaustive]
pub enum CauseClass {
    /// A required `SorobanAuthorizationEntry` was absent from the envelope.
    MissingAuthorizationEntry,
    /// An authorization entry was present but did not match what was required.
    InvalidAuthorizationEntry,
    /// A ledger key was accessed that the declared footprint did not include.
    FootprintEntryMissing,
    /// A required ledger entry has been archived and must be restored first.
    ArchivedEntryRequiresRestore,
    /// A metered resource limit was exceeded.
    ResourceLimitExceeded,
    /// The declared refundable fee was too low for actual consumption.
    InsufficientResourceFee,
    /// The contract returned one of its own declared error codes.
    ContractDefinedError,
    /// The contract panicked or trapped without a declared error code.
    ContractTrap,
    /// The host function payload was structurally invalid.
    MalformedHostFunction,
    /// A cause could not be determined from the available evidence.
    Undetermined,
}

impl CauseClass {
    /// A short, stable, machine-friendly identifier.
    pub const fn id(self) -> &'static str {
        match self {
            Self::MissingAuthorizationEntry => "missing_authorization_entry",
            Self::InvalidAuthorizationEntry => "invalid_authorization_entry",
            Self::FootprintEntryMissing => "footprint_entry_missing",
            Self::ArchivedEntryRequiresRestore => "archived_entry_requires_restore",
            Self::ResourceLimitExceeded => "resource_limit_exceeded",
            Self::InsufficientResourceFee => "insufficient_resource_fee",
            Self::ContractDefinedError => "contract_defined_error",
            Self::ContractTrap => "contract_trap",
            Self::MalformedHostFunction => "malformed_host_function",
            Self::Undetermined => "undetermined",
        }
    }

    /// The stage this cause class is normally observed at.
    ///
    /// Advisory only — used for ordering and presentation, never to *infer* a
    /// stage. The stage is observed, not derived from a guess.
    pub const fn typical_stage(self) -> FailureStage {
        match self {
            Self::MissingAuthorizationEntry | Self::InvalidAuthorizationEntry => FailureStage::Auth,
            Self::FootprintEntryMissing => FailureStage::Footprint,
            Self::ArchivedEntryRequiresRestore => FailureStage::StateArchival,
            Self::ResourceLimitExceeded => FailureStage::ResourceLimit,
            Self::InsufficientResourceFee => FailureStage::ResourceFee,
            Self::ContractDefinedError | Self::ContractTrap => FailureStage::ContractExecution,
            Self::MalformedHostFunction => FailureStage::HostFunction,
            Self::Undetermined => FailureStage::Unknown,
        }
    }
}

impl fmt::Display for CauseClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_ids_are_unique() {
        let all = [
            FailureStage::Validation,
            FailureStage::Sequence,
            FailureStage::Fee,
            FailureStage::Operation,
            FailureStage::HostFunction,
            FailureStage::ContractExecution,
            FailureStage::Auth,
            FailureStage::Footprint,
            FailureStage::StateArchival,
            FailureStage::ResourceLimit,
            FailureStage::ResourceFee,
            FailureStage::Unknown,
        ];
        let mut ids: Vec<_> = all.iter().map(|s| s.id()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "FailureStage ids must be unique");
    }

    #[test]
    fn cause_ids_are_unique() {
        let all = [
            CauseClass::MissingAuthorizationEntry,
            CauseClass::InvalidAuthorizationEntry,
            CauseClass::FootprintEntryMissing,
            CauseClass::ArchivedEntryRequiresRestore,
            CauseClass::ResourceLimitExceeded,
            CauseClass::InsufficientResourceFee,
            CauseClass::ContractDefinedError,
            CauseClass::ContractTrap,
            CauseClass::MalformedHostFunction,
            CauseClass::Undetermined,
        ];
        let mut ids: Vec<_> = all.iter().map(|c| c.id()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "CauseClass ids must be unique");
    }

    #[test]
    fn only_contract_reaching_stages_report_reached_code() {
        assert!(FailureStage::Auth.reached_contract_code());
        assert!(FailureStage::ContractExecution.reached_contract_code());
        assert!(!FailureStage::Validation.reached_contract_code());
        assert!(!FailureStage::Unknown.reached_contract_code());
        assert!(!FailureStage::Operation.reached_contract_code());
    }
}
