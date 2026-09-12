//! `TransactionResult` → [`Outcome`] and [`FailureStage`].
//!
//! # Fee bumps
//!
//! A fee-bumped transaction's result is a wrapper. Its outer arm is either
//! `TxFeeBumpInnerSuccess` or `TxFeeBumpInnerFailed`, and **neither says
//! anything about why the inner transaction failed**. The real result sits in
//! the `InnerTransactionResultPair` inside the wrapper.
//!
//! M0 found that every sampled mainnet Soroban failure was fee-bumped, so
//! reading only the outer arm would lose the cause in essentially every real
//! case. [`classify`] always unwraps, and records the wrapper separately in
//! [`FeeBumpResult`] so nothing is lost.
//!
//! # Exhaustive matches, on purpose
//!
//! The result unions are matched exhaustively rather than with a catch-all. When
//! a `stellar-xdr` upgrade adds a result code (28.0.0 added
//! `TxFrozenKeyAccessed`), this module stops compiling and someone has to decide
//! what stage the new code means. A wildcard would silently map it to whatever
//! the wildcard said.

use stellar_xdr::{
    ExtendFootprintTtlResult, InnerTransactionResultResult as Inner, InvokeHostFunctionResult,
    OperationResult, OperationResultTr, RestoreFootprintResult, TransactionResult,
    TransactionResultResult as Outer,
};

use crate::taxonomy::FailureStage;

/// The wrapper recorded when a transaction was fee-bumped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeeBumpResult {
    /// The outer result arm, e.g. `"TxFeeBumpInnerFailed"`. Carries no cause.
    pub outer_code: &'static str,
    /// Hash of the inner transaction.
    pub inner_transaction_hash: [u8; 32],
    /// Fee charged against the inner transaction (normally zero: the fee
    /// source of the bump pays).
    pub inner_fee_charged: i64,
}

/// The operation whose result code made the transaction fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedOperation {
    /// Zero-based index of the operation.
    pub index: u32,
    /// Operation type, as named by the XDR, e.g. `"InvokeHostFunction"`.
    ///
    /// `None` when the failure used a generic wrapper code such as `OpBadAuth`,
    /// which does not say which operation type it was.
    pub operation: Option<&'static str>,
    /// Result code, as named by the XDR, e.g. `"Trapped"` or `"OpBadAuth"`.
    pub code: &'static str,
}

/// What the transaction result says happened, with fee bumps unwrapped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// Fee charged, from the outermost result.
    pub fee_charged: i64,
    /// Present when the result was a fee-bump wrapper.
    pub fee_bump: Option<FeeBumpResult>,
    /// The effective result code — the **inner** one when fee-bumped — as named
    /// by the XDR, e.g. `"TxFailed"`.
    pub result_code: &'static str,
    /// The first operation whose result was not a success, if the transaction
    /// failed at the operation level.
    pub failed_operation: Option<FailedOperation>,
    /// Where the transaction failed. `None` means it **succeeded**.
    pub stage: Option<FailureStage>,
}

impl Outcome {
    /// Whether the transaction succeeded.
    pub fn succeeded(&self) -> bool {
        self.stage.is_none()
    }
}

/// Classify a transaction result.
pub fn classify(result: &TransactionResult) -> Outcome {
    let (fee_bump, level) = match &result.result {
        Outer::TxFeeBumpInnerSuccess(pair) | Outer::TxFeeBumpInnerFailed(pair) => (
            Some(FeeBumpResult {
                outer_code: result.result.name(),
                inner_transaction_hash: pair.transaction_hash.0,
                inner_fee_charged: pair.result.fee_charged,
            }),
            inner_level(&pair.result.result),
        ),
        other => (None, outer_level(other)),
    };

    let (result_code, failed_operation, stage) = match level {
        Level::Success(code) => (code, None, None),
        Level::Failed(code, ops) => {
            let failed = first_failed_operation(ops);
            let stage = failed
                .as_ref()
                .map_or(FailureStage::Unknown, |(_, stage)| *stage);
            (code, failed.map(|(op, _)| op), Some(stage))
        }
        Level::Rejected(code, stage) => (code, None, Some(stage)),
    };

    Outcome {
        fee_charged: result.fee_charged,
        fee_bump,
        result_code,
        failed_operation,
        stage,
    }
}

