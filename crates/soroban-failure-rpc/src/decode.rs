//! Turn a `getTransaction` JSON result into a typed [`AnalysisInput`].
//!
//! **This module performs no I/O.** It takes JSON that someone else fetched —
//! from the network, or from a committed fixture — and decodes it. Keeping the
//! decode step pure is what lets the whole test suite run offline.

use serde_json::Value;
use soroban_failure_analysis::AnalysisInput;
use stellar_xdr::{
    DiagnosticEvent, ReadXdr, TransactionEnvelope, TransactionMeta, TransactionResult,
};

use crate::error::DecodeError;
use crate::xdr::limits_for_base64;

/// Where the diagnostic events for a transaction were found.
///
/// RPC has carried these in three different places across protocol versions, so
/// this records which one actually supplied them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSource {
    /// Top-level `diagnosticEventsXdr` in the RPC result.
    TopLevel,
    /// Nested `events.diagnosticEventsXdr`.
    EventsObject,
    /// Embedded in `resultMetaXdr` (V3 `sorobanMeta`, or V4 top level).
    ResultMeta,
}

fn base64_array(v: Option<&Value>) -> Vec<&str> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// Locate diagnostic events, checking every place RPC is known to put them.
fn locate_diagnostics(
    result: &Value,
    meta: Option<&TransactionMeta>,
) -> Option<(DiagnosticSource, Vec<DiagnosticEvent>)> {
    // 1. Top-level field.
    let top = base64_array(result.get("diagnosticEventsXdr"));
    if !top.is_empty() {
        return Some((DiagnosticSource::TopLevel, decode_events(&top)));
    }

    // 2. Nested under `events`.
    let nested = base64_array(
        result
            .get("events")
            .and_then(|e| e.get("diagnosticEventsXdr")),
    );
    if !nested.is_empty() {
        return Some((DiagnosticSource::EventsObject, decode_events(&nested)));
    }

    // 3. Embedded in the metadata. The location moved at protocol 23: V3 keeps
    //    them inside `sorobanMeta`, V4 lifted them to the meta's top level.
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
        return Some((DiagnosticSource::ResultMeta, embedded.to_vec()));
    }

    None
}

/// Decode what we can, discarding individually malformed events.
///
/// A single unreadable event must not discard the rest: partial evidence is
/// still evidence, and the count difference is visible to callers.
fn decode_events(raw: &[&str]) -> Vec<DiagnosticEvent> {
    raw.iter()
        .filter_map(|b| DiagnosticEvent::from_xdr_base64(b, limits_for_base64(b)).ok())
        .collect()
}

/// The transaction status reported by RPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionStatus {
    /// The transaction succeeded.
    Success,
    /// The transaction failed.
    Failed,
    /// The endpoint does not have this transaction.
    NotFound,
}

/// A decoded `getTransaction` response.
#[derive(Debug, Clone)]
pub struct DecodedTransaction {
    /// The status RPC reported.
    pub status: TransactionStatus,
    /// Typed input ready for the analysis engine.
    pub input: AnalysisInput,
    /// Where diagnostic events came from, if any were present.
    pub diagnostic_source: Option<DiagnosticSource>,
}

/// Decode an RPC `getTransaction` result object.
///
/// `tx_hash` is threaded through for reporting only.
pub fn decode_get_transaction(
    tx_hash: &str,
    result: &Value,
) -> Result<DecodedTransaction, DecodeError> {
    let status = match result.get("status").and_then(Value::as_str) {
        Some("SUCCESS") => TransactionStatus::Success,
        Some("FAILED") => TransactionStatus::Failed,
        Some("NOT_FOUND") => return Err(DecodeError::TransactionNotFound(tx_hash.to_string())),
        Some(other) => return Err(DecodeError::UnknownStatus(other.to_string())),
        None => return Err(DecodeError::MissingField("status")),
    };

    let envelope_b64 = result
        .get("envelopeXdr")
        .and_then(Value::as_str)
        .ok_or(DecodeError::MissingField("envelopeXdr"))?;
    let result_b64 = result
        .get("resultXdr")
        .and_then(Value::as_str)
        .ok_or(DecodeError::MissingField("resultXdr"))?;

    let envelope =
        TransactionEnvelope::from_xdr_base64(envelope_b64, limits_for_base64(envelope_b64))
            .map_err(|e| DecodeError::Xdr("envelopeXdr", e.to_string()))?;
    let tx_result = TransactionResult::from_xdr_base64(result_b64, limits_for_base64(result_b64))
        .map_err(|e| DecodeError::Xdr("resultXdr", e.to_string()))?;

    // Metadata is optional: it is genuinely absent for some transactions, and an
    // analyzer should degrade rather than refuse.
    let meta = match result.get("resultMetaXdr").and_then(Value::as_str) {
        Some(b64) => Some(
            TransactionMeta::from_xdr_base64(b64, limits_for_base64(b64))
                .map_err(|e| DecodeError::Xdr("resultMetaXdr", e.to_string()))?,
        ),
        None => None,
    };

    let located = locate_diagnostics(result, meta.as_ref());
    let diagnostic_source = located.as_ref().map(|(s, _)| *s);

    let mut builder = AnalysisInput::builder(envelope, tx_result).transaction_hash(tx_hash);
    if let Some(m) = meta {
        builder = builder.meta(m);
    }
    if let Some((_, events)) = located {
        builder = builder.diagnostic_events(events);
    }

    Ok(DecodedTransaction {
        status,
        input: builder.build(),
        diagnostic_source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn not_found_is_its_own_error_not_a_decode_failure() {
        let err = decode_get_transaction("ab", &json!({ "status": "NOT_FOUND" })).unwrap_err();
        assert!(matches!(err, DecodeError::TransactionNotFound(_)));
    }

    #[test]
    fn missing_status_is_reported_precisely() {
        let err = decode_get_transaction("ab", &json!({})).unwrap_err();
        assert!(matches!(err, DecodeError::MissingField("status")));
    }

    #[test]
    fn malformed_xdr_is_an_error_not_a_panic() {
        let err = decode_get_transaction(
            "ab",
            &json!({ "status": "FAILED", "envelopeXdr": "!!!", "resultXdr": "!!!" }),
        )
        .unwrap_err();
        assert!(matches!(err, DecodeError::Xdr("envelopeXdr", _)));
    }

    #[test]
    fn unknown_status_does_not_silently_become_failed() {
        let err = decode_get_transaction("ab", &json!({ "status": "PENDING" })).unwrap_err();
        assert!(matches!(err, DecodeError::UnknownStatus(s) if s == "PENDING"));
    }

    #[test]
    fn individually_malformed_events_do_not_discard_the_readable_ones() {
        let decoded = decode_events(&["!!! not xdr !!!"]);
        assert!(decoded.is_empty());
    }
}
