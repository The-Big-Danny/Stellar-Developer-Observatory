//! Deduplication cluster keys for the M5 evaluation protocol.
//!
//! The protocol (`docs/evaluation/protocol.md` §5) deduplicates evaluation
//! samples by the *code* a transaction ran and by *who submitted it*, so that
//! one busy contract or one busy submitter cannot dominate a dataset. This
//! module derives both keys.
//!
//! Everything here is **pure**: it reads an already-decoded transaction and an
//! already-fetched contract instance. The network calls that obtain those live
//! in [`crate::client`] and [`crate::contract`]. That split is what makes the
//! keys testable offline, against recorded responses.
//!
//! Two properties matter more than convenience, and both are enforced by tests:
//!
//! * **An unresolved code identity never falls back to the contract ID.** Two
//!   different contracts running the same code are one cluster; the same
//!   contract upgraded to different code is two. A contract-ID fallback would
//!   silently break both, so an unresolved identity stays unresolved and its
//!   reason is recorded.
//! * **No key is derived from SDO's own diagnosis.** Cluster keys come from the
//!   envelope, the footprint and ledger entries only. If selection could see a
//!   cause class, the evaluation would be measuring its own output.

use serde::{Deserialize, Serialize};
use soroban_failure_analysis::model::error_label;
use soroban_failure_analysis::TransactionModel;
use stellar_xdr::{ContractExecutable, ScAddress, ScError};

/// Why a root contract's code identity could not be determined.
///
/// Each variant serialises to the exclusion reason the protocol's funnel
/// (§4.2, check 7) uses, so the wire form and the documentation cannot drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnresolvedCode {
    /// The instance ledger entry could not be read: absent, archived, or the
    /// lookup failed after the protocol's retry and fallback procedure.
    #[serde(rename = "code_identity_instance_unavailable")]
    InstanceUnavailable,
    /// The instance names a WASM hash that the transaction's own declared
    /// footprint does not contain, so the contract that ran may not be the
    /// contract now installed.
    #[serde(rename = "code_identity_not_in_footprint")]
    NotInFootprint,
    /// The instance holds an executable kind this protocol version does not
    /// cluster.
    #[serde(rename = "code_identity_unsupported_executable")]
    UnsupportedExecutable,
}

impl UnresolvedCode {
    /// The protocol's exclusion reason code.
    pub fn reason(self) -> &'static str {
        match self {
            Self::InstanceUnavailable => "code_identity_instance_unavailable",
            Self::NotInFootprint => "code_identity_not_in_footprint",
            Self::UnsupportedExecutable => "code_identity_unsupported_executable",
        }
    }
}

impl std::fmt::Display for UnresolvedCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.reason())
    }
}

/// The identity of the code a transaction's root contract ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeIdentity {
    /// A cluster key: `wasm:<64 hex chars>` or `builtin:stellar-asset-contract`.
    Resolved(String),
    /// No key, and why. Never substituted with the contract ID.
    Unresolved(UnresolvedCode),
}

/// The cluster key for the Stellar Asset Contract.
///
/// Every SAC instance runs the same code built into the host, so they share one
/// cluster. There is no `ContractCode` entry to check a footprint against.
pub const STELLAR_ASSET_CLUSTER: &str = "builtin:stellar-asset-contract";

/// Derive the `code_cluster` of a root contract, per protocol §5.1.
///
/// `instance` is the executable read from the contract's instance entry, or
/// `None` if that entry could not be read. `footprint_code_hashes` are the
/// `ContractCode` hashes of the transaction's own declared footprint, which is
/// what ties the code now installed to the code that actually ran.
pub fn code_cluster(
    instance: Option<&ContractExecutable>,
    footprint_code_hashes: &[[u8; 32]],
) -> CodeIdentity {
    match instance {
        None => CodeIdentity::Unresolved(UnresolvedCode::InstanceUnavailable),
        Some(ContractExecutable::Wasm(hash)) => {
            if footprint_code_hashes.contains(&hash.0) {
                CodeIdentity::Resolved(format!("wasm:{}", hex(&hash.0)))
            } else {
                CodeIdentity::Unresolved(UnresolvedCode::NotInFootprint)
            }
        }
        Some(ContractExecutable::StellarAsset) => {
            CodeIdentity::Resolved(STELLAR_ASSET_CLUSTER.to_string())
        }
        Some(_) => CodeIdentity::Unresolved(UnresolvedCode::UnsupportedExecutable),
    }
}

