//! Fetching and decoding Stellar transactions for failure analysis.
//!
//! This crate is the I/O boundary of the project. It has two halves, and the
//! split is deliberate:
//!
//! * [`decode`] and [`fixture`] are **pure** — they turn JSON into a typed
//!   [`soroban_failure_analysis::AnalysisInput`] and touch no network.
//! * [`client`] is the only part that makes network calls.
//!
//! Everything downstream can therefore be tested against committed fixtures
//! with no RPC endpoint, no rate limit, and no flakiness.
//!
//! # Status
//!
//! **Milestones M2–M3.** Fetching, decoding and fixture loading for
//! transactions, plus fetching contract specs so the engine can name contract
//! errors ([`contract`]). The spec *parsing and resolution* is pure and lives
//! in the analysis crate; this crate only obtains the bytes.

#![doc(html_root_url = "https://docs.rs/soroban-failure-rpc")]

pub mod client;
pub mod contract;
pub mod decode;
pub mod error;
pub mod fixture;

pub use client::RpcClient;
pub use contract::{fetch_specs, ContractSource, SourceError};
pub use decode::{decode_get_transaction, DecodedTransaction, DiagnosticSource, TransactionStatus};
pub use error::{DecodeError, RpcError};
pub use fixture::FixtureContractSource;
