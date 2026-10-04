//! The M0 measurement itself.
//!
//! Given a `getTransaction` result, determine what an analyzer would actually
//! have to work with. Every field here is an observation. Nothing is inferred
//! optimistically: when a field is absent we say absent, and when we cannot tell
//! we say so.

use serde::Serialize;

use crate::result_codes::{failure_signal, FailureSignal};
use serde_json::Value;
use stellar_xdr::{
    DiagnosticEvent, Limits, OperationBody, ReadXdr, TransactionEnvelope, TransactionMeta,
    TransactionResult,
};

/// Whether an artifact was present and whether it survived decoding.
///
/// The distinction matters: a field that is present but undecodable is a very
/// different (and worse) finding than a field that is simply absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE", tag = "state", content = "detail")]
pub enum ArtifactState {
    /// Present in the response and decoded successfully.
    Decoded,
    /// Present in the response but decoding failed. Carries the decoder error.
    DecodeFailed(String),
    /// Not present in the response at all.
    Absent,
}

impl ArtifactState {
    /// Decode a base64 XDR artifact of type `T`, recording the outcome.
    fn decode<T: ReadXdr>(field: Option<&str>) -> (Self, Option<T>) {
        match field {
            None => (Self::Absent, None),
            Some(b64) => {
                match T::from_xdr_base64(b64, soroban_failure_rpc::xdr::limits_for_base64(b64)) {
                    Ok(v) => (Self::Decoded, Some(v)),
                    Err(e) => (Self::DecodeFailed(e.to_string()), None),
                }
            }
        }
    }

    /// Whether the artifact is usable by an analyzer.
    pub fn is_usable(&self) -> bool {
        matches!(self, Self::Decoded)
    }

    /// A short label for human output.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Decoded => "AVAILABLE",
            Self::DecodeFailed(_) => "PRESENT BUT UNDECODABLE",
            Self::Absent => "NOT AVAILABLE",
        }
    }
}

/// Where in the RPC response the diagnostic events were found.
///
/// RPC has carried these in more than one place across versions, so the probe
/// records which shape a provider actually used rather than assuming one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticLocation {
    /// Top-level `diagnosticEventsXdr`.
    TopLevel,
    /// Nested `events.diagnosticEventsXdr`.
    EventsObject,
    /// Recovered from `resultMetaXdr`'s Soroban metadata.
    ResultMeta,
}

/// What the probe found out about diagnostic events.
#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticEventsReport {
    /// Whether any diagnostic events were returned at all.
    pub available: bool,
    /// Where they were found, if they were.
    pub location: Option<DiagnosticLocation>,
    /// How many were returned.
    pub count: usize,
    /// How many decoded cleanly.
    pub decoded_count: usize,
    /// Whether every returned event decoded.
    pub decoded: bool,
    /// Decoder errors, if any.
    pub decode_errors: Vec<String>,
}

/// The verdict for a single transaction on a single endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    /// Envelope, result, meta and decodable diagnostic events are all present.
    SufficientForAnalysis,
    /// Envelope, result and meta are present, but diagnostic events are not.
    /// Stage classification is possible; evidence-backed causal analysis is not.
    InsufficientForFullAnalysis,
    /// Core artifacts are missing or undecodable. Nothing meaningful is possible.
    Unusable,
    /// The endpoint did not have this transaction (outside its retention window,
    /// or wrong network).
    TransactionNotFound,
    /// The transaction succeeded — this probe measures failures.
    NotAFailedTransaction,
}

impl Verdict {
    /// A short label for human output.
    pub fn label(self) -> &'static str {
        match self {
            Self::SufficientForAnalysis => "SUFFICIENT_FOR_ANALYSIS",
            Self::InsufficientForFullAnalysis => "INSUFFICIENT_FOR_FULL_ANALYSIS",
            Self::Unusable => "UNUSABLE",
            Self::TransactionNotFound => "TRANSACTION_NOT_FOUND",
            Self::NotAFailedTransaction => "NOT_A_FAILED_TRANSACTION",
        }
    }
}