/// Derive the `submitter_cluster` of a transaction, per protocol §5.2.
///
/// The fee-bump fee source when there is one, otherwise the inner source
/// account, always reduced to the underlying `G…` account so that one key
/// cannot present itself as many muxed identities.
pub fn submitter_cluster(model: &TransactionModel) -> String {
    let account = match &model.fee_bump {
        Some(bump) => &bump.fee_source,
        None => &model.source_account,
    };
    base_account(account)
}

/// Reduce a muxed account strkey (`M…`) to its underlying `G…` account.
///
/// Anything that is not a decodable `M…` strkey is returned unchanged: a `G…`
/// address is already the base account, and an address this build cannot parse
/// is better left visible than rewritten.
pub fn base_account(strkey: &str) -> String {
    use stellar_xdr::{MuxedAccount, PublicKey};
    match strkey.parse::<MuxedAccount>() {
        Ok(MuxedAccount::MuxedEd25519(m)) => PublicKey::PublicKeyTypeEd25519(m.ed25519).to_string(),
        _ => strkey.to_string(),
    }
}

/// The root contract a transaction invoked, as a strkey, if it invoked one.
pub fn root_contract_id(model: &TransactionModel) -> Option<String> {
    match model.invocation()?.contract {
        ScAddress::Contract(ref id) => Some(id.to_string()),
        _ => None,
    }
}

/// The terminal host error as its raw type and code, e.g. `Storage/ExceededLimit`.
///
/// Contract errors collapse to `Contract` with no number: the number is a
/// contract's own value, and the pilot groups by *kind* of failure.
pub fn terminal_error_key(error: &ScError) -> String {
    match error {
        ScError::Contract(_) => "Contract".to_string(),
        other => error_label(other)
            .trim_start_matches("Error(")
            .trim_end_matches(')')
            .replace(", ", "/"),
    }
}

/// The outcome of reading a root contract's instance entry.
///
/// Kept distinct from [`CodeIdentity`] so the pilot can tell a population fact
/// (the entry does not exist) from an artefact of the run (the lookup failed).
/// Both map to `code_identity_instance_unavailable` in the protocol, but only
/// the first says anything about the population.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceLookup {
    /// The entry was read and decoded.
    Found(ContractExecutable),
    /// The provider answered that no live entry exists.
    NotFound,
    /// No answer could be obtained, or the answer did not decode.
    Failed,
}

impl InstanceLookup {
    /// The executable, if the entry was read.
    pub fn executable(&self) -> Option<&ContractExecutable> {
        match self {
            Self::Found(executable) => Some(executable),
            Self::NotFound | Self::Failed => None,
        }
    }

    /// The status as written to the pilot's JSONL.
    pub fn status(&self) -> &'static str {
        match self {
            Self::Found(_) => "found",
            Self::NotFound => "not_found",
            Self::Failed => "failed",
        }
    }
}

/// One line of the M5.2 population pilot's JSONL output.
///
/// This is a *population* record, not an evaluation sample. It exists to
/// measure how many distinct clusters a retention window holds. It therefore
/// carries no cause class, verdict, confidence or rule id — a test asserts the
/// field set, because a pilot that recorded SDO's own opinion could quietly
/// become the thing the evaluation is measured against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PilotRecord {
    /// The transaction hash as reported by RPC (the outer hash for a fee bump).
    pub transaction_hash: String,
    /// The inner transaction's hash, for a fee bump. Both hashes go on the
    /// pilot's exclusion list, because the protocol checks both (§4.2, check 5).
    pub inner_transaction_hash: Option<String>,
    /// The ledger it was applied in.
    pub ledger: u32,
    /// Whether the envelope is a fee bump.
    pub fee_bumped: bool,
    /// Operations in the inner transaction (eligibility check 3 wants one).
    pub operation_count: usize,
    /// Diagnostic events decoded (eligibility check 4 wants at least one).
    pub diagnostic_event_count: usize,
    /// Protocol §5.2.
    pub submitter_cluster: String,
    /// The invoked contract, if the operation was an `InvokeContract`.
    pub root_contract_id: Option<String>,
    /// `found`, `not_found` or `failed`: how the root contract's instance
    /// lookup went. Absent when there is no root contract.
    pub instance_lookup: Option<String>,
    /// Protocol §5.1, when it resolves.
    pub code_cluster: Option<String>,
    /// Why §5.1 did not resolve, when it did not.
    pub code_unresolved_reason: Option<UnresolvedCode>,
    /// The raw terminal host error type and code, if the host reported one.
    pub terminal_error: Option<String>,
}

