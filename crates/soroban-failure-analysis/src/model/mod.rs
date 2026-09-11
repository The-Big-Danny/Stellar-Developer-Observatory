//! The canonical transaction model (milestone M2).
//!
//! [`TransactionModel`] is the one representation of a transaction that the
//! rest of the engine — and every future rule — works against. It exists so
//! that nothing downstream needs to understand `stellar-xdr` layout:
//!
//! * fee bumps are unwrapped once, here ([`envelope`], [`outcome`]);
//! * `TransactionMeta` V3 vs V4 differences are absorbed here;
//! * diagnostic events arrive classified, with the call tree reconstructed
//!   ([`events`]);
//! * declared resources and observed consumption are separate types
//!   ([`soroban`]).
//!
//! Raw XDR values (`ScVal`, `LedgerKey`, auth entries) are retained where a rule
//! may need detail the model does not summarise. The model reduces; it does not
//! discard.
//!
//! Construction is infallible. The inputs are already-decoded XDR, and anything
//! surprising about them is recorded in [`TransactionModel::notes`] rather than
//! turned into an error that would prevent analysis.

pub mod envelope;
pub mod events;
pub mod outcome;
pub mod soroban;

use crate::input::AnalysisInput;
use crate::taxonomy::FailureStage;

pub use envelope::{FeeBump, Invocation, Operation, OperationKind};
pub use events::{
    error_label, CallFrame, CallOutcome, DiagEvent, DiagnosticAvailability, Diagnostics, EventKind,
    TerminalError,
};
pub use outcome::{FailedOperation, FeeBumpResult, Outcome};
pub use soroban::{
    AuthCredentials, AuthEntry, ChargedFees, DeclaredResources, Footprint, FootprintAccess,
    ObservedResources, SorobanData,
};

/// A decoded transaction, reduced to what diagnosis needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionModel {
    /// Hex transaction hash, if supplied. For a fee bump this is the outer hash.
    pub hash: Option<String>,
    /// The fee-bump wrapper, if the envelope was fee-bumped.
    pub fee_bump: Option<FeeBump>,
    /// Source account of the inner transaction, as a strkey.
    pub source_account: String,
    /// Fee offered by the inner transaction, in stroops.
    pub fee: u32,
    /// Operations of the inner transaction.
    pub operations: Vec<Operation>,
    /// Soroban data. `None` for classic transactions — this is "not
    /// applicable", not "missing".
    pub soroban: Option<SorobanData>,
    /// What the result says happened.
    pub outcome: Outcome,
    /// Diagnostic events, classified.
    pub diagnostics: Diagnostics,
    /// Consumption reported by the host and fees charged.
    pub observed: ObservedResources,
    /// Anything inconsistent or surprising found while building the model.
    pub notes: Vec<String>,
}

impl TransactionModel {
    /// Build the model from decoded input.
    pub fn from_input(input: &AnalysisInput) -> Self {
        let env = envelope::unwrap(&input.envelope);
        let operations = envelope::operations(env.operations);

        let soroban = match env.ext {
            Some(stellar_xdr::TransactionExt::V1(data)) => {
                Some(SorobanData::from_xdr(data, &operations, env.operations))
            }
            _ => None,
        };

        let outcome = outcome::classify(&input.result);
        let diagnostics =
            Diagnostics::from_events(&input.diagnostic_events, input.diagnostics_enabled);
        let observed = ObservedResources {
            core_metrics: diagnostics.core_metrics(),
            fees_charged: soroban::fees_charged(input.meta.as_ref()),
        };

        let mut notes = Vec::new();
        if env.fee_bump.is_some() != outcome.fee_bump.is_some() {
            notes.push(format!(
                "Envelope and result disagree about fee bumping (envelope: {}, result: {}).",
                if env.fee_bump.is_some() {
                    "fee-bumped"
                } else {
                    "not fee-bumped"
                },
                if outcome.fee_bump.is_some() {
                    "fee-bump wrapper"
                } else {
                    "plain"
                },
            ));
        }
        if soroban.is_some() && !operations.iter().any(|o| o.kind.is_soroban()) {
            notes.push("Soroban transaction data is present but no Soroban operation is.".into());
        }

        Self {
            hash: input.transaction_hash.clone(),
            fee_bump: env.fee_bump,
            source_account: env.source_account,
            fee: env.fee,
            operations,
            soroban,
            outcome,
            diagnostics,
            observed,
            notes,
        }
    }

    /// Whether the envelope was fee-bumped.
    pub fn is_fee_bumped(&self) -> bool {
        self.fee_bump.is_some()
    }

    /// Where the transaction failed. `None` means it succeeded.
    pub fn stage(&self) -> Option<FailureStage> {
        self.outcome.stage
    }

    /// The contract invocation, if the transaction calls a contract.
    pub fn invocation(&self) -> Option<&Invocation> {
        self.operations.iter().find_map(|op| match &op.kind {
            OperationKind::InvokeHostFunction { invocation, .. } => invocation.as_ref(),
            _ => None,
        })
    }

    /// Whether the transaction contains any Soroban operation.
    pub fn is_soroban(&self) -> bool {
        self.operations.iter().any(|o| o.kind.is_soroban())
    }
}

#[cfg(test)]
mod tests {
    //! Synthetic transactions for cases the real fixture corpus lacks: every
    //! recorded Soroban failure is fee-bumped and authorizes via the source
    //! account. These are built in code and are **not** real transactions.