/// A transaction-level result, after fee-bump unwrapping.
enum Level<'a> {
    Success(&'static str),
    Failed(&'static str, &'a [OperationResult]),
    /// Rejected before or instead of running operations.
    Rejected(&'static str, FailureStage),
}

fn outer_level(r: &Outer) -> Level<'_> {
    use FailureStage as S;
    let code = r.name();
    match r {
        Outer::TxSuccess(_) => Level::Success(code),
        Outer::TxFailed(ops) => Level::Failed(code, ops.as_slice()),
        // Unreachable here: `classify` unwraps these before calling. Kept in the
        // exhaustive match rather than behind a wildcard, see the module docs.
        Outer::TxFeeBumpInnerSuccess(_) | Outer::TxFeeBumpInnerFailed(_) => {
            Level::Rejected(code, S::Unknown)
        }
        Outer::TxTooEarly
        | Outer::TxTooLate
        | Outer::TxMissingOperation
        | Outer::TxBadAuth
        | Outer::TxBadAuthExtra
        | Outer::TxNoAccount
        | Outer::TxNotSupported
        | Outer::TxBadSponsorship
        | Outer::TxMalformed
        | Outer::TxSorobanInvalid => Level::Rejected(code, S::Validation),
        Outer::TxBadSeq | Outer::TxBadMinSeqAgeOrGap => Level::Rejected(code, S::Sequence),
        Outer::TxInsufficientFee | Outer::TxInsufficientBalance => Level::Rejected(code, S::Fee),
        // A core internal error says nothing about the transaction.
        Outer::TxInternalError => Level::Rejected(code, S::Unknown),
        // New in stellar-xdr 28. Its semantics have not been checked against a
        // real transaction, so it is not guessed at.
        Outer::TxFrozenKeyAccessed => Level::Rejected(code, S::Unknown),
    }
}

fn inner_level(r: &Inner) -> Level<'_> {
    use FailureStage as S;
    let code = r.name();
    match r {
        Inner::TxSuccess(_) => Level::Success(code),
        Inner::TxFailed(ops) => Level::Failed(code, ops.as_slice()),
        Inner::TxTooEarly
        | Inner::TxTooLate
        | Inner::TxMissingOperation
        | Inner::TxBadAuth
        | Inner::TxBadAuthExtra
        | Inner::TxNoAccount
        | Inner::TxNotSupported
        | Inner::TxBadSponsorship
        | Inner::TxMalformed
        | Inner::TxSorobanInvalid => Level::Rejected(code, S::Validation),
        Inner::TxBadSeq | Inner::TxBadMinSeqAgeOrGap => Level::Rejected(code, S::Sequence),
        Inner::TxInsufficientFee | Inner::TxInsufficientBalance => Level::Rejected(code, S::Fee),
        Inner::TxInternalError => Level::Rejected(code, S::Unknown),
        Inner::TxFrozenKeyAccessed => Level::Rejected(code, S::Unknown),
    }
}

/// Find the first operation that did not succeed, and the stage it implies.
fn first_failed_operation(ops: &[OperationResult]) -> Option<(FailedOperation, FailureStage)> {
    use FailureStage as S;

    ops.iter().enumerate().find_map(|(i, op)| {
        let index = u32::try_from(i).unwrap_or(u32::MAX);
        let (operation, code, stage) = match op {
            OperationResult::OpInner(tr) => {
                let (code, success) = inner_code(tr);
                if success {
                    return None;
                }
                (
                    Some(tr.name()),
                    code,
                    soroban_stage(tr).unwrap_or(S::Operation),
                )
            }
            // Wrapper codes that do not say which operation type failed.
            OperationResult::OpBadAuth | OperationResult::OpNoAccount => {
                (None, op.name(), S::Validation)
            }
            OperationResult::OpNotSupported
            | OperationResult::OpTooManySubentries
            | OperationResult::OpExceededWorkLimit
            | OperationResult::OpTooManySponsoring => (None, op.name(), S::Unknown),
        };
        Some((
            FailedOperation {
                index,
                operation,
                code,
            },
            stage,
        ))
    })
}

/// The stage implied by a failed Soroban operation, or `None` for classic ops.
fn soroban_stage(tr: &OperationResultTr) -> Option<FailureStage> {
    use FailureStage as S;
    Some(match tr {
        OperationResultTr::InvokeHostFunction(r) => match r {
            InvokeHostFunctionResult::Success(_) => return None,
            InvokeHostFunctionResult::Malformed => S::HostFunction,
            InvokeHostFunctionResult::Trapped => S::ContractExecution,
            InvokeHostFunctionResult::ResourceLimitExceeded => S::ResourceLimit,
            InvokeHostFunctionResult::EntryArchived => S::StateArchival,
            InvokeHostFunctionResult::InsufficientRefundableFee => S::ResourceFee,
        },
        OperationResultTr::ExtendFootprintTtl(r) => match r {
            ExtendFootprintTtlResult::Success => return None,
            ExtendFootprintTtlResult::ResourceLimitExceeded => S::ResourceLimit,
            ExtendFootprintTtlResult::InsufficientRefundableFee => S::ResourceFee,
            // Issue #5 deliberately leaves this unmapped: `Malformed` on a TTL
            // operation is not a malformed *host function*.
            ExtendFootprintTtlResult::Malformed => S::Unknown,
        },
        OperationResultTr::RestoreFootprint(r) => match r {
            RestoreFootprintResult::Success => return None,
            RestoreFootprintResult::ResourceLimitExceeded => S::ResourceLimit,
            RestoreFootprintResult::InsufficientRefundableFee => S::ResourceFee,
            RestoreFootprintResult::Malformed => S::Unknown,
        },
        _ => return None,
    })
}

