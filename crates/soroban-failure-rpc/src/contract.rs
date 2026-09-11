//! Obtaining contract specs for error resolution (milestone M3).
//!
//! The analysis engine names contract errors from specs it is *given*. This
//! module is how they are got:
//!
//! ```text
//! contract ID
//!   └─ getLedgerEntries(ContractData instance key)  → ContractExecutable::Wasm(hash)
//!        └─ getLedgerEntries(ContractCode key)      → WASM bytes
//!             └─ ContractSpec::from_wasm            → error enums   (pure, in the engine)
//! ```
//!
//! [`ContractSource`] abstracts the two lookups so the same orchestration runs
//! against the network ([`RpcClient`]) or against recorded responses
//! ([`crate::fixture::FixtureContractSource`]). Both decode through the same
//! pure functions here, so fixture tests exercise real decoding.

use std::collections::BTreeMap;

use serde_json::Value;
use soroban_failure_analysis::contract::{ContractSpec, SpecAvailability};
use stellar_xdr::{
    ContractDataDurability, ContractExecutable, ContractId, Hash, LedgerEntryData, LedgerKey,
    LedgerKeyContractCode, LedgerKeyContractData, Limits, ReadXdr, ScAddress, ScVal, WriteXdr,
};

use crate::client::RpcClient;

/// A contract's code could not be obtained.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceError {
    /// No live entry exists for the key. The contract may not exist, may be on
    /// another network, or its entry may be archived.
    #[error("{0} not found; it may not exist on this network, or may be archived")]
    NotFound(&'static str),
    /// The contract is not backed by WASM, so it has no on-chain spec.
    #[error("{0}")]
    NoWasm(String),
    /// The lookup failed.
    #[error("lookup failed: {0}")]
    Lookup(String),
    /// A response arrived but did not decode as expected.
    #[error("malformed ledger entry: {0}")]
    Malformed(String),
}

/// Somewhere contract executables and WASM can be read from.
pub trait ContractSource {
    /// The executable of a contract instance.
    fn contract_executable(&self, contract: &ContractId)
        -> Result<ContractExecutable, SourceError>;

    /// The WASM stored under a code hash.
    fn contract_wasm(&self, wasm_hash: &Hash) -> Result<Vec<u8>, SourceError>;
}

/// The ledger key of a contract's instance entry.
pub fn instance_key(contract: &ContractId) -> LedgerKey {
    LedgerKey::ContractData(LedgerKeyContractData {
        contract: ScAddress::Contract(contract.clone()),
        key: ScVal::LedgerKeyContractInstance,
        durability: ContractDataDurability::Persistent,
    })
}

/// The ledger key of a WASM code entry.
pub fn code_key(wasm_hash: &Hash) -> LedgerKey {
    LedgerKey::ContractCode(LedgerKeyContractCode {
        hash: wasm_hash.clone(),
    })
}

/// The single `LedgerEntryData` in a `getLedgerEntries` result.
fn single_entry(result: &Value, what: &'static str) -> Result<LedgerEntryData, SourceError> {
    let entries = result
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Malformed("result has no `entries` array".into()))?;
    let entry = match entries.as_slice() {
        [] => return Err(SourceError::NotFound(what)),
        [one] => one,
        more => {
            return Err(SourceError::Malformed(format!(
                "expected one entry, got {}",
                more.len()
            )))
        }
    };
    let xdr = entry
        .get("xdr")
        .and_then(Value::as_str)
        .ok_or_else(|| SourceError::Malformed("entry has no `xdr` field".into()))?;
    LedgerEntryData::from_xdr_base64(xdr, Limits::none())
        .map_err(|e| SourceError::Malformed(e.to_string()))
}