/// The full probe result for one transaction on one endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct ProbeReport {
    /// Schema version, so downstream consumers can detect format changes.
    pub schema: &'static str,
    /// The endpoint probed.
    pub rpc_url: String,
    /// Network passphrase reported by the endpoint, if it answered `getNetwork`.
    pub network_passphrase: Option<String>,
    /// The transaction hash probed.
    pub transaction: String,
    /// Transaction status as reported by RPC.
    pub status: String,
    /// The ledger the transaction was included in.
    pub ledger: Option<u64>,
    /// Whether the envelope contains a Soroban operation.
    pub is_soroban: bool,
    /// Which Soroban operation kind was found, if any.
    pub soroban_operation: Option<String>,
    /// State of `envelopeXdr`.
    pub envelope_xdr: ArtifactState,
    /// State of `resultXdr`.
    pub result_xdr: ArtifactState,
    /// State of `resultMetaXdr`.
    pub result_meta_xdr: ArtifactState,
    /// Which `TransactionMeta` union arm was returned (3 = pre-P23, 4 = P23+).
    pub meta_version: Option<u32>,
    /// Whether Soroban metadata was present inside the decoded transaction meta.
    pub soroban_meta_present: bool,
    /// Result-code names, when the result decoded.
    pub failure_signal: Option<FailureSignal>,
    /// Diagnostic event findings.
    pub diagnostic_events: DiagnosticEventsReport,
    /// The overall verdict.
    pub verdict: Verdict,
    /// Anything the probe could not determine, stated plainly.
    pub notes: Vec<String>,
}

/// Pull all base64 strings out of a JSON array field.
fn string_array(v: Option<&Value>) -> Option<Vec<String>> {
    let arr = v?.as_array()?;
    Some(
        arr.iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect(),
    )
}

/// Locate diagnostic events in a `getTransaction` result.
///
/// Checks both documented shapes rather than assuming one, then falls back to
/// the copy carried inside `resultMetaXdr`.
fn locate_diagnostic_events(
    result: &Value,
    meta: Option<&TransactionMeta>,
) -> (Option<DiagnosticLocation>, Vec<String>) {
    if let Some(v) = string_array(result.get("diagnosticEventsXdr")) {
        if !v.is_empty() {
            return (Some(DiagnosticLocation::TopLevel), v);
        }
    }
    if let Some(v) = string_array(
        result
            .get("events")
            .and_then(|e| e.get("diagnosticEventsXdr")),
    ) {
        if !v.is_empty() {
            return (Some(DiagnosticLocation::EventsObject), v);
        }
    }
    // Fall back to diagnostic events embedded in the transaction metadata.
    //
    // These live in different places by protocol version: in `TransactionMetaV3`
    // they sit inside `sorobanMeta`, but in `TransactionMetaV4` (protocol 23)
    // they were lifted to the top level of the meta and `SorobanTransactionMetaV2`
    // no longer carries events at all. Both are checked.
    let embedded: &[DiagnosticEvent] = match meta {
        Some(TransactionMeta::V3(m)) => m
            .soroban_meta
            .as_ref()
            .map(|s| s.diagnostic_events.as_slice())
            .unwrap_or(&[]),
        Some(TransactionMeta::V4(m)) => m.diagnostic_events.as_slice(),
        _ => &[],
    };

    if !embedded.is_empty() {
        use stellar_xdr::WriteXdr;
        let encoded: Vec<String> = embedded
            .iter()
            .filter_map(|e| e.to_xdr_base64(Limits::none()).ok())
            .collect();
        if !encoded.is_empty() {
            return (Some(DiagnosticLocation::ResultMeta), encoded);
        }
    }

    (None, Vec::new())
}

/// The `TransactionMeta` union arm the endpoint returned.
///
/// Worth recording: the meta layout changed materially at protocol 23, and an
/// analyzer that only understands one arm will silently see nothing.
fn meta_version(meta: &TransactionMeta) -> u32 {
    match meta {
        TransactionMeta::V0(_) => 0,
        TransactionMeta::V1(_) => 1,
        TransactionMeta::V2(_) => 2,
        TransactionMeta::V3(_) => 3,
        TransactionMeta::V4(_) => 4,
    }
}

