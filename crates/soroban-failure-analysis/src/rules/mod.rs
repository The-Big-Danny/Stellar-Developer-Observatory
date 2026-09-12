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
