//! Contract error resolution (milestone M3).
//!
//! Turns `Error(Contract, #2)` into `NoHarvestablePails` — using only the name
//! the contract itself declares in its spec, and only when the evidence says
//! which contract raised it and that the spec matches the code that ran.
//!
//! This module is pure. It never fetches a spec. The caller supplies specs as
//! data through [`crate::AnalysisInput`]; `soroban-failure-rpc` provides the
//! fetching. [`contracts_needing_specs`] tells the caller what to fetch.
//!
//! * [`wasm`](mod@wasm) — find the `contractspecv0` section in contract WASM
//! * [`spec`](mod@spec) — decode error enums from it
//! * [`resolve`](mod@resolve) — identify the contract, check provenance, name the error

pub mod resolve;
pub mod spec;
pub mod wasm;

pub use resolve::{
    contracts_needing_specs, identify, resolve, resolve_contract_errors, ContractErrorReport,
    ContractIdentification, ErrorResolution, IdentificationBasis, SpecAvailability, SpecProvenance,
    ORIGIN_MARKER,
};
pub use spec::{ContractSpec, ErrorCase, ErrorEnum, ErrorLookup, SpecError};
pub use wasm::WasmError;
