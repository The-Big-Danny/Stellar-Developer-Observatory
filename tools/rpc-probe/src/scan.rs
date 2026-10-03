//! Find real failed Soroban transactions to probe.
//!
//! There is no RPC method that filters for failures, so this walks recent
//! ledgers via `getTransactions` and decodes each envelope to find the ones that
//! failed *and* carry a Soroban operation. Slow, but it uses only documented RPC
//! and produces genuinely real samples rather than invented ones.

use serde_json::Value;
use stellar_xdr::{ReadXdr, TransactionEnvelope};

use crate::probe::soroban_operation_of;
use crate::rpc::{RpcClient, RpcError};

/// A failed Soroban transaction found by scanning.
#[derive(Debug, Clone)]
pub struct FoundTransaction {
    /// Hex transaction hash.
    pub hash: String,
    /// Ledger it was included in.
    pub ledger: u64,
    /// Which Soroban operation it carried.
    pub operation: String,
}

/// Scan forward from `start_ledger`, returning up to `want` failed Soroban
/// transactions.
///
/// `max_pages` bounds the work so this cannot run away against a busy network.
pub fn scan_for_failed_soroban(
    client: &RpcClient,
    start_ledger: u32,
    want: usize,
    max_pages: usize,
    page_size: u32,
    mut on_page: impl FnMut(usize, u64, usize),
) -> Result<Vec<FoundTransaction>, RpcError> {
    let mut found = Vec::new();
    let mut ledger = start_ledger;

    for page in 0..max_pages {
        if found.len() >= want {
            break;
        }

        let result = client.get_transactions(ledger, page_size)?;
        let txs = result
            .get("transactions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        if txs.is_empty() {
            break;
        }

        let mut last_ledger = ledger as u64;
        let mut hits_this_page = 0usize;

        for tx in &txs {
            if let Some(l) = tx.get("ledger").and_then(Value::as_u64) {
                last_ledger = l;
            }

            if tx.get("status").and_then(Value::as_str) != Some("FAILED") {
                continue;
            }

            let Some(envelope_b64) = tx.get("envelopeXdr").and_then(Value::as_str) else {
                continue;
            };
            let Ok(envelope) = TransactionEnvelope::from_xdr_base64(
                envelope_b64,
                soroban_failure_rpc::xdr::limits_for_base64(envelope_b64),
            ) else {
                continue;
            };
            let Some(operation) = soroban_operation_of(&envelope) else {
                continue;
            };
            let Some(hash) = tx.get("txHash").and_then(Value::as_str) else {
                continue;
            };

            found.push(FoundTransaction {
                hash: hash.to_string(),
                ledger: last_ledger,
                operation,
            });
            hits_this_page += 1;

            if found.len() >= want {
                break;
            }
        }

        on_page(page, last_ledger, hits_this_page);

        // Advance past the last ledger seen. `getTransactions` is ledger-based,
        // so stepping to last+1 avoids re-reading the same page forever.
        let next = (last_ledger as u32).saturating_add(1);
        if next <= ledger {
            break;
        }
        ledger = next;
    }

    Ok(found)
}