/// Identify the Soroban operation in an envelope, if there is one.
pub fn soroban_operation_of(envelope: &TransactionEnvelope) -> Option<String> {
    let operations = match envelope {
        TransactionEnvelope::Tx(e) => e.tx.operations.as_slice(),
        TransactionEnvelope::TxV0(e) => e.tx.operations.as_slice(),
        TransactionEnvelope::TxFeeBump(e) => {
            let stellar_xdr::FeeBumpTransactionInnerTx::Tx(inner) = &e.tx.inner_tx;
            inner.tx.operations.as_slice()
        }
    };

    operations.iter().find_map(|op| match &op.body {
        OperationBody::InvokeHostFunction(_) => Some("invoke_host_function".to_string()),
        OperationBody::ExtendFootprintTtl(_) => Some("extend_footprint_ttl".to_string()),
        OperationBody::RestoreFootprint(_) => Some("restore_footprint".to_string()),
        _ => None,
    })
}

/// Whether decoded metadata carries Soroban-specific metadata.
fn has_soroban_meta(meta: &TransactionMeta) -> bool {
    match meta {
        TransactionMeta::V3(m) => m.soroban_meta.is_some(),
        TransactionMeta::V4(m) => m.soroban_meta.is_some(),
        _ => false,
    }
}

/// Run the probe against one `getTransaction` result.
pub fn probe_transaction(
    rpc_url: &str,
    network_passphrase: Option<String>,
    tx_hash: &str,
    result: &Value,
) -> ProbeReport {
    let mut notes = Vec::new();

    let status = result
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("UNKNOWN")
        .to_string();

    let ledger = result.get("ledger").and_then(Value::as_u64);

    let (envelope_xdr, envelope) = ArtifactState::decode::<TransactionEnvelope>(
        result.get("envelopeXdr").and_then(Value::as_str),
    );
    let (result_xdr, result_decoded) =
        ArtifactState::decode::<TransactionResult>(result.get("resultXdr").and_then(Value::as_str));
    let (result_meta_xdr, meta) = ArtifactState::decode::<TransactionMeta>(
        result.get("resultMetaXdr").and_then(Value::as_str),
    );

    if let ArtifactState::DecodeFailed(e) = &result_meta_xdr {
        notes.push(format!(
            "resultMetaXdr present but failed to decode with the pinned stellar-xdr: {e}"
        ));
    }

    let soroban_op = envelope.as_ref().and_then(soroban_operation_of);
    let is_soroban = soroban_op.is_some();
    let soroban_meta_present = meta.as_ref().is_some_and(has_soroban_meta);
    let meta_version = meta.as_ref().map(meta_version);

    let failure_signal = result_decoded.as_ref().map(failure_signal);

    let (location, raw_events) = locate_diagnostic_events(result, meta.as_ref());

    let mut decoded_count = 0usize;
    let mut decode_errors = Vec::new();
    for (i, b64) in raw_events.iter().enumerate() {
        match DiagnosticEvent::from_xdr_base64(
            b64,
            soroban_failure_rpc::xdr::limits_for_base64(b64),
        ) {
            Ok(_) => decoded_count += 1,
            Err(e) => decode_errors.push(format!("event {i}: {e}")),
        }
    }

    let diagnostic_events = DiagnosticEventsReport {
        available: !raw_events.is_empty(),
        location,
        count: raw_events.len(),
        decoded_count,
        decoded: !raw_events.is_empty() && decode_errors.is_empty(),
        decode_errors,
    };

    let verdict = if status == "NOT_FOUND" {
        notes.push(
            "Transaction not found on this endpoint. It may be outside the retention \
             window, or the endpoint may serve a different network."
                .into(),
        );
        Verdict::TransactionNotFound
    } else if status == "SUCCESS" {
        Verdict::NotAFailedTransaction
    } else if !envelope_xdr.is_usable() || !result_xdr.is_usable() {
        Verdict::Unusable
    } else if diagnostic_events.decoded {
        Verdict::SufficientForAnalysis
    } else {
        Verdict::InsufficientForFullAnalysis
    };

    if status == "FAILED" && !is_soroban {
        notes.push(
            "Transaction failed but contains no Soroban operation; diagnostic events \
             are not expected for classic transactions."
                .into(),
        );
    }

    if verdict == Verdict::InsufficientForFullAnalysis && is_soroban {
        notes.push(
            "No diagnostic events returned. This is the M0 risk: the node may not run \
             with --enable-soroban-diagnostic-events."
                .into(),
        );
    }

    ProbeReport {
        schema: "sdo.probe.v1",
        rpc_url: rpc_url.to_string(),
        network_passphrase,
        transaction: tx_hash.to_string(),
        status,
        ledger,
        is_soroban,
        soroban_operation: soroban_op,
        envelope_xdr,
        result_xdr,
        result_meta_xdr,
        meta_version,
        soroban_meta_present,
        failure_signal,
        diagnostic_events,
        verdict,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn absent_artifacts_are_reported_absent_not_guessed() {
        let result = json!({ "status": "FAILED" });
        let r = probe_transaction("http://example.invalid", None, "ab", &result);
        assert_eq!(r.envelope_xdr, ArtifactState::Absent);
        assert_eq!(r.verdict, Verdict::Unusable);
        assert!(!r.diagnostic_events.available);
    }

    #[test]
    fn not_found_is_distinguished_from_unusable() {
        let result = json!({ "status": "NOT_FOUND" });
        let r = probe_transaction("http://example.invalid", None, "ab", &result);
        assert_eq!(r.verdict, Verdict::TransactionNotFound);
    }

    #[test]
    fn success_is_not_treated_as_a_failure_sample() {
        let result = json!({ "status": "SUCCESS", "envelopeXdr": "bogus" });
        let r = probe_transaction("http://example.invalid", None, "ab", &result);
        assert_eq!(r.verdict, Verdict::NotAFailedTransaction);
    }

    #[test]
    fn malformed_base64_is_reported_as_decode_failure_not_a_panic() {
        let result = json!({
            "status": "FAILED",
            "envelopeXdr": "!!!! not base64 !!!!",
            "resultXdr": "AAAA",
        });
        let r = probe_transaction("http://example.invalid", None, "ab", &result);
        assert!(matches!(r.envelope_xdr, ArtifactState::DecodeFailed(_)));
        assert_eq!(r.verdict, Verdict::Unusable);
    }

    #[test]
    fn truncated_valid_base64_is_handled_safely() {
        // Valid base64, invalid XDR. Must not panic.
        let result = json!({
            "status": "FAILED",
            "envelopeXdr": "AAAAAgAAAAA=",
            "resultXdr": "AAAAAAAAAGQ=",
            "resultMetaXdr": "AAAAAw==",
        });
        let r = probe_transaction("http://example.invalid", None, "ab", &result);
        assert!(!r.is_soroban);
        assert_ne!(r.verdict, Verdict::SufficientForAnalysis);
    }

    #[test]
    fn diagnostic_events_are_searched_in_both_response_shapes() {
        let top = json!({ "status": "FAILED", "diagnosticEventsXdr": ["AAAA"] });
        let (loc, ev) = locate_diagnostic_events(&top, None);
        assert_eq!(loc, Some(DiagnosticLocation::TopLevel));
        assert_eq!(ev.len(), 1);

        let nested = json!({
            "status": "FAILED",
            "events": { "diagnosticEventsXdr": ["AAAA", "BBBB"] }
        });
        let (loc, ev) = locate_diagnostic_events(&nested, None);
        assert_eq!(loc, Some(DiagnosticLocation::EventsObject));
        assert_eq!(ev.len(), 2);
    }

    #[test]
    fn empty_diagnostic_array_counts_as_absent() {
        let empty = json!({ "status": "FAILED", "diagnosticEventsXdr": [] });
        let (loc, ev) = locate_diagnostic_events(&empty, None);
        assert_eq!(loc, None);
        assert!(ev.is_empty());
    }

    #[test]
    fn undecodable_diagnostic_events_do_not_count_as_sufficient() {
        let result = json!({
            "status": "FAILED",
            "envelopeXdr": "!!!",
            "diagnosticEventsXdr": ["not-xdr"],
        });
        let r = probe_transaction("http://example.invalid", None, "ab", &result);
        assert!(r.diagnostic_events.available);
        assert!(!r.diagnostic_events.decoded);
        assert_eq!(r.diagnostic_events.decoded_count, 0);
        assert_ne!(r.verdict, Verdict::SufficientForAnalysis);
    }
}
