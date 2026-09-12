//! Rule: an authorization entry was present but not valid.
//!
//! **Fires on:** an `error` diagnostic event of type `Auth` whose host message
//! is one of two failures observed on mainnet:
//!
//! | Host message | Error | Event data |
//! |---|---|---|
//! | [`SIGNATURE_EXPIRED_MARKER`] | `Error(Auth, InvalidInput)` | `[message, address, current ledger, expiration ledger]` |
//! | [`NONCE_REUSED_MARKER`] | `Error(Auth, ExistingValue)` | `[message, address]` |
//!
//! **Confidence:**
//!
//! | Evidence | Confidence |
//! |---|---|
//! | expired signature, and the envelope's own auth entry for that address carries the expiration ledger the host checked | `Confirmed` |
//! | expired signature without that corroboration | `Likely` |
//! | reused nonce, with an auth entry for the reported address | `Likely` — prior use of a nonce is ledger state the transaction cannot prove |
//! | reused nonce with no matching auth entry | `Possible` |
//! | the authorization error did not end the invocation | `Possible` |
//!
//! **Does not fire on:** any other authorization error. An unrecognised
//! `Error(Auth, …)` returns "no evidence", because without knowing the failure
//! it is impossible to tell a *missing* entry from an *invalid* one. It never
//! turns a generic trap into an authorization failure.
//!
//! **Missing authorization** (`CauseClass::MissingAuthorizationEntry`) has no
//! rule: the corpus contains no example of that failure, so its evidence shape
//! is unknown.
//!
//! **Fixtures:** `soroban-auth-signature-expired` and `soroban-auth-nonce-reused`
//! (real mainnet).

use stellar_xdr::{ScAddress, ScError, ScVal};

use crate::diagnosis::{CandidateCause, Confidence, Evidence, EvidenceSource};
use crate::model::{error_label, AuthCredentials, AuthEntry, DiagEvent, DiagnosticAvailability};
use crate::rule::{FailureContext, Rule, RuleOutcome};
use crate::taxonomy::{CauseClass, FailureStage};

/// Host message for an authorization signature past its expiration ledger.
pub const SIGNATURE_EXPIRED_MARKER: &str = "signature has expired";

/// Host message for an authorization nonce that has already been consumed.
pub const NONCE_REUSED_MARKER: &str = "nonce already exists for address";

/// See the [module documentation](self).
pub struct InvalidAuthorizationEntry;

#[derive(Clone, Copy)]
enum Failure {
    SignatureExpired,
    NonceReused,
}

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

/// `(current ledger, expiration ledger)` from an expired-signature event.
fn event_ledgers(data: &ScVal) -> Option<(u32, u32)> {
    match (event_items(data).get(2), event_items(data).get(3)) {
        (Some(ScVal::U32(current)), Some(ScVal::U32(expiry))) => Some((*current, *expiry)),
        _ => None,
    }
}

fn entry_for<'a>(auth: &'a [AuthEntry], address: &ScAddress) -> Option<&'a AuthEntry> {
    auth.iter().find(
        |e| matches!(&e.credentials, AuthCredentials::Address { address: a, .. } if a == address),
    )
}