/// The XDR name of an operation's inner result code, and whether it succeeded.
///
/// Every Stellar result-code enum is `#[repr(i32)]` with `Success = 0` and
/// failures negative, so success is `discriminant == 0` for all of them. The
/// arms are spelled out rather than generated so the list is greppable.
fn inner_code(tr: &OperationResultTr) -> (&'static str, bool) {
    use OperationResultTr as T;
    let (name, code) = match tr {
        T::CreateAccount(r) => (r.name(), r.discriminant() as i32),
        T::Payment(r) => (r.name(), r.discriminant() as i32),
        T::PathPaymentStrictReceive(r) => (r.name(), r.discriminant() as i32),
        T::ManageSellOffer(r) => (r.name(), r.discriminant() as i32),
        T::CreatePassiveSellOffer(r) => (r.name(), r.discriminant() as i32),
        T::SetOptions(r) => (r.name(), r.discriminant() as i32),
        T::ChangeTrust(r) => (r.name(), r.discriminant() as i32),
        T::AllowTrust(r) => (r.name(), r.discriminant() as i32),
        T::AccountMerge(r) => (r.name(), r.discriminant() as i32),
        T::Inflation(r) => (r.name(), r.discriminant() as i32),
        T::ManageData(r) => (r.name(), r.discriminant() as i32),
        T::BumpSequence(r) => (r.name(), r.discriminant() as i32),
        T::ManageBuyOffer(r) => (r.name(), r.discriminant() as i32),
        T::PathPaymentStrictSend(r) => (r.name(), r.discriminant() as i32),
        T::CreateClaimableBalance(r) => (r.name(), r.discriminant() as i32),
        T::ClaimClaimableBalance(r) => (r.name(), r.discriminant() as i32),
        T::BeginSponsoringFutureReserves(r) => (r.name(), r.discriminant() as i32),
        T::EndSponsoringFutureReserves(r) => (r.name(), r.discriminant() as i32),
        T::RevokeSponsorship(r) => (r.name(), r.discriminant() as i32),
        T::Clawback(r) => (r.name(), r.discriminant() as i32),
        T::ClawbackClaimableBalance(r) => (r.name(), r.discriminant() as i32),
        T::SetTrustLineFlags(r) => (r.name(), r.discriminant() as i32),
        T::LiquidityPoolDeposit(r) => (r.name(), r.discriminant() as i32),
        T::LiquidityPoolWithdraw(r) => (r.name(), r.discriminant() as i32),
        T::InvokeHostFunction(r) => (r.name(), r.discriminant() as i32),
        T::ExtendFootprintTtl(r) => (r.name(), r.discriminant() as i32),
        T::RestoreFootprint(r) => (r.name(), r.discriminant() as i32),
    };
    (name, code == 0)
}

#[cfg(test)]
mod tests {
    //! Synthetic results built in code. Real mainnet results are exercised by
    //! the fixture tests in `crates/sdo/tests/`.

    use super::*;
    use stellar_xdr::{
        Hash, InnerTransactionResult, InnerTransactionResultExt, InnerTransactionResultPair,
        PaymentResult, TransactionResultExt,
    };

    fn op(tr: OperationResultTr) -> OperationResult {
        OperationResult::OpInner(tr)
    }

    fn plain(result: Outer) -> TransactionResult {
        TransactionResult {
            fee_charged: 100,
            result,
            ext: TransactionResultExt::V0,
        }
    }

    fn fee_bumped(inner: Inner, failed: bool) -> TransactionResult {
        let pair = InnerTransactionResultPair {
            transaction_hash: Hash([7; 32]),
            result: InnerTransactionResult {
                fee_charged: 0,
                result: inner,
                ext: InnerTransactionResultExt::V0,
            },
        };
        plain(if failed {
            Outer::TxFeeBumpInnerFailed(pair)
        } else {
            Outer::TxFeeBumpInnerSuccess(pair)
        })
    }

    fn invoke(r: InvokeHostFunctionResult) -> Vec<OperationResult> {
        vec![op(OperationResultTr::InvokeHostFunction(r))]
    }

    #[test]
    fn success_has_no_stage() {
        let o = classify(&plain(Outer::TxSuccess(
            invoke(InvokeHostFunctionResult::Success(Hash([0; 32])))
                .try_into()
                .unwrap(),
        )));
        assert!(o.succeeded());
        assert_eq!(o.result_code, "TxSuccess");
        assert!(o.failed_operation.is_none());
    }

