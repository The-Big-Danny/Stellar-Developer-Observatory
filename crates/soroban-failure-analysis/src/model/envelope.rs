//! `TransactionEnvelope` → fee bump, inner transaction, and operations.
//!
//! A fee-bump envelope wraps an ordinary V1 transaction. Everything that
//! matters for diagnosis — source account, operations, Soroban data — belongs
//! to the **inner** transaction; the wrapper only contributes a fee source and
//! a fee. This module unwraps once, here, so nothing downstream ever has to
//! know that fee bumps exist.

use stellar_xdr::{
    FeeBumpTransactionInnerTx, HostFunction, InvokeContractArgs, Operation as XdrOperation,
    OperationBody, ScAddress, ScVal, Transaction, TransactionEnvelope, TransactionExt,
};

/// The fee-bump wrapper of an envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeeBump {
    /// The account paying for the bump, as a strkey (`G…` or `M…`).
    pub fee_source: String,
    /// The fee offered by the wrapper, in stroops.
    pub fee: i64,
}

/// A contract invocation, as submitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    /// The contract being called.
    pub contract: ScAddress,
    /// The function name.
    pub function: String,
    /// The arguments, undecoded.
    pub args: Vec<ScVal>,
}

impl Invocation {
    fn from_args(a: &InvokeContractArgs) -> Self {
        Self {
            contract: a.contract_address.clone(),
            function: a.function_name.0.to_utf8_string_lossy(),
            args: a.args.to_vec(),
        }
    }
}

/// What an operation does, reduced to what diagnosis needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationKind {
    /// Soroban `InvokeHostFunction`.
    InvokeHostFunction {
        /// Which host function, as named by the XDR, e.g. `"InvokeContract"`.
        host_function: &'static str,
        /// Present when the host function is a contract call.
        invocation: Option<Invocation>,
    },
    /// Soroban `ExtendFootprintTtl`.
    ExtendFootprintTtl {
        /// The ledger the TTL is extended to.
        extend_to: u32,
    },
    /// Soroban `RestoreFootprint`.
    RestoreFootprint,
    /// Any classic (non-Soroban) operation.
    Classic {
        /// The operation type, as named by the XDR, e.g. `"Payment"`.
        name: &'static str,
    },
}

impl OperationKind {
    /// Whether this is one of the three Soroban operation types.
    pub fn is_soroban(&self) -> bool {
        !matches!(self, Self::Classic { .. })
    }
}

/// One operation of the (inner) transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operation {
    /// Zero-based index within the transaction.
    pub index: u32,
    /// Operation-level source account override, as a strkey, if set.
    pub source_account: Option<String>,
    /// What the operation does.
    pub kind: OperationKind,
}

/// The envelope reduced to its inner transaction.
#[derive(Debug, Clone)]
pub(crate) struct UnwrappedEnvelope<'a> {
    pub fee_bump: Option<FeeBump>,
    pub source_account: String,
    pub fee: u32,
    pub operations: &'a [XdrOperation],
    /// The inner transaction's extension, which carries Soroban data. `None`
    /// for legacy V0 envelopes, which cannot carry any.
    pub ext: Option<&'a TransactionExt>,
}

pub(crate) fn unwrap(envelope: &TransactionEnvelope) -> UnwrappedEnvelope<'_> {
    fn from_tx(tx: &Transaction, fee_bump: Option<FeeBump>) -> UnwrappedEnvelope<'_> {
        UnwrappedEnvelope {
            fee_bump,
            source_account: tx.source_account.to_string(),
            fee: tx.fee,
            operations: tx.operations.as_slice(),
            ext: Some(&tx.ext),
        }
    }

    match envelope {
        TransactionEnvelope::Tx(e) => from_tx(&e.tx, None),
        TransactionEnvelope::TxFeeBump(e) => {
            // The XDR union has exactly one arm: a fee bump always wraps a V1
            // transaction. The irrefutable pattern documents that.
            let FeeBumpTransactionInnerTx::Tx(inner) = &e.tx.inner_tx;
            from_tx(
                &inner.tx,
                Some(FeeBump {
                    fee_source: e.tx.fee_source.to_string(),
                    fee: e.tx.fee,
                }),
            )
        }
        TransactionEnvelope::TxV0(e) => UnwrappedEnvelope {
            fee_bump: None,
            source_account: stellar_xdr::PublicKey::PublicKeyTypeEd25519(
                e.tx.source_account_ed25519.clone(),
            )
            .to_string(),
            fee: e.tx.fee,
            operations: e.tx.operations.as_slice(),
            ext: None,
        },
    }
}

pub(crate) fn operations(ops: &[XdrOperation]) -> Vec<Operation> {
    ops.iter()
        .enumerate()
        .map(|(i, op)| Operation {
            index: u32::try_from(i).unwrap_or(u32::MAX),
            source_account: op.source_account.as_ref().map(ToString::to_string),
            kind: kind(&op.body),
        })
        .collect()
}

fn kind(body: &OperationBody) -> OperationKind {
    match body {
        OperationBody::InvokeHostFunction(op) => OperationKind::InvokeHostFunction {
            host_function: op.host_function.name(),
            invocation: match &op.host_function {
                HostFunction::InvokeContract(args) => Some(Invocation::from_args(args)),
                _ => None,
            },
        },
        OperationBody::ExtendFootprintTtl(op) => OperationKind::ExtendFootprintTtl {
            extend_to: op.extend_to,
        },
        OperationBody::RestoreFootprint(_) => OperationKind::RestoreFootprint,
        other => OperationKind::Classic { name: other.name() },
    }
}
