//! Rule: the contract required authorization from an address the transaction
//! carries no authorization entry for.
//!
//! **Fires on:** an `error` diagnostic event of type `Auth` with the host
//! message [`UNAUTHORIZED_MARKER`] (`"Unauthorized function call for address"`),
//! raised as `Error(Auth, InvalidAction)`. Its data is
//! `[message, address]`, which names the address that did not authorize.
//!
//! **Confidence:**
//!
//! | Evidence | Confidence |
//! |---|---|
//! | the error ended the invocation, and no authorization entry in the envelope carries credentials for that address | `Confirmed` |
//! | the error did not end the invocation, or the address cannot be read from the event | `Likely` |
//!
//! `Confirmed` rests on two checks: the host's structured error, and the
//! envelope's own authorization list, which holds no credentials for the
//! address. The absence is verified from the transaction, not inferred from the
//! message.
//!
//! **Does not fire on:**
//!
//! - the transaction's own source account. A source account authorizes its own
//!   invocations implicitly, so a rejection naming it is not a missing entry.
//!   Returns `NotApplicable`.
//! - an address that *does* have an authorization entry. The entry is present
//!   but not accepted, which is [`super::InvalidAuthorizationEntry`]'s
//!   territory. Returns `NoEvidence`, not a guess.
//! - other authorization errors, including expired signatures and reused nonces.
//! - a transaction with no diagnostic events, since the address is only visible
//!   in them.
//!
//! **Deliberately does not claim:** that the caller forgot to sign. The host
//! message says the address did not authorize the call. Whether the entry was
//! never built, or was built and then removed, the transaction cannot show.
//!
//! The message is a host diagnostic string, not a stable API. If a host release
//! rewords it, this rule degrades to "no evidence", never to a false diagnosis.
//!
//! **Fixtures:** `soroban-auth-missing-testnet` (real Soroban testnet, deliberately
//! caused: the authorization entry for a `require_auth()` was removed from a
//! signed envelope before submission). Source in
//! `fixtures/contract-sources/contract-auth/`.

use stellar_xdr::{ScAddress, ScError, ScErrorCode, ScVal};

use crate::diagnosis::{CandidateCause, Confidence, Evidence, EvidenceSource};
use crate::model::{error_label, AuthCredentials, AuthEntry, DiagnosticAvailability};
use crate::rule::{FailureContext, Rule, RuleOutcome};
use crate::taxonomy::{CauseClass, FailureStage};

/// The host message for an authorization the transaction does not provide, as
/// observed on testnet.
pub const UNAUTHORIZED_MARKER: &str = "Unauthorized function call for address";

/// See the [module documentation](self).
pub struct MissingAuthorizationEntry;

fn event_items(data: &ScVal) -> &[ScVal] {
    match data {
        ScVal::Vec(Some(items)) => items.as_slice(),
        _ => &[],
    }
}

fn event_address(data: &ScVal) -> Option<&ScAddress> {
    match event_items(data).get(1) {
        Some(ScVal::Address(a)) => Some(a),
        _ => None,
    }
}

/// The authorization entry that carries credentials for `address`, if any.
fn entry_for<'a>(auth: &'a [AuthEntry], address: &ScAddress) -> Option<&'a AuthEntry> {
    auth.iter().find(
        |e| matches!(&e.credentials, AuthCredentials::Address { address: a, .. } if a == address),
    )
}

impl Rule for MissingAuthorizationEntry {
    fn id(&self) -> &'static str {
        "missing_authorization_entry"
    }

    fn description(&self) -> &'static str {
        "The contract required authorization from an address the transaction did not provide"
    }

    fn evaluate(&self, ctx: &FailureContext<'_>) -> RuleOutcome {
        let m = ctx.model;
        let Some(stage) = m.stage() else {
            return RuleOutcome::not_applicable("the transaction succeeded");
        };
        let Some(soroban) = &m.soroban else {
            return RuleOutcome::not_applicable(
                "not a Soroban transaction, so it has no authorization entries",
            );
        };
        if stage != FailureStage::ContractExecution {
            return RuleOutcome::not_applicable(format!(
                "the transaction failed at stage `{stage}`; a missing authorization ends \
                 contract execution with a trap"
            ));
        }
        if m.diagnostics.availability == DiagnosticAvailability::NotEmitted {
            return RuleOutcome::no_evidence(
                "diagnostic events were not available, and the unauthorized address is only \
                 visible in them",
            );
        }

        let Some((event, error, _)) = m.diagnostics.errors().find(|(_, e, msg)| {
            matches!(e, ScError::Auth(ScErrorCode::InvalidAction))
                && msg.is_some_and(|s| s.starts_with(UNAUTHORIZED_MARKER))
        }) else {
            return RuleOutcome::no_evidence(
                "no diagnostic event reports an unauthorized call for an address",
            );
        };

        let terminal = m.diagnostics.terminal_error.as_ref();
        let ended_by_it = terminal.filter(|t| t.error == *error);
        let address = event_address(&event.data);
        let who = address.map_or("an unidentified address".to_string(), ToString::to_string);

        if address.is_some_and(|a| a.to_string() == m.source_account) {
            return RuleOutcome::not_applicable(format!(
                "the unauthorized address {who} is the transaction's own source account, which \
                 authorizes its invocations implicitly"
            ));
        }
        if let Some(a) = address {
            if let Some(entry) = entry_for(&soroban.auth, a) {
                return RuleOutcome::no_evidence(format!(
                    "authorization entry {} carries credentials for {who}, so the entry is \
                     present but not accepted; that is an invalid entry, not a missing one",
                    entry.index
                ));
            }
        }

        let mut evidence = vec![Evidence::new(
            EvidenceSource::DiagnosticEvent { index: event.index },
            format!(
                "the host raised {} for {who}: \"{UNAUTHORIZED_MARKER}\"",
                error_label(error)
            ),
        )];
        match ended_by_it {
            Some(t) => evidence.push(Evidence::new(
                EvidenceSource::DiagnosticEvent {
                    index: t.event_index,
                },
                format!(
                    "the invocation ended with the same error, {}",
                    error_label(error)
                ),
            )),
            None => evidence.push(Evidence::new(
                EvidenceSource::DiagnosticEvent { index: event.index },
                "the invocation did not end with this error",
            )),
        }

        if address.is_some() {
            evidence.push(Evidence::new(
                EvidenceSource::DiagnosticEvent { index: event.index },
                format!(
                    "no authorization entry in the transaction carries credentials for {who}; \
                     the transaction has {} authorization entr{}",
                    soroban.auth.len(),
                    if soroban.auth.len() == 1 { "y" } else { "ies" }
                ),
            ));
        }

        let confidence = match (ended_by_it.is_some(), address.is_some()) {
            (true, true) => Confidence::Confirmed,
            _ => Confidence::Likely,
        };

        RuleOutcome::Match(CandidateCause {
            class: CauseClass::MissingAuthorizationEntry,
            confidence,
            summary: format!(
                "The contract required authorization from {who}, but the transaction carries no \
                 authorization entry for that address."
            ),
            evidence,
            remediation: Some(format!(
                "Simulate the transaction so the authorization entry for {who} is generated, \
                 have {who} sign that entry, then submit. If the transaction is built by hand, \
                 include the entry before the envelope is signed."
            )),
            rule_id: self.id().into(),
        })
    }
}
