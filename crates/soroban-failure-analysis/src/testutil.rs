//! Builders for **synthetic** test data, shared by the unit tests.
//!
//! Nothing here is a real transaction. Real mainnet data lives in `fixtures/`
//! and is exercised by the integration tests in `crates/sdo/tests/`. These
//! builders exist to pin down behaviour the real corpus does not (yet) cover.

use stellar_xdr::{
    ContractDataDurability, ContractEvent, ContractEventBody, ContractEventType, ContractEventV0,
    ContractId, DiagnosticEvent, ExtensionPoint, Hash, HostFunction, InvokeContractArgs,
    InvokeHostFunctionOp, InvokeHostFunctionResult, LedgerFootprint, LedgerKey,
    LedgerKeyContractCode, LedgerKeyContractData, Memo, MuxedAccount, Operation, OperationBody,
    OperationResult, OperationResultTr, Preconditions, ScAddress, ScError, ScErrorCode,
    ScSpecEntry, ScSpecUdtErrorEnumCaseV0, ScSpecUdtErrorEnumV0, ScString, ScSymbol, ScVal, ScVec,
    SequenceNumber, SorobanResources, SorobanTransactionData, SorobanTransactionDataExt,
    Transaction, TransactionEnvelope, TransactionExt, TransactionResult, TransactionResultExt,
    TransactionResultResult, TransactionV1Envelope, Uint256,
};

use crate::contract::resolve_contract_errors;
use crate::input::AnalysisInput;
use crate::model::TransactionModel;
use crate::rule::{FailureContext, RuleOutcome};
use crate::taxonomy::FailureStage;
use stellar_xdr::{
    SorobanAddressCredentials, SorobanAuthorizationEntry, SorobanAuthorizedFunction,
    SorobanAuthorizedInvocation, SorobanCredentials,
};

/// An address-credential authorization entry for `cid(5)::f`.
pub(crate) fn address_auth(
    address: ScAddress,
    nonce: i64,
    expiry: u32,
) -> SorobanAuthorizationEntry {
    SorobanAuthorizationEntry {
        credentials: SorobanCredentials::Address(SorobanAddressCredentials {
            address,
            nonce,
            signature_expiration_ledger: expiry,
            signature: ScVal::Void,
        }),
        root_invocation: SorobanAuthorizedInvocation {
            function: SorobanAuthorizedFunction::ContractFn(InvokeContractArgs {
                contract_address: ScAddress::Contract(cid(5)),
                function_name: ScSymbol("f".try_into().unwrap()),
                args: Vec::new().try_into().unwrap(),
            }),
            sub_invocations: Vec::new().try_into().unwrap(),
        },
    }
}

/// An `Error(Auth, code)` event in the mainnet shape: data `[message, extra…]`.
pub(crate) fn auth_error(
    from: ContractId,
    code: ScErrorCode,
    message: &str,
    extra: Vec<ScVal>,
) -> DiagnosticEvent {
    let mut data = vec![ScVal::String(ScString(message.try_into().unwrap()))];
    data.extend(extra);
    ev(
        Some(from),
        vec![sym("error"), ScVal::Error(ScError::Auth(code))],
        ScVal::Vec(Some(ScVec(data.try_into().unwrap()))),
    )
}

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