    #[test]
    fn every_invoke_host_function_failure_maps_to_its_stage() {
        use FailureStage as S;
        let cases = [
            (InvokeHostFunctionResult::Malformed, S::HostFunction),
            (InvokeHostFunctionResult::Trapped, S::ContractExecution),
            (
                InvokeHostFunctionResult::ResourceLimitExceeded,
                S::ResourceLimit,
            ),
            (InvokeHostFunctionResult::EntryArchived, S::StateArchival),
            (
                InvokeHostFunctionResult::InsufficientRefundableFee,
                S::ResourceFee,
            ),
        ];
        for (r, expected) in cases {
            let o = classify(&plain(Outer::TxFailed(invoke(r).try_into().unwrap())));
            assert_eq!(o.stage, Some(expected));
            let failed = o.failed_operation.unwrap();
            assert_eq!(failed.index, 0);
            assert_eq!(failed.operation, Some("InvokeHostFunction"));
        }
    }

    #[test]
    fn fee_bump_is_unwrapped_to_the_inner_result() {
        let o = classify(&fee_bumped(
            Inner::TxFailed(
                invoke(InvokeHostFunctionResult::Trapped)
                    .try_into()
                    .unwrap(),
            ),
            true,
        ));
        // The wrapper must not be mistaken for the cause...
        assert_eq!(o.result_code, "TxFailed");
        assert_eq!(o.stage, Some(FailureStage::ContractExecution));
        // ...but must not be thrown away either.
        let bump = o.fee_bump.unwrap();
        assert_eq!(bump.outer_code, "TxFeeBumpInnerFailed");
        assert_eq!(bump.inner_transaction_hash, [7; 32]);
    }

    #[test]
    fn fee_bumped_success_is_a_success() {
        let o = classify(&fee_bumped(
            Inner::TxSuccess(Vec::new().try_into().unwrap()),
            false,
        ));
        assert!(o.succeeded());
        assert!(o.fee_bump.is_some());
    }

    #[test]
    fn transaction_level_rejections_map_without_operations() {
        use FailureStage as S;
        for (r, expected) in [
            (Outer::TxBadSeq, S::Sequence),
            (Outer::TxBadMinSeqAgeOrGap, S::Sequence),
            (Outer::TxInsufficientFee, S::Fee),
            (Outer::TxMalformed, S::Validation),
            (Outer::TxBadAuth, S::Validation),
            (Outer::TxSorobanInvalid, S::Validation),
            (Outer::TxInternalError, S::Unknown),
            (Outer::TxFrozenKeyAccessed, S::Unknown),
        ] {
            let o = classify(&plain(r));
            assert_eq!(o.stage, Some(expected), "{}", o.result_code);
            assert!(o.failed_operation.is_none());
        }
    }

    #[test]
    fn classic_operation_failure_is_the_operation_stage() {
        let ops = vec![op(OperationResultTr::Payment(PaymentResult::Underfunded))];
        let o = classify(&plain(Outer::TxFailed(ops.try_into().unwrap())));
        assert_eq!(o.stage, Some(FailureStage::Operation));
        let failed = o.failed_operation.unwrap();
        assert_eq!(failed.operation, Some("Payment"));
        assert_eq!(failed.code, "Underfunded");
    }

    #[test]
    fn the_first_failing_operation_is_reported_not_the_first_operation() {
        let ops = vec![
            op(OperationResultTr::Payment(PaymentResult::Success)),
            op(OperationResultTr::Payment(PaymentResult::NoDestination)),
        ];
        let o = classify(&plain(Outer::TxFailed(ops.try_into().unwrap())));
        let failed = o.failed_operation.unwrap();
        assert_eq!(failed.index, 1);
        assert_eq!(failed.code, "NoDestination");
    }

    #[test]
    fn op_bad_auth_is_validation_and_does_not_invent_an_operation_type() {
        let o = classify(&plain(Outer::TxFailed(
            vec![OperationResult::OpBadAuth].try_into().unwrap(),
        )));
        assert_eq!(o.stage, Some(FailureStage::Validation));
        assert_eq!(o.failed_operation.unwrap().operation, None);
    }

    #[test]
    fn failed_with_no_failing_operation_is_unknown_not_a_guess() {
        let o = classify(&plain(Outer::TxFailed(Vec::new().try_into().unwrap())));
        assert_eq!(o.stage, Some(FailureStage::Unknown));
    }

    #[test]
    fn ttl_operation_malformed_is_left_unknown_per_issue_5() {
        let ops = vec![op(OperationResultTr::ExtendFootprintTtl(
            ExtendFootprintTtlResult::Malformed,
        ))];
        let o = classify(&plain(Outer::TxFailed(ops.try_into().unwrap())));
        assert_eq!(o.stage, Some(FailureStage::Unknown));
    }
}