impl Rule for InvalidAuthorizationEntry {
    fn id(&self) -> &'static str {
        "invalid_authorization_entry"
    }

    fn description(&self) -> &'static str {
        "An authorization entry was present but invalid: expired signature or reused nonce"
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
                "the transaction failed at stage `{stage}`; a failed authorization ends contract \
                 execution with a trap"
            ));
        }
        if m.diagnostics.availability == DiagnosticAvailability::NotEmitted {
            return RuleOutcome::no_evidence(
                "diagnostic events were not available; without them a failed authorization is \
                 indistinguishable from any other trap",
            );
        }

        let terminal = m.diagnostics.terminal_error.as_ref();
        let auth_errors: Vec<(&DiagEvent, &ScError, Option<&str>)> = m
            .diagnostics
            .errors()
            .filter(|(_, e, _)| matches!(e, ScError::Auth(_)))
            .collect();

        let Some(&(first, first_error, first_message)) = auth_errors.first() else {
            return match terminal {
                Some(t) => RuleOutcome::not_applicable(format!(
                    "no authorization error was raised; the invocation ended with {}",
                    error_label(&t.error)
                )),
                None => RuleOutcome::no_evidence(
                    "no diagnostic event reports an authorization failure, and no terminal error \
                     explains the trap",
                ),
            };
        };

        let recognised = auth_errors.iter().find_map(|&(event, error, message)| {
            let message = message?;
            let failure = if message.starts_with(SIGNATURE_EXPIRED_MARKER) {
                Failure::SignatureExpired
            } else if message.starts_with(NONCE_REUSED_MARKER) {
                Failure::NonceReused
            } else {
                return None;
            };
            Some((event, error, message, failure))
        });
        let Some((event, error, message, failure)) = recognised else {
            let _ = first;
            return RuleOutcome::no_evidence(format!(
                "authorization failed with {}{}, a failure this rule does not recognise, so a \
                 missing entry cannot be told apart from an invalid one",
                error_label(first_error),
                first_message
                    .map(|m| format!(" (\"{m}\")"))
                    .unwrap_or_default()
            ));
        };

        let ended_by_it = terminal.filter(|t| t.error == *error);
        let address = event_address(&event.data);
        let who = address.map_or("an unidentified address".to_string(), ToString::to_string);
        let entry = address.and_then(|a| entry_for(&soroban.auth, a));

        let mut evidence = vec![Evidence::new(
            EvidenceSource::DiagnosticEvent { index: event.index },
            format!(
                "the host raised {} for {who}: \"{message}\"",
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
        match entry {
            Some(e) => {
                if let AuthCredentials::Address {
                    nonce,
                    signature_expiration_ledger,
                    ..
                } = &e.credentials
                {
                    evidence.push(Evidence::new(
                        EvidenceSource::AuthorizationEntry { index: e.index },
                        format!(
                            "authorization entry {} carries credentials for that address (nonce \
                             {nonce}, signature valid until ledger {signature_expiration_ledger})",
                            e.index
                        ),
                    ));
                }
            }
            None => evidence.push(Evidence::new(
                EvidenceSource::DiagnosticEvent { index: event.index },
                "no authorization entry in the transaction carries credentials for that address",
            )),
        }

        let (confidence, summary, remediation) = match failure {
            Failure::SignatureExpired => {
                let ledgers = event_ledgers(&event.data);
                let corroborated = match (entry, ledgers) {
                    (Some(e), Some((current, expiry))) => {
                        current > expiry
                            && matches!(
                                &e.credentials,
                                AuthCredentials::Address { signature_expiration_ledger, .. }
                                    if *signature_expiration_ledger == expiry
                            )
                    }
                    _ => false,
                };
                if let Some((current, expiry)) = ledgers {
                    evidence.push(Evidence::new(
                        EvidenceSource::DiagnosticEvent { index: event.index },
                        format!(
                            "the host checked it at ledger {current}; the signature was valid \
                             until ledger {expiry}"
                        ),
                    ));
                }
                if corroborated {
                    if let Some(e) = entry {
                        evidence.push(Evidence::new(
                            EvidenceSource::AuthorizationEntry { index: e.index },
                            "the entry's own signature expiration ledger matches the one the host \
                             checked",
                        ));
                    }
                }
                let confidence = match (ended_by_it.is_some(), corroborated) {
                    (false, _) => Confidence::Possible,
                    (true, true) => Confidence::Confirmed,
                    (true, false) => Confidence::Likely,
                };
                let when = ledgers.map_or(String::new(), |(current, expiry)| {
                    format!(" (valid until ledger {expiry}, checked at ledger {current})")
                });
                (
                    confidence,
                    format!(
                        "The authorization entry for {who} carried a signature that had \
                         expired{when}."
                    ),
                    format!(
                        "Sign a new authorization for {who} with a signature expiration ledger \
                         that leaves enough margin for the time between signing and inclusion, \
                         then resubmit."
                    ),
                )
            }
            Failure::NonceReused => {
                let confidence = match (ended_by_it.is_some(), entry.is_some()) {
                    (true, true) => Confidence::Likely,
                    _ => Confidence::Possible,
                };
                (
                    confidence,
                    format!(
                        "The authorization entry for {who} used a nonce the host reports as \
                         already consumed."
                    ),
                    "A signed authorization can be used only once. Build and sign a new \
                     authorization entry, for example by simulating again, instead of \
                     resubmitting one that has already been used."
                        .to_string(),
                )
            }
        };

        RuleOutcome::Match(CandidateCause {
            class: CauseClass::InvalidAuthorizationEntry,
            confidence,
            summary,
            evidence,
            remediation: Some(remediation),
            rule_id: self.id().into(),
        })
    }
}

#[cfg(test)]
mod tests {
    //! Synthetic. The real mainnet cases are tested in
    //! `crates/sdo/tests/classification.rs`.

    use super::*;
    use crate::contract::ORIGIN_MARKER;
    use crate::testutil::{
        address_auth, analyze_ctx, auth_error, call, cid, err, host_fn_failed, minimal_input,
        soroban_failure, soroban_failure_with_auth,
    };
    use stellar_xdr::{InvokeHostFunctionResult as R, ScErrorCode};

    const EXPIRED: ScError = ScError::Auth(ScErrorCode::InvalidInput);
    const REUSED: ScError = ScError::Auth(ScErrorCode::ExistingValue);

    fn wallet() -> ScAddress {
        ScAddress::Contract(cid(7))
    }

    fn evaluate(
        expiry_in_entry: Option<u32>,
        events: Vec<stellar_xdr::DiagnosticEvent>,
    ) -> RuleOutcome {
        let auth = expiry_in_entry
            .map(|exp| vec![address_auth(wallet(), 11, exp)])
            .unwrap_or_default();
        let mut all = vec![call(None, cid(5), "f")];
        all.extend(events);
        let input = soroban_failure_with_auth(R::Trapped, Vec::new(), auth, Some(all));
        analyze_ctx(&input, |ctx| InvalidAuthorizationEntry.evaluate(ctx))
    }

    fn expired_event(current: u32, expiry: u32) -> stellar_xdr::DiagnosticEvent {
        auth_error(
            cid(5),
            ScErrorCode::InvalidInput,
            SIGNATURE_EXPIRED_MARKER,
            vec![
                ScVal::Address(wallet()),
                ScVal::U32(current),
                ScVal::U32(expiry),
            ],
        )
    }

    #[test]
    fn an_expired_signature_corroborated_by_the_envelope_is_confirmed() {
        let outcome = evaluate(
            Some(100),
            vec![expired_event(102, 100), host_fn_failed(EXPIRED)],
        );
        let c = outcome.candidate().expect("should match");
        assert_eq!(c.class, CauseClass::InvalidAuthorizationEntry);
        assert_eq!(c.confidence, Confidence::Confirmed);
        assert!(c
            .summary
            .contains("valid until ledger 100, checked at ledger 102"));
        assert!(c
            .evidence
            .iter()
            .any(|e| e.source == EvidenceSource::AuthorizationEntry { index: 0 }));
    }

    #[test]
    fn an_expired_signature_the_envelope_does_not_corroborate_is_only_likely() {
        let outcome = evaluate(
            Some(99),
            vec![expired_event(102, 100), host_fn_failed(EXPIRED)],
        );
        assert_eq!(outcome.candidate().unwrap().confidence, Confidence::Likely);
    }

    #[test]
    fn a_reused_nonce_is_likely_never_confirmed() {
        let event = auth_error(
            cid(5),
            ScErrorCode::ExistingValue,
            NONCE_REUSED_MARKER,
            vec![ScVal::Address(wallet())],
        );
        let outcome = evaluate(Some(100), vec![event, host_fn_failed(REUSED)]);
        let c = outcome.candidate().unwrap();
        assert_eq!(c.confidence, Confidence::Likely);
        assert!(c.summary.contains("already consumed"));
    }

    #[test]
    fn a_reused_nonce_with_no_matching_entry_is_only_possible() {
        let event = auth_error(
            cid(5),
            ScErrorCode::ExistingValue,
            NONCE_REUSED_MARKER,
            vec![ScVal::Address(wallet())],
        );
        let outcome = evaluate(None, vec![event, host_fn_failed(REUSED)]);
        assert_eq!(
            outcome.candidate().unwrap().confidence,
            Confidence::Possible
        );
    }

    #[test]
    fn an_unrecognised_auth_failure_is_no_evidence_not_a_guess_between_missing_and_invalid() {
        let event = auth_error(
            cid(5),
            ScErrorCode::InvalidAction,
            "something else",
            Vec::new(),
        );
        match evaluate(
            Some(100),
            vec![
                event,
                host_fn_failed(ScError::Auth(ScErrorCode::InvalidAction)),
            ],
        ) {
            RuleOutcome::NoEvidence { reason } => assert!(reason.contains("cannot be told apart")),
            other => panic!("expected NoEvidence, got {other:?}"),
        }
    }

    #[test]
    fn an_auth_failure_that_did_not_end_the_invocation_is_only_possible() {
        let outcome = evaluate(
            Some(100),
            vec![
                expired_event(102, 100),
                err(cid(5), ScError::Contract(2), ORIGIN_MARKER),
                host_fn_failed(ScError::Contract(2)),
            ],
        );
        assert_eq!(
            outcome.candidate().unwrap().confidence,
            Confidence::Possible
        );
    }

    #[test]
    fn a_non_auth_terminal_error_is_not_applicable() {
        let outcome = evaluate(
            Some(100),
            vec![
                err(cid(5), ScError::Contract(2), ORIGIN_MARKER),
                host_fn_failed(ScError::Contract(2)),
            ],
        );
        assert!(matches!(outcome, RuleOutcome::NotApplicable { .. }));
    }

    #[test]
    fn a_generic_trap_is_not_turned_into_an_auth_failure() {
        assert!(matches!(
            evaluate(Some(100), Vec::new()),
            RuleOutcome::NoEvidence { .. }
        ));
        let input = soroban_failure(R::Trapped, Vec::new(), None);
        assert!(matches!(
            analyze_ctx(&input, |ctx| InvalidAuthorizationEntry.evaluate(ctx)),
            RuleOutcome::NoEvidence { .. }
        ));
    }

    #[test]
    fn a_classic_transaction_is_not_applicable() {
        assert!(matches!(
            analyze_ctx(&minimal_input(), |ctx| InvalidAuthorizationEntry
                .evaluate(ctx)),
            RuleOutcome::NotApplicable { .. }
        ));
    }
}
