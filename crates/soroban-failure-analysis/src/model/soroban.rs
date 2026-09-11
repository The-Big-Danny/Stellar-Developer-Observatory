//! Soroban transaction data: authorization, footprint, and resources.
//!
//! # Declared is not observed
//!
//! [`DeclaredResources`] is what the *submitter asked for*, read from
//! `SorobanTransactionData`. [`ObservedResources`] is what the *host reports
//! having used*, read from `core_metrics` diagnostic events and the fees charged
//! in the metadata. They are separate types because a resource rule's entire
//! job is comparing one against the other; merging them would make that
//! comparison impossible to write correctly.
//!
//! Observed values are `Option` throughout. When the source did not report a
//! value it is absent — never zero, never estimated.

use std::collections::BTreeMap;

use stellar_xdr::{
    LedgerKey, ScAddress, SorobanAuthorizationEntry, SorobanAuthorizedFunction, SorobanCredentials,
    SorobanTransactionData, SorobanTransactionDataExt, SorobanTransactionMetaExt, TransactionMeta,
};

use super::envelope::{Operation, OperationKind};

/// Who is authorizing an invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthCredentials {
    /// Authorized implicitly by the transaction's source account signature.
    SourceAccount,
    /// Authorized by a separate signature from `address`.
    ///
    /// Covers `Address`, `AddressV2` and `AddressWithDelegates`, which all carry
    /// the same address credentials; `variant` records which arm it was.
    Address {
        /// The authorizing address.
        address: ScAddress,
        /// Replay-protection nonce.
        nonce: i64,
        /// Last ledger at which the signature is valid.
        signature_expiration_ledger: u32,
        /// The XDR arm, e.g. `"Address"` or `"AddressWithDelegates"`.
        variant: &'static str,
    },
}

/// One `SorobanAuthorizationEntry`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthEntry {
    /// Zero-based index within the operation's auth entries.
    pub index: u32,
    /// Who is authorizing.
    pub credentials: AuthCredentials,
    /// The contract the root invocation calls, when it is a contract call.
    pub root_contract: Option<ScAddress>,
    /// The root function name, when it is a contract call.
    pub root_function: Option<String>,
    /// Number of nested sub-invocations authorized under the root.
    pub sub_invocations: usize,
    /// The entry verbatim, for rules that need the full invocation tree.
    pub raw: SorobanAuthorizationEntry,
}

impl AuthEntry {
    fn from_xdr(index: u32, e: &SorobanAuthorizationEntry) -> Self {
        let credentials = match &e.credentials {
            SorobanCredentials::SourceAccount => AuthCredentials::SourceAccount,
            SorobanCredentials::Address(c) | SorobanCredentials::AddressV2(c) => {
                AuthCredentials::Address {
                    address: c.address.clone(),
                    nonce: c.nonce,
                    signature_expiration_ledger: c.signature_expiration_ledger,
                    variant: e.credentials.name(),
                }
            }
            SorobanCredentials::AddressWithDelegates(d) => AuthCredentials::Address {
                address: d.address_credentials.address.clone(),
                nonce: d.address_credentials.nonce,
                signature_expiration_ledger: d.address_credentials.signature_expiration_ledger,
                variant: e.credentials.name(),
            },
        };
        let (root_contract, root_function) = match &e.root_invocation.function {
            SorobanAuthorizedFunction::ContractFn(args) => (
                Some(args.contract_address.clone()),
                Some(args.function_name.0.to_utf8_string_lossy()),
            ),
            _ => (None, None),
        };
        Self {
            index,
            credentials,
            root_contract,
            root_function,
            sub_invocations: e.root_invocation.sub_invocations.len(),
            raw: e.clone(),
        }
    }
}

/// Whether a footprint entry may be read or also written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FootprintAccess {
    /// Declared in the read-only footprint.
    ReadOnly,
    /// Declared in the read-write footprint.
    ReadWrite,
}

/// The ledger keys a transaction declared it would touch.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Footprint {
    /// Keys that may only be read.
    pub read_only: Vec<LedgerKey>,
    /// Keys that may be read and written.
    pub read_write: Vec<LedgerKey>,
}

impl Footprint {
    /// How a key was declared, or `None` if the footprint does not cover it.
    pub fn access(&self, key: &LedgerKey) -> Option<FootprintAccess> {
        if self.read_write.contains(key) {
            Some(FootprintAccess::ReadWrite)
        } else if self.read_only.contains(key) {
            Some(FootprintAccess::ReadOnly)
        } else {
            None
        }
    }

    /// WASM hashes of every `ContractCode` entry in the footprint.
    ///
    /// These identify the exact contract code the transaction loaded, which is
    /// how M3 checks that a contract spec fetched later still matches the code
    /// that actually ran — contracts can be upgraded after a failure.
    pub fn contract_code_hashes(&self) -> Vec<[u8; 32]> {
        self.read_only
            .iter()
            .chain(&self.read_write)
            .filter_map(|k| match k {
                LedgerKey::ContractCode(c) => Some(c.hash.0),
                _ => None,
            })
            .collect()
    }