/// Decode a `getLedgerEntries` result for an instance key. Pure.
pub fn decode_instance(result: &Value) -> Result<ContractExecutable, SourceError> {
    match single_entry(result, "contract instance")? {
        LedgerEntryData::ContractData(d) => match d.val {
            ScVal::ContractInstance(instance) => Ok(instance.executable),
            other => Err(SourceError::Malformed(format!(
                "instance entry holds {} rather than a contract instance",
                other.name()
            ))),
        },
        other => Err(SourceError::Malformed(format!(
            "expected contract data, got {}",
            other.name()
        ))),
    }
}

/// Decode a `getLedgerEntries` result for a code key. Pure.
pub fn decode_code(result: &Value) -> Result<Vec<u8>, SourceError> {
    match single_entry(result, "contract code")? {
        LedgerEntryData::ContractCode(c) => Ok(c.code.to_vec()),
        other => Err(SourceError::Malformed(format!(
            "expected contract code, got {}",
            other.name()
        ))),
    }
}

fn get_ledger_entry(client: &RpcClient, key: &LedgerKey) -> Result<Value, SourceError> {
    let key = key
        .to_xdr_base64(Limits::none())
        .map_err(|e| SourceError::Lookup(e.to_string()))?;
    client
        .call("getLedgerEntries", serde_json::json!({ "keys": [key] }))
        .map_err(|e| SourceError::Lookup(e.to_string()))
}

impl RpcClient {
    /// The raw `getLedgerEntries` result for a contract's instance.
    pub fn contract_instance_entry(&self, contract: &ContractId) -> Result<Value, SourceError> {
        get_ledger_entry(self, &instance_key(contract))
    }

    /// The raw `getLedgerEntries` result for a WASM code entry.
    pub fn contract_code_entry(&self, wasm_hash: &Hash) -> Result<Value, SourceError> {
        get_ledger_entry(self, &code_key(wasm_hash))
    }
}

impl ContractSource for RpcClient {
    fn contract_executable(
        &self,
        contract: &ContractId,
    ) -> Result<ContractExecutable, SourceError> {
        decode_instance(&self.contract_instance_entry(contract)?)
    }

    fn contract_wasm(&self, wasm_hash: &Hash) -> Result<Vec<u8>, SourceError> {
        decode_code(&self.contract_code_entry(wasm_hash)?)
    }
}

/// Obtain a spec — or the reason there is none — for each contract.
///
/// Contracts sharing a WASM hash are fetched and parsed once. That per-call
/// map is the only caching: there is deliberately no persistent cache.
pub fn fetch_specs(
    source: &dyn ContractSource,
    contracts: &[ContractId],
) -> BTreeMap<ContractId, SpecAvailability> {
    let mut by_hash: BTreeMap<[u8; 32], SpecAvailability> = BTreeMap::new();
    let mut out = BTreeMap::new();

    for contract in contracts {
        let availability = match source.contract_executable(contract) {
            Ok(ContractExecutable::Wasm(hash)) => by_hash
                .entry(hash.0)
                .or_insert_with(|| spec_for(source, &hash))
                .clone(),
            Ok(ContractExecutable::StellarAsset) => SpecAvailability::Unavailable {
                // The Stellar Asset Contract is built into the host. Its error
                // codes are real, but it has no on-chain spec to read them from,
                // and names are only ever taken from a spec.
                reason: "Stellar Asset Contract: built into the host, with no on-chain WASM spec"
                    .into(),
            },
            Ok(other) => SpecAvailability::Unavailable {
                reason: format!("contract executable `{}` is not supported", other.name()),
            },
            Err(e) => SpecAvailability::Unavailable {
                reason: e.to_string(),
            },
        };
        out.insert(contract.clone(), availability);
    }
    out
}