pub(crate) fn error_enum(name: &str, cases: &[(&str, u32)]) -> ScSpecEntry {
    ScSpecEntry::UdtErrorEnumV0(ScSpecUdtErrorEnumV0 {
        doc: "".try_into().unwrap(),
        lib: "".try_into().unwrap(),
        name: name.try_into().unwrap(),
        cases: cases
            .iter()
            .map(|(n, v)| ScSpecUdtErrorEnumCaseV0 {
                doc: "".try_into().unwrap(),
                name: (*n).try_into().unwrap(),
                value: *v,
            })
            .collect::<Vec<_>>()
            .try_into()
            .unwrap(),
    })
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

/// [`minimal_input`] carrying the given diagnostic events.
pub(crate) fn input_with_events(events: Vec<DiagnosticEvent>) -> AnalysisInput {
    let base = minimal_input();
    AnalysisInput::builder(base.envelope, base.result)
        .diagnostic_events(events)
        .build()
}

/// [`minimal_input`] rejected at transaction level with `result`.
pub(crate) fn tx_level_failure(result: TransactionResultResult) -> AnalysisInput {
    let base = minimal_input();
    AnalysisInput::builder(
        base.envelope,
        TransactionResult {
            fee_charged: 100,
            result,
            ext: TransactionResultExt::V0,
        },
    )
    .build()
}

/// A plain (not fee-bumped) Soroban transaction invoking `cid(5)::f`, whose
/// single operation failed with `result`.
///
/// Declares 1000 CPU instructions, a 300-stroop resource fee, and `footprint`
/// as its read-write footprint. `events: None` means the source emitted no
/// diagnostic events at all; `Some(vec![])` means it emitted none for this
/// transaction.
pub(crate) fn soroban_failure(
    result: InvokeHostFunctionResult,
    footprint: Vec<LedgerKey>,
    events: Option<Vec<DiagnosticEvent>>,
) -> AnalysisInput {
    soroban_failure_with_auth(result, footprint, Vec::new(), events)
}

/// [`soroban_failure`] carrying `auth` as the operation's authorization entries.
pub(crate) fn soroban_failure_with_auth(
    result: InvokeHostFunctionResult,
    footprint: Vec<LedgerKey>,
    auth: Vec<SorobanAuthorizationEntry>,
    events: Option<Vec<DiagnosticEvent>>,
) -> AnalysisInput {
    let args = InvokeContractArgs {
        contract_address: ScAddress::Contract(cid(5)),
        function_name: ScSymbol("f".try_into().unwrap()),
        args: Vec::new().try_into().unwrap(),
    };
    let op = Operation {
        source_account: None,
        body: OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
            host_function: HostFunction::InvokeContract(args),
            auth: auth.try_into().unwrap(),
        }),
    };
    let data = SorobanTransactionData {
        ext: SorobanTransactionDataExt::V0,
        resources: SorobanResources {
            footprint: LedgerFootprint {
                read_only: Vec::new().try_into().unwrap(),
                read_write: footprint.try_into().unwrap(),
            },
            instructions: 1000,
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
    let envelope = TransactionEnvelope::Tx(TransactionV1Envelope {
        tx,
        signatures: Vec::new().try_into().unwrap(),
    });
    let result = TransactionResult {
        fee_charged: 350,
        result: TransactionResultResult::TxFailed(
            vec![OperationResult::OpInner(
                OperationResultTr::InvokeHostFunction(result),
            )]
            .try_into()
            .unwrap(),
        ),
        ext: TransactionResultExt::V0,
    };
    let builder = AnalysisInput::builder(envelope, result);
    match events {
        Some(events) => builder.diagnostic_events(events).build(),
        None => builder.build(),
    }
}

/// A persistent contract-data footprint key.
pub(crate) fn data_key(contract: ContractId, key: ScVal) -> LedgerKey {
    LedgerKey::ContractData(LedgerKeyContractData {
        contract: ScAddress::Contract(contract),
        key,
        durability: ContractDataDurability::Persistent,
    })
}

/// A contract-code footprint key.
pub(crate) fn code_key(hash: [u8; 32]) -> LedgerKey {
    LedgerKey::ContractCode(LedgerKeyContractCode { hash: Hash(hash) })
}

/// A `core_metrics` diagnostic event.
pub(crate) fn core_metric(name: &str, value: u64) -> DiagnosticEvent {
    ev(
        None,
        vec![sym("core_metrics"), sym(name)],
        ScVal::U64(value),
    )
}

/// The host's footprint-violation error event, in the shape observed on
/// mainnet: data `[message, address, key]`.
pub(crate) fn footprint_violation(contract: ContractId, key: ScVal) -> DiagnosticEvent {
    ev(
        Some(contract.clone()),
        vec![
            sym("error"),
            ScVal::Error(ScError::Storage(ScErrorCode::ExceededLimit)),
        ],
        ScVal::Vec(Some(ScVec(
            vec![
                ScVal::String(ScString(
                    "trying to access contract data key outside of the footprint"
                        .try_into()
                        .unwrap(),
                )),
                ScVal::Address(ScAddress::Contract(contract)),
                key,
            ]
            .try_into()
            .unwrap(),
        ))),
    )
}

/// Evaluate `f` against the context `analyze` would build for `input`.
pub(crate) fn analyze_ctx(
    input: &AnalysisInput,
    f: impl FnOnce(&FailureContext<'_>) -> RuleOutcome,
) -> RuleOutcome {
    let model = TransactionModel::from_input(input);
    let contract_errors = resolve_contract_errors(&model, &input.contract_specs);
    let ctx = FailureContext {
        input,
        model: &model,
        stage: model.stage().unwrap_or(FailureStage::Unknown),
        contract_errors: &contract_errors,
    };
    f(&ctx)
}