    /// Total number of declared entries.
    pub fn len(&self) -> usize {
        self.read_only.len() + self.read_write.len()
    }

    /// Whether the footprint declares nothing.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Resource limits and fee declared by the submitter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredResources {
    /// CPU instruction limit.
    pub instructions: u32,
    /// Disk read byte limit. Named `read_bytes` before protocol 23.
    pub disk_read_bytes: u32,
    /// Write byte limit.
    pub write_bytes: u32,
    /// Maximum resource fee the submitter will pay, in stroops.
    pub resource_fee: i64,
    /// Read-write footprint indexes of archived entries to restore
    /// automatically (`SorobanResourcesExtV0`, protocol 23+). Empty for V0 data.
    pub archived_entry_indexes: Vec<u32>,
}

/// Everything from the Soroban side of the transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SorobanData {
    /// The `SorobanTransactionDataExt` arm: 0 or 1.
    pub data_version: u32,
    /// Authorization entries of the invoke operation, in order.
    pub auth: Vec<AuthEntry>,
    /// Declared footprint.
    pub footprint: Footprint,
    /// Declared limits and resource fee.
    pub declared: DeclaredResources,
}

impl SorobanData {
    pub(crate) fn from_xdr(
        data: &SorobanTransactionData,
        operations: &[Operation],
        raw_ops: &[stellar_xdr::Operation],
    ) -> Self {
        let (data_version, archived_entry_indexes) = match &data.ext {
            SorobanTransactionDataExt::V0 => (0, Vec::new()),
            SorobanTransactionDataExt::V1(ext) => (1, ext.archived_soroban_entries.to_vec()),
        };

        // Protocol rule: a Soroban transaction carries exactly one operation.
        // Auth entries live on that operation, so they are collected from any
        // invoke operation rather than assuming its index.
        let auth = operations
            .iter()
            .filter(|op| matches!(op.kind, OperationKind::InvokeHostFunction { .. }))
            .filter_map(|op| match &raw_ops.get(op.index as usize)?.body {
                stellar_xdr::OperationBody::InvokeHostFunction(ih) => Some(&ih.auth),
                _ => None,
            })
            .flat_map(|entries| entries.iter())
            .enumerate()
            .map(|(i, e)| AuthEntry::from_xdr(u32::try_from(i).unwrap_or(u32::MAX), e))
            .collect();

        let r = &data.resources;
        Self {
            data_version,
            auth,
            footprint: Footprint {
                read_only: r.footprint.read_only.to_vec(),
                read_write: r.footprint.read_write.to_vec(),
            },
            declared: DeclaredResources {
                instructions: r.instructions,
                disk_read_bytes: r.disk_read_bytes,
                write_bytes: r.write_bytes,
                resource_fee: data.resource_fee,
                archived_entry_indexes,
            },
        }
    }
}

/// Fees actually charged, from `SorobanTransactionMetaExtV1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChargedFees {
    /// Non-refundable resource fee charged, in stroops.
    pub non_refundable: i64,
    /// Refundable resource fee charged, in stroops.
    pub refundable: i64,
    /// Rent fee charged, in stroops.
    pub rent: i64,
}

/// What the host reports having consumed.
///
/// Sourced from diagnostic events, which are **not part of consensus**: treat
/// these as the reporting node's measurement, not as ledger truth.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObservedResources {
    /// Every `core_metrics` value reported, keyed by the host's metric name
    /// (`cpu_insn`, `mem_byte`, `read_entry`, …). Kept whole so no reported
    /// value is lost to a field list that is incomplete.
    pub core_metrics: BTreeMap<String, u64>,
    /// Fees charged, from the metadata. `None` when the meta did not carry them.
    pub fees_charged: Option<ChargedFees>,
}

impl ObservedResources {
    /// CPU instructions consumed (`cpu_insn`). Compare with
    /// [`DeclaredResources::instructions`].
    pub fn cpu_instructions(&self) -> Option<u64> {
        self.core_metrics.get("cpu_insn").copied()
    }

    /// Memory bytes consumed (`mem_byte`).
    pub fn memory_bytes(&self) -> Option<u64> {
        self.core_metrics.get("mem_byte").copied()
    }
}

/// Read the charged fees out of V3 or V4 transaction metadata.
pub(crate) fn fees_charged(meta: Option<&TransactionMeta>) -> Option<ChargedFees> {
    let ext = match meta? {
        TransactionMeta::V3(m) => &m.soroban_meta.as_ref()?.ext,
        TransactionMeta::V4(m) => &m.soroban_meta.as_ref()?.ext,
        _ => return None,
    };
    match ext {
        SorobanTransactionMetaExt::V1(v) => Some(ChargedFees {
            non_refundable: v.total_non_refundable_resource_fee_charged,
            refundable: v.total_refundable_resource_fee_charged,
            rent: v.rent_fee_charged,
        }),
        SorobanTransactionMetaExt::V0 => None,
    }
}