fn spec_for(source: &dyn ContractSource, hash: &Hash) -> SpecAvailability {
    match source.contract_wasm(hash) {
        Ok(wasm) => match ContractSpec::from_wasm(&wasm) {
            Ok(spec) => SpecAvailability::Available(spec.with_wasm_hash(hash.0)),
            Err(e) => SpecAvailability::Unavailable {
                reason: e.to_string(),
            },
        },
        Err(e) => SpecAvailability::Unavailable {
            reason: e.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::Cell;

    /// An in-memory source that counts WASM fetches.
    struct Stub {
        executable: ContractExecutable,
        wasm: Result<Vec<u8>, SourceError>,
        wasm_fetches: Cell<usize>,
    }

    impl ContractSource for Stub {
        fn contract_executable(&self, _: &ContractId) -> Result<ContractExecutable, SourceError> {
            Ok(self.executable.clone())
        }
        fn contract_wasm(&self, _: &Hash) -> Result<Vec<u8>, SourceError> {
            self.wasm_fetches.set(self.wasm_fetches.get() + 1);
            self.wasm.clone()
        }
    }

    fn cid(n: u8) -> ContractId {
        ContractId(Hash([n; 32]))
    }

    #[test]
    fn contracts_sharing_wasm_fetch_it_once() {
        let stub = Stub {
            executable: ContractExecutable::Wasm(Hash([1; 32])),
            wasm: Ok(b"not wasm".to_vec()),
            wasm_fetches: Cell::new(0),
        };
        let specs = fetch_specs(&stub, &[cid(1), cid(2)]);
        assert_eq!(specs.len(), 2);
        assert_eq!(stub.wasm_fetches.get(), 1);
    }

    #[test]
    fn unparseable_wasm_is_unavailable_with_a_reason() {
        let stub = Stub {
            executable: ContractExecutable::Wasm(Hash([1; 32])),
            wasm: Ok(b"not wasm".to_vec()),
            wasm_fetches: Cell::new(0),
        };
        let specs = fetch_specs(&stub, &[cid(1)]);
        assert!(matches!(
            &specs[&cid(1)],
            SpecAvailability::Unavailable { reason } if reason.contains("WebAssembly")
        ));
    }

    #[test]
    fn stellar_asset_contract_is_unavailable_not_named_from_a_hardcoded_table() {
        let stub = Stub {
            executable: ContractExecutable::StellarAsset,
            wasm: Err(SourceError::NotFound("unused")),
            wasm_fetches: Cell::new(0),
        };
        let specs = fetch_specs(&stub, &[cid(1)]);
        assert!(matches!(
            &specs[&cid(1)],
            SpecAvailability::Unavailable { reason } if reason.contains("Stellar Asset Contract")
        ));
        assert_eq!(stub.wasm_fetches.get(), 0);
    }

    #[test]
    fn lookup_failure_is_unavailable_with_the_reason() {
        let stub = Stub {
            executable: ContractExecutable::Wasm(Hash([1; 32])),
            wasm: Err(SourceError::NotFound("contract code")),
            wasm_fetches: Cell::new(0),
        };
        let specs = fetch_specs(&stub, &[cid(1)]);
        assert!(matches!(
            &specs[&cid(1)],
            SpecAvailability::Unavailable { reason } if reason.contains("not found")
        ));
    }

    #[test]
    fn empty_entries_means_not_found() {
        let err = decode_instance(&json!({ "entries": [] })).unwrap_err();
        assert_eq!(err, SourceError::NotFound("contract instance"));
    }

    #[test]
    fn malformed_entries_are_errors_not_panics() {
        assert!(matches!(
            decode_code(&json!({})),
            Err(SourceError::Malformed(_))
        ));
        assert!(matches!(
            decode_code(&json!({ "entries": [{ "xdr": "!!!" }] })),
            Err(SourceError::Malformed(_))
        ));
        assert!(matches!(
            decode_code(&json!({ "entries": [{}, {}] })),
            Err(SourceError::Malformed(_))
        ));
    }

    #[test]
    fn keys_round_trip_through_xdr() {
        let k = instance_key(&cid(3));
        let b64 = k.to_xdr_base64(Limits::none()).unwrap();
        assert_eq!(LedgerKey::from_xdr_base64(b64, Limits::none()).unwrap(), k);
    }
}