impl PilotRecord {
    /// Build a record from a decoded transaction and the lookup of its root
    /// contract's instance (`None` when there is no root contract to look up).
    pub fn new(
        transaction_hash: &str,
        ledger: u32,
        model: &TransactionModel,
        lookup: Option<&InstanceLookup>,
    ) -> Self {
        let instance = lookup.and_then(InstanceLookup::executable);
        let footprint = model
            .soroban
            .as_ref()
            .map(|s| s.footprint.contract_code_hashes())
            .unwrap_or_default();
        let root = root_contract_id(model);
        let (code_cluster_key, unresolved) = match &root {
            // Not an `InvokeContract` call: there is no root contract to cluster
            // by, and the protocol excludes it before §5.1 applies.
            None => (None, None),
            Some(_) => match code_cluster(instance, &footprint) {
                CodeIdentity::Resolved(key) => (Some(key), None),
                CodeIdentity::Unresolved(reason) => (None, Some(reason)),
            },
        };
        Self {
            transaction_hash: transaction_hash.to_string(),
            inner_transaction_hash: model
                .outcome
                .fee_bump
                .as_ref()
                .map(|bump| hex(&bump.inner_transaction_hash)),
            ledger,
            fee_bumped: model.is_fee_bumped(),
            operation_count: model.operations.len(),
            diagnostic_event_count: model.diagnostics.events.len(),
            submitter_cluster: submitter_cluster(model),
            instance_lookup: root.as_ref().and(lookup).map(|l| l.status().to_string()),
            root_contract_id: root,
            code_cluster: code_cluster_key,
            code_unresolved_reason: unresolved,
            terminal_error: model
                .diagnostics
                .terminal_error
                .as_ref()
                .map(|t| terminal_error_key(&t.error)),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_xdr::{Hash, ScErrorCode};

    const WASM: [u8; 32] = [0xab; 32];

    /// An executable kind protocol v1 does not cluster.
    fn external_ref() -> ContractExecutable {
        use stellar_xdr::{ContractExecutableExternalRef, ContractId, ScAddress};
        ContractExecutable::ExternalRef(ContractExecutableExternalRef {
            executable_owner: ScAddress::Contract(ContractId(Hash([0; 32]))),
            tag: stellar_xdr::ScString(stellar_xdr::StringM::default()),
        })
    }

    #[test]
    fn a_wasm_hash_in_the_footprint_resolves_to_that_hash() {
        let identity = code_cluster(
            Some(&ContractExecutable::Wasm(Hash(WASM))),
            &[[0x11; 32], WASM],
        );
        assert_eq!(
            identity,
            CodeIdentity::Resolved(format!("wasm:{}", "ab".repeat(32)))
        );
    }

    #[test]
    fn a_wasm_hash_absent_from_the_footprint_never_becomes_a_cluster() {
        assert_eq!(
            code_cluster(Some(&ContractExecutable::Wasm(Hash(WASM))), &[[0x11; 32]]),
            CodeIdentity::Unresolved(UnresolvedCode::NotInFootprint)
        );
        assert_eq!(
            code_cluster(Some(&ContractExecutable::Wasm(Hash(WASM))), &[]),
            CodeIdentity::Unresolved(UnresolvedCode::NotInFootprint)
        );
    }

    #[test]
    fn the_stellar_asset_contract_is_one_shared_cluster_with_no_footprint_check() {
        assert_eq!(
            code_cluster(Some(&ContractExecutable::StellarAsset), &[]),
            CodeIdentity::Resolved(STELLAR_ASSET_CLUSTER.to_string())
        );
    }

    #[test]
    fn an_unreadable_instance_is_unresolved_and_says_why() {
        assert_eq!(
            code_cluster(None, &[WASM]),
            CodeIdentity::Unresolved(UnresolvedCode::InstanceUnavailable)
        );
    }

    #[test]
    fn an_executable_kind_this_version_does_not_cluster_is_unresolved() {
        // Protocol v1 clusters WASM and the built-in SAC. Anything else is
        // recorded as unsupported rather than guessed at.
        assert_eq!(
            code_cluster(Some(&external_ref()), &[WASM]),
            CodeIdentity::Unresolved(UnresolvedCode::UnsupportedExecutable)
        );
    }

    #[test]
    fn no_unresolved_identity_ever_yields_a_cluster_key() {
        // The one property the whole deduplication scheme rests on.
        for identity in [
            code_cluster(None, &[]),
            code_cluster(Some(&ContractExecutable::Wasm(Hash(WASM))), &[]),
        ] {
            match identity {
                CodeIdentity::Unresolved(_) => {}
                CodeIdentity::Resolved(key) => panic!("unresolved identity produced {key}"),
            }
        }
    }

    #[test]
    fn unresolved_reasons_match_the_protocols_exclusion_codes() {
        for (reason, code) in [
            (
                UnresolvedCode::InstanceUnavailable,
                "code_identity_instance_unavailable",
            ),
            (
                UnresolvedCode::NotInFootprint,
                "code_identity_not_in_footprint",
            ),
            (
                UnresolvedCode::UnsupportedExecutable,
                "code_identity_unsupported_executable",
            ),
        ] {
            assert_eq!(reason.reason(), code);
            assert_eq!(reason.to_string(), code);
            assert_eq!(serde_json::to_value(reason).unwrap(), code);
        }
    }

    #[test]
    fn a_muxed_account_reduces_to_its_underlying_account() {
        // A SEP-23 muxed address and the account underneath it. Both halves
        // were checked by decoding the strkey by hand, independently of the
        // library this function uses.
        let muxed = "MA7QYNF7SOWQ3GLR2BGMZEHXAVIRZA4KVWLTJJFC7MGXUA74P7UJVAAAAAAAAAAAAAJLK";
        let base = "GA7QYNF7SOWQ3GLR2BGMZEHXAVIRZA4KVWLTJJFC7MGXUA74P7UJVSGZ";
        assert_eq!(base_account(muxed), base);
        assert_eq!(base_account(base), base, "a G… address is already the base");
    }

    #[test]
    fn an_unparseable_address_is_left_visible_rather_than_rewritten() {
        assert_eq!(base_account("not an address"), "not an address");
    }

    #[test]
    fn terminal_errors_are_raw_types_and_codes_with_contract_codes_collapsed() {
        assert_eq!(
            terminal_error_key(&ScError::Storage(ScErrorCode::ExceededLimit)),
            "Storage/ExceededLimit"
        );
        assert_eq!(
            terminal_error_key(&ScError::Budget(ScErrorCode::ExceededLimit)),
            "Budget/ExceededLimit"
        );
        assert_eq!(terminal_error_key(&ScError::Contract(9)), "Contract");
        // Every host error renders as `Type/Code`, never as a sentence.
        for error in [
            ScError::WasmVm(ScErrorCode::InternalError),
            ScError::Auth(ScErrorCode::InvalidAction),
            ScError::Value(ScErrorCode::UnexpectedType),
        ] {
            let key = terminal_error_key(&error);
            assert_eq!(key.matches('/').count(), 1, "{key}");
        }
    }

    #[test]
    fn the_pilot_record_carries_no_sdo_derived_field() {
        // If this ever fails, the pilot has started recording an opinion rather
        // than an observation. See the type's documentation.
        let record = PilotRecord {
            transaction_hash: "abc".into(),
            inner_transaction_hash: None,
            ledger: 1,
            fee_bumped: false,
            operation_count: 1,
            diagnostic_event_count: 0,
            submitter_cluster: "G".into(),
            root_contract_id: None,
            instance_lookup: None,
            code_cluster: None,
            code_unresolved_reason: None,
            terminal_error: None,
        };
        let json = serde_json::to_value(&record).unwrap();
        let fields: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        // `serde_json` orders object keys, so compare the set, sorted.
        assert_eq!(
            fields,
            [
                "code_cluster",
                "code_unresolved_reason",
                "diagnostic_event_count",
                "fee_bumped",
                "inner_transaction_hash",
                "instance_lookup",
                "ledger",
                "operation_count",
                "root_contract_id",
                "submitter_cluster",
                "terminal_error",
                "transaction_hash",
            ]
        );
        for forbidden in [
            "cause",
            "class",
            "verdict",
            "confidence",
            "rule",
            "diagnosis",
            "explanation",
        ] {
            assert!(
                !fields.iter().any(|f| f.contains(forbidden)),
                "pilot records must not carry `{forbidden}`"
            );
        }
    }
}
