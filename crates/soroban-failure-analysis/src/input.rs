//! The typed input boundary of the analysis engine.
//!
//! Everything the engine needs must arrive through [`AnalysisInput`]. The engine
//! never fetches anything itself — that is what makes it deterministic and
//! testable from committed fixtures with no network.
//!
//! Construct one with [`AnalysisInput::builder`].

use std::collections::BTreeMap;

use stellar_xdr::{
    ContractId, DiagnosticEvent, TransactionEnvelope, TransactionMeta, TransactionResult,
};

use crate::contract::SpecAvailability;

/// Decoded transaction artifacts for a single failed transaction.
///
/// Marked `#[non_exhaustive]`: later milestones will add fields (the contract
/// spec in M3, for example). Use [`AnalysisInput::builder`] so those additions
/// are not breaking changes for downstream consumers.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AnalysisInput {
    /// Hex-encoded transaction hash, if known. Purely for reporting.
    pub transaction_hash: Option<String>,
    /// The submitted transaction envelope.
    pub envelope: TransactionEnvelope,
    /// The transaction result.
    pub result: TransactionResult,
    /// Transaction metadata, when the caller had it.
    pub meta: Option<TransactionMeta>,
    /// Diagnostic events, in the order returned.
    ///
    /// An empty vector is ambiguous on its own — it may mean "the node does not
    /// emit diagnostic events" or "there genuinely were none". Read
    /// [`AnalysisInput::diagnostics_enabled`] to tell those apart.
    pub diagnostic_events: Vec<DiagnosticEvent>,
    /// Whether the source node was known to emit diagnostic events at all.
    ///
    /// `false` means their absence carries no information, and rules that depend
    /// on them must decline to fire rather than concluding from silence.
    pub diagnostics_enabled: bool,
    /// Contract specs, keyed by contract, for naming contract errors (M3).
    ///
    /// Supplied by the caller: the engine never fetches them. A contract absent
    /// from this map is reported as "no spec supplied", never guessed at. Use
    /// [`crate::contract::contracts_needing_specs`] to learn which to fetch.
    pub contract_specs: BTreeMap<ContractId, SpecAvailability>,
}

impl AnalysisInput {
    /// Start building an input from the two artifacts that are always required.
    pub fn builder(
        envelope: TransactionEnvelope,
        result: TransactionResult,
    ) -> AnalysisInputBuilder {
        AnalysisInputBuilder {
            inner: AnalysisInput {
                transaction_hash: None,
                envelope,
                result,
                meta: None,
                diagnostic_events: Vec::new(),
                diagnostics_enabled: false,
                contract_specs: BTreeMap::new(),
            },
        }
    }

    /// Whether diagnostic-event evidence is actually usable for this input.
    ///
    /// Rules that reason from diagnostic events must check this first. Absence
    /// of evidence is not evidence of absence when the node never emitted any.
    pub fn has_diagnostic_evidence(&self) -> bool {
        self.diagnostics_enabled && !self.diagnostic_events.is_empty()
    }

    /// Attach contract specs obtained after decoding.
    ///
    /// Specs are usually fetched *after* the transaction is decoded, because
    /// which contracts need them depends on its diagnostic events.
    pub fn with_contract_specs(mut self, specs: BTreeMap<ContractId, SpecAvailability>) -> Self {
        self.contract_specs.extend(specs);
        self
    }
}

/// Builder for [`AnalysisInput`].
#[derive(Debug, Clone)]
pub struct AnalysisInputBuilder {
    inner: AnalysisInput,
}

impl AnalysisInputBuilder {
    /// Attach the hex-encoded transaction hash.
    pub fn transaction_hash(mut self, hash: impl Into<String>) -> Self {
        self.inner.transaction_hash = Some(hash.into());
        self
    }

    /// Attach transaction metadata.
    pub fn meta(mut self, meta: TransactionMeta) -> Self {
        self.inner.meta = Some(meta);
        self
    }

    /// Attach diagnostic events, and record that the node emits them.
    ///
    /// Calling this — even with an empty list — asserts that diagnostic events
    /// were available from the source, so their absence becomes meaningful.
    pub fn diagnostic_events(mut self, events: Vec<DiagnosticEvent>) -> Self {
        self.inner.diagnostic_events = events;
        self.inner.diagnostics_enabled = true;
        self
    }

    /// Attach one contract's spec, or the reason it is unavailable.
    pub fn contract_spec(mut self, contract: ContractId, spec: SpecAvailability) -> Self {
        self.inner.contract_specs.insert(contract, spec);
        self
    }

    /// Finish building.
    pub fn build(self) -> AnalysisInput {
        self.inner
    }
}
