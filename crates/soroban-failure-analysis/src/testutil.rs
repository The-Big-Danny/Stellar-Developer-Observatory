//! Builders for **synthetic** test data, shared by the unit tests.
//!
//! Nothing here is a real transaction. Real mainnet data lives in `fixtures/`
//! and is exercised by the integration tests in `crates/sdo/tests/`. These
//! builders exist to pin down behaviour the real corpus does not (yet) cover.

use stellar_xdr::{
    ContractEvent, ContractEventBody, ContractEventType, ContractEventV0, ContractId,
    DiagnosticEvent, ExtensionPoint, Hash, Memo, MuxedAccount, Operation, Preconditions, ScError,
    ScString, ScSymbol, ScVal, SequenceNumber, Transaction, TransactionEnvelope, TransactionExt,
    TransactionResult, TransactionResultExt, TransactionResultResult, TransactionV1Envelope,
    Uint256,
};

use crate::input::AnalysisInput;

pub(crate) fn cid(n: u8) -> ContractId {
    ContractId(Hash([n; 32]))
}

pub(crate) fn sym(s: &str) -> ScVal {
    ScVal::Symbol(ScSymbol(s.try_into().unwrap()))
}

pub(crate) fn ev(contract: Option<ContractId>, topics: Vec<ScVal>, data: ScVal) -> DiagnosticEvent {
    DiagnosticEvent {
        in_successful_contract_call: false,
        event: ContractEvent {
            ext: ExtensionPoint::V0,
            contract_id: contract,
            type_: ContractEventType::Diagnostic,
            body: ContractEventBody::V0(ContractEventV0 {
                topics: topics.try_into().unwrap(),
                data,
            }),
        },
    }
}

pub(crate) fn call(from: Option<ContractId>, to: ContractId, function: &str) -> DiagnosticEvent {
    ev(
        from,
        vec![
            sym("fn_call"),
            ScVal::Bytes(to.0 .0.to_vec().try_into().unwrap()),
            sym(function),
        ],
        ScVal::Void,
    )
}

pub(crate) fn err(from: ContractId, e: ScError, message: &str) -> DiagnosticEvent {
    ev(
        Some(from),
        vec![sym("error"), ScVal::Error(e)],
        ScVal::String(ScString(message.try_into().unwrap())),
    )
}

pub(crate) fn host_fn_failed(e: ScError) -> DiagnosticEvent {
    ev(
        None,
        vec![sym("host_fn_failed"), ScVal::Error(e)],
        ScVal::Void,
    )
}

/// A structurally valid, operation-less, failed transaction.
pub(crate) fn minimal_input() -> AnalysisInput {
    let tx = Transaction {
        source_account: MuxedAccount::Ed25519(Uint256([0; 32])),
        fee: 100,
        seq_num: SequenceNumber(1),
        cond: Preconditions::None,
        memo: Memo::None,
        operations: Vec::<Operation>::new().try_into().unwrap(),
        ext: TransactionExt::V0,
    };
    let envelope = TransactionEnvelope::Tx(TransactionV1Envelope {
        tx,
        signatures: Vec::new().try_into().unwrap(),
    });
    let result = TransactionResult {
        fee_charged: 100,
        result: TransactionResultResult::TxFailed(Vec::new().try_into().unwrap()),
        ext: TransactionResultExt::V0,
    };
    AnalysisInput::builder(envelope, result).build()
}
