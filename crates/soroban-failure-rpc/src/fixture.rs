//! Loading recorded RPC responses from disk.
//!
//! A fixture is a verbatim recording of a `getTransaction` result. Fixtures are
//! what make the analysis engine testable without network access, which in turn
//! is what makes this project contributable by people who do not want to run a
//! node or spend a rate limit. See `fixtures/README.md`.

use std::path::Path;

use serde_json::Value;

use crate::decode::{decode_get_transaction, DecodedTransaction};
use crate::error::DecodeError;

/// Something went wrong loading a fixture from disk.
#[derive(Debug, thiserror::Error)]
pub enum FixtureError {
    /// The file could not be read.
    #[error("could not read fixture at {path}: {source}")]
    Io {
        /// The path that failed.
        path: String,
        /// The underlying I/O error.
        source: std::io::Error,
    },

    /// The file was not valid JSON.
    #[error("fixture at {path} is not valid JSON: {source}")]
    Json {
        /// The path that failed.
        path: String,
        /// The underlying parse error.
        source: serde_json::Error,
    },

    /// The JSON was valid but could not be decoded as a transaction.
    #[error(transparent)]
    Decode(#[from] DecodeError),
}

/// The filename every fixture directory must contain.
pub const RESPONSE_FILE: &str = "rpc-response.json";

/// Read and parse the recorded response of a fixture without decoding it.
pub fn load_raw(dir: impl AsRef<Path>) -> Result<Value, FixtureError> {
    let path = dir.as_ref().join(RESPONSE_FILE);
    let display = path.display().to_string();

    let bytes = std::fs::read(&path).map_err(|source| FixtureError::Io {
        path: display.clone(),
        source,
    })?;

    serde_json::from_slice(&bytes).map_err(|source| FixtureError::Json {
        path: display,
        source,
    })
}

/// Load a fixture directory and decode it into typed analysis input.
///
/// The transaction hash is read from the recorded response itself, so a fixture
/// cannot drift out of sync with its own directory name.
pub fn load(dir: impl AsRef<Path>) -> Result<DecodedTransaction, FixtureError> {
    let raw = load_raw(&dir)?;
    let hash = raw
        .get("txHash")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok(decode_get_transaction(&hash, &raw)?)
}

/// Contract specs from recorded `getLedgerEntries` responses.
///
/// Expects the layout under `fixtures/contracts/`:
///
/// ```text
/// <root>/<CONTRACT_ID>/instance.json   getLedgerEntries result, instance key
/// <root>/<CONTRACT_ID>/code.json       getLedgerEntries result, code key
/// ```
///
/// Both files are decoded by the same functions as live responses. A contract
/// with no directory is reported as not found, exactly as an absent ledger
/// entry would be.
pub struct FixtureContractSource {
    root: std::path::PathBuf,
}

impl FixtureContractSource {
    /// Read contract fixtures from `root`.
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn read(path: &Path) -> Result<Value, crate::contract::SourceError> {
        let bytes = std::fs::read(path).map_err(|e| {
            crate::contract::SourceError::Lookup(format!("{}: {e}", path.display()))
        })?;
        serde_json::from_slice(&bytes).map_err(|e| {
            crate::contract::SourceError::Malformed(format!("{}: {e}", path.display()))
        })
    }
}

impl crate::contract::ContractSource for FixtureContractSource {
    fn contract_executable(
        &self,
        contract: &stellar_xdr::ContractId,
    ) -> Result<stellar_xdr::ContractExecutable, crate::contract::SourceError> {
        let path = self.root.join(contract.to_string()).join("instance.json");
        if !path.is_file() {
            return Err(crate::contract::SourceError::NotFound(
                "contract instance fixture",
            ));
        }
        crate::contract::decode_instance(&Self::read(&path)?)
    }

    fn contract_wasm(
        &self,
        wasm_hash: &stellar_xdr::Hash,
    ) -> Result<Vec<u8>, crate::contract::SourceError> {
        use stellar_xdr::WriteXdr;

        // Code is keyed by hash, not contract, so find the recording whose
        // ledger key is this hash's code key.
        let wanted = crate::contract::code_key(wasm_hash)
            .to_xdr_base64(stellar_xdr::Limits::none())
            .map_err(|e| crate::contract::SourceError::Lookup(e.to_string()))?;
        let dirs = std::fs::read_dir(&self.root).map_err(|e| {
            crate::contract::SourceError::Lookup(format!("{}: {e}", self.root.display()))
        })?;
        let mut paths: Vec<_> = dirs
            .filter_map(Result::ok)
            .map(|d| d.path().join("code.json"))
            .filter(|p| p.is_file())
            .collect();
        paths.sort();
        for path in paths {
            let recorded = Self::read(&path)?;
            let key = recorded
                .get("entries")
                .and_then(|e| e.get(0))
                .and_then(|e| e.get("key"))
                .and_then(Value::as_str);
            if key == Some(wanted.as_str()) {
                return crate::contract::decode_code(&recorded);
            }
        }
        Err(crate::contract::SourceError::NotFound(
            "contract code fixture",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fixture_reports_the_path_it_looked_for() {
        let err = load_raw("does/not/exist").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains(RESPONSE_FILE),
            "error should name the file: {msg}"
        );
    }
}