    use super::*;
    use crate::testutil::cid;
    use stellar_xdr::{
        Hash, InvokeContractArgs, InvokeHostFunctionOp, InvokeHostFunctionResult, LedgerFootprint,
        LedgerKey, LedgerKeyContractCode, Memo, MuxedAccount, Operation as XdrOperation,
        OperationBody, OperationResult, OperationResultTr, Preconditions, ScAddress, ScSymbol,
        ScVal, SequenceNumber, SorobanAddressCredentials, SorobanAuthorizationEntry,
        SorobanAuthorizedFunction, SorobanAuthorizedInvocation, SorobanCredentials,
        SorobanResources, SorobanResourcesExtV0, SorobanTransactionData, SorobanTransactionDataExt,
        Transaction, TransactionEnvelope, TransactionExt, TransactionResult, TransactionResultExt,
        TransactionResultResult, TransactionV1Envelope, Uint256,
    };

    fn args() -> InvokeContractArgs {
        InvokeContractArgs {
            contract_address: ScAddress::Contract(cid(5)),
            function_name: ScSymbol("transfer".try_into().unwrap()),
            args: vec![ScVal::U32(1)].try_into().unwrap(),
        }
    }

    /// A plain (not fee-bumped) Soroban transaction with one address-credential
    /// auth entry, a two-key footprint, and V1 resource data.
    fn soroban_input() -> AnalysisInput {
        let auth = SorobanAuthorizationEntry {
            credentials: SorobanCredentials::Address(SorobanAddressCredentials {
                address: ScAddress::Contract(cid(7)),
                nonce: 42,
                signature_expiration_ledger: 1000,
                signature: ScVal::Void,
            }),
            root_invocation: SorobanAuthorizedInvocation {
                function: SorobanAuthorizedFunction::ContractFn(args()),
                sub_invocations: Vec::new().try_into().unwrap(),
            },
        };
        let op = XdrOperation {
            source_account: None,
            body: OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
                host_function: stellar_xdr::HostFunction::InvokeContract(args()),
                auth: vec![auth].try_into().unwrap(),
            }),
        };
        let code = LedgerKey::ContractCode(LedgerKeyContractCode {
            hash: Hash([9; 32]),
        });
        let data = SorobanTransactionData {
            ext: SorobanTransactionDataExt::V1(SorobanResourcesExtV0 {
                archived_soroban_entries: vec![0].try_into().unwrap(),
            }),
            resources: SorobanResources {
                footprint: LedgerFootprint {
                    read_only: vec![code].try_into().unwrap(),
                    read_write: Vec::new().try_into().unwrap(),
                },
                instructions: 500,
                disk_read_bytes: 10,
                write_bytes: 20,
            },
            resource_fee: 300,
        };
        let tx = Transaction {
            source_account: MuxedAccount::Ed25519(Uint256([1; 32])),
            fee: 400,
            seq_num: SequenceNumber(1),
            cond: Preconditions::None,
            memo: Memo::None,
            operations: vec![op].try_into().unwrap(),
            ext: TransactionExt::V1(data),
        };
        let result = TransactionResult {
            fee_charged: 350,
            result: TransactionResultResult::TxFailed(
                vec![OperationResult::OpInner(
                    OperationResultTr::InvokeHostFunction(
                        InvokeHostFunctionResult::ResourceLimitExceeded,
                    ),
                )]
                .try_into()
                .unwrap(),
            ),
            ext: TransactionResultExt::V0,
        };
        let envelope = TransactionEnvelope::Tx(TransactionV1Envelope {
            tx,
            signatures: Vec::new().try_into().unwrap(),
        });
        AnalysisInput::builder(envelope, result).build()
    }

    #[test]
    fn a_plain_soroban_transaction_is_not_mistaken_for_a_fee_bump() {
        let m = TransactionModel::from_input(&soroban_input());
        assert!(!m.is_fee_bumped());
        assert!(m.outcome.fee_bump.is_none());
        assert!(m.notes.is_empty(), "{:?}", m.notes);
        assert_eq!(m.stage(), Some(FailureStage::ResourceLimit));
        assert_eq!(m.invocation().unwrap().function, "transfer");
    }

    #[test]
    fn address_credential_auth_entries_are_extracted() {
        let m = TransactionModel::from_input(&soroban_input());
        let auth = &m.soroban.as_ref().unwrap().auth;
        assert_eq!(auth.len(), 1);
        assert_eq!(auth[0].index, 0);
        assert_eq!(
            auth[0].credentials,
            AuthCredentials::Address {
                address: ScAddress::Contract(cid(7)),
                nonce: 42,
                signature_expiration_ledger: 1000,
                variant: "Address",
            }
        );
        assert_eq!(auth[0].root_contract, Some(ScAddress::Contract(cid(5))));
        assert_eq!(auth[0].root_function.as_deref(), Some("transfer"));
    }

    #[test]
    fn v1_resource_data_exposes_archived_entry_indexes() {
        let m = TransactionModel::from_input(&soroban_input());
        let s = m.soroban.unwrap();
        assert_eq!(s.data_version, 1);
        assert_eq!(s.declared.archived_entry_indexes, vec![0]);
        assert_eq!(
            (
                s.declared.instructions,
                s.declared.disk_read_bytes,
                s.declared.write_bytes
            ),
            (500, 10, 20)
        );
        assert_eq!(s.footprint.contract_code_hashes(), vec![[9; 32]]);
        assert_eq!(
            s.footprint.access(&s.footprint.read_only[0]),
            Some(FootprintAccess::ReadOnly)
        );
    }

    #[test]
    fn without_meta_or_diagnostics_observed_values_are_absent_not_zero() {
        let m = TransactionModel::from_input(&soroban_input());
        assert_eq!(m.observed.cpu_instructions(), None);
        assert_eq!(m.observed.fees_charged, None);
        assert_eq!(
            m.diagnostics.availability,
            DiagnosticAvailability::NotEmitted
        );
    }
}
