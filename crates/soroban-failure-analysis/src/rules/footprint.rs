//! Rule: the invocation accessed contract data its footprint did not declare.
//!
//! **Fires on:** an `error` diagnostic event of type `Storage` whose host
//! message contains [`FOOTPRINT_MARKER`]. On mainnet this event carries the
//! accessed key as data `[message, contract address, key]`; the rule extracts
//! it and **checks it against the declared footprint** rather than trusting the
//! message alone.
//!
//! **Confidence:**
//!
//! | Evidence | Confidence |
//! |---|---|
//! | key extracted, absent from the footprint, and the same error ended the invocation | `Confirmed` |
//! | message present, key not extractable, same error ended the invocation | `Likely` |
//! | the violation did not end the invocation | `Possible` |
//! | key extracted but the footprint *does* declare it (conflicting evidence) | `Possible` |
//!
//! **Does not fire on:** other storage errors without the marker (a
//! `Storage/ExceededLimit` error alone also covers other limits); the
//! `EntryArchived` result code, which is a different failure
//! ([`super::ArchivedEntry`]); anything without diagnostic events.
//!
//! Durability is not in the event, so a key counts as declared if the footprint
//! holds it under either durability.
//!
//! The message is a host diagnostic string, not a stable API. If a host release
//! rewords it, this rule stops firing — it degrades to "no evidence", never to a
//! false diagnosis.
//!
//! **Fixtures:** `soroban-trapped-feebump-24ev` (real mainnet). The mainnet
//! survey found this shape in 175 of 219 failed Soroban transactions sampled.

use stellar_xdr::{LedgerKey, ScAddress, ScError, ScVal};

use crate::diagnosis::{CandidateCause, Confidence, Evidence, EvidenceSource};
use crate::model::{error_label, value_label, DiagnosticAvailability};
use crate::rule::{FailureContext, Rule, RuleOutcome};
use crate::taxonomy::{CauseClass, FailureStage};

/// The fragment of the host's message that identifies a footprint violation,
/// as observed on mainnet: "trying to access contract data key outside of the
/// footprint".
pub const FOOTPRINT_MARKER: &str = "outside of the footprint";

/// See the [module documentation](self).
pub struct FootprintEntryMissing;

/// The `(contract, key)` an access-violation event reports, if it has the
/// `[message, address, key]` shape.
fn accessed_key(data: &ScVal) -> Option<(ScAddress, ScVal)> {
    let ScVal::Vec(Some(items)) = data else {
        return None;
    };
    match (items.get(1), items.get(2)) {
        (Some(ScVal::Address(a)), Some(key)) => Some((a.clone(), key.clone())),
        _ => None,
    }
}

impl Rule for FootprintEntryMissing {
    fn id(&self) -> &'static str {
        "footprint_entry_missing"
    }

    fn description(&self) -> &'static str {
        "The invocation accessed a contract data key its declared footprint did not include"
    }

    fn evaluate(&self, ctx: &FailureContext<'_>) -> RuleOutcome {
        let m = ctx.model;
        let Some(stage) = m.stage() else {
            return RuleOutcome::not_applicable("the transaction succeeded");
        };
        let Some(soroban) = &m.soroban else {
            return RuleOutcome::not_applicable(
                "not a Soroban transaction, so it has no footprint",
            );
        };
        if stage != FailureStage::ContractExecution {
            return RuleOutcome::not_applicable(format!(
                "the transaction failed at stage `{stage}`; an access outside the footprint ends \
                 contract execution with a trap"
            ));
        }
        if m.diagnostics.availability == DiagnosticAvailability::NotEmitted {
            return RuleOutcome::no_evidence(
                "diagnostic events were not available; without them an access outside the \
                 footprint is indistinguishable from any other trap",
            );
        }

        let ended = m
            .diagnostics
            .terminal_error
            .as_ref()
            .map_or("no terminal error event".to_string(), |t| {
                error_label(&t.error)
            });

        let Some((event, error, message)) = m.diagnostics.errors().find(|(_, e, msg)| {
            matches!(e, ScError::Storage(_)) && msg.is_some_and(|s| s.contains(FOOTPRINT_MARKER))
        }) else {
            return RuleOutcome::no_evidence(format!(
                "no diagnostic event reports an access outside the footprint (the invocation \
                 ended with {ended})"
            ));
        };

        let terminal = m
            .diagnostics
            .terminal_error
            .as_ref()
            .filter(|t| t.error == *error);
        let who = event
            .contract
            .as_ref()
            .map_or("the host".to_string(), |c| format!("contract {c}"));

        let mut evidence = vec![Evidence::new(
            EvidenceSource::DiagnosticEvent { index: event.index },
            format!(
                "{who} hit {}: \"{}\"",
                error_label(error),
                message.unwrap_or_default()
            ),
        )];
        match terminal {
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
                format!("the invocation ended with {ended}, not with this error"),
            )),
        }

        let (confidence, summary, remediation) = match accessed_key(&event.data) {
            Some((address, key)) => {
                let label = value_label(&key);
                // Every footprint entry declaring this (contract, key), under
                // any durability, with its real position in its own list.
                let declared: Vec<(bool, u32, &'static str)> = [
                    (false, &soroban.footprint.read_only),
                    (true, &soroban.footprint.read_write),
                ]
                .into_iter()
                .flat_map(|(rw, list)| list.iter().enumerate().map(move |(i, k)| (rw, i, k)))
                .filter_map(|(rw, i, k)| match k {
                    LedgerKey::ContractData(d) if d.contract == address && d.key == key => Some((
                        rw,
                        u32::try_from(i).unwrap_or(u32::MAX),
                        d.durability.name(),
                    )),
                    _ => None,
                })
                .collect();

                if declared.is_empty() {
                    let data_keys = soroban
                        .footprint
                        .read_only
                        .iter()
                        .chain(&soroban.footprint.read_write)
                        .filter(|k| matches!(k, LedgerKey::ContractData(_)))
                        .count();
                    evidence.push(Evidence::new(
                        EvidenceSource::Footprint,
                        format!(
                            "contract data key {label} of {address} is not among the {data_keys} \
                             contract-data keys the footprint declares ({} read-only and {} \
                             read-write entries in total)",
                            soroban.footprint.read_only.len(),
                            soroban.footprint.read_write.len()
                        ),
                    ));
                    (
                        if terminal.is_some() {
                            Confidence::Confirmed
                        } else {
                            Confidence::Possible
                        },
                        format!(
                            "Contract {address} accessed contract data key {label}, which the \
                             transaction's declared footprint does not include."
                        ),
                        format!(
                            "Re-simulate the transaction immediately before submitting so its \
                             footprint includes contract data key {label} of {address}. If that \
                             key is derived from state that changes between simulation and \
                             inclusion (a counter, sequence, block or ledger number), a simulated \
                             footprint can be stale by the time the transaction executes."
                        ),
                    )
                } else {
                    for (rw, index, durability) in &declared {
                        evidence.push(Evidence::new(
                            EvidenceSource::FootprintEntry {
                                read_write: *rw,
                                index: *index,
                            },
                            format!(
                                "yet the footprint declares that key ({durability}, {} entry \
                                 {index}), so the evidence conflicts",
                                if *rw { "read-write" } else { "read-only" }
                            ),
                        ));
                    }
                    (
                        Confidence::Possible,
                        format!(
                            "The host reported an access outside the footprint for contract data \
                             key {label} of {address}, but the footprint declares that key; the \
                             evidence is inconsistent."
                        ),
                        "Compare the durability and exact encoding of the declared key with what \
                         the contract accesses; a freshly simulated footprint contains the keys \
                         the host expects."
                            .to_string(),
                    )
                }
            }
            None => {
                evidence.push(Evidence::new(
                    EvidenceSource::DiagnosticEvent { index: event.index },
                    "the event does not carry the accessed key in the expected [message, \
                     address, key] shape, so it could not be checked against the footprint",
                ));
                (
                    if terminal.is_some() {
                        Confidence::Likely
                    } else {
                        Confidence::Possible
                    },
                    "The host reported an access outside the transaction's declared footprint; \
                     the accessed key could not be identified."
                        .to_string(),
                    "Re-simulate the transaction immediately before submitting so its footprint \
                     covers every entry the invocation touches."
                        .to_string(),
                )
            }
        };

        RuleOutcome::Match(CandidateCause {
            class: CauseClass::FootprintEntryMissing,
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
    //! Synthetic. The real mainnet case is tested in
    //! `crates/sdo/tests/classification.rs`.

    use super::*;
    use crate::contract::ORIGIN_MARKER;
    use crate::testutil::{
        analyze_ctx, call, cid, data_key, err, footprint_violation, host_fn_failed,
        soroban_failure, sym,
    };
    use stellar_xdr::{InvokeHostFunctionResult as R, ScErrorCode, ScVec};

    fn block(n: u32) -> ScVal {
        ScVal::Vec(Some(ScVec(
            vec![sym("Block"), ScVal::U32(n)].try_into().unwrap(),
        )))
    }

    const EXCEEDED: ScError = ScError::Storage(ScErrorCode::ExceededLimit);

    fn evaluate(
        footprint: Vec<LedgerKey>,
        events: Vec<stellar_xdr::DiagnosticEvent>,
    ) -> RuleOutcome {
        let mut all = vec![call(None, cid(5), "f")];
        all.extend(events);
        let input = soroban_failure(R::Trapped, footprint, Some(all));
        analyze_ctx(&input, |ctx| FootprintEntryMissing.evaluate(ctx))
    }

    #[test]
    fn a_key_absent_from_the_footprint_that_ended_the_invocation_is_confirmed() {
        let outcome = evaluate(
            vec![data_key(cid(5), block(1))],
            vec![
                footprint_violation(cid(5), block(2)),
                host_fn_failed(EXCEEDED),
            ],
        );
        let c = outcome.candidate().expect("should match");
        assert_eq!(c.class, CauseClass::FootprintEntryMissing);
        assert_eq!(c.confidence, Confidence::Confirmed);
        assert!(c.summary.contains("vec[Block, 2]"), "{}", c.summary);
        assert!(c
            .evidence
            .iter()
            .any(|e| e.source == EvidenceSource::Footprint));
    }

    #[test]
    fn a_key_the_footprint_does_declare_is_conflicting_evidence_not_confirmed() {
        let outcome = evaluate(
            vec![data_key(cid(5), block(2))],
            vec![
                footprint_violation(cid(5), block(2)),
                host_fn_failed(EXCEEDED),
            ],
        );
        let c = outcome.candidate().unwrap();
        assert_eq!(c.confidence, Confidence::Possible);
        assert!(c.evidence.iter().any(|e| matches!(
            e.source,
            EvidenceSource::FootprintEntry {
                read_write: true,
                index: 0
            }
        )));
    }

    #[test]
    fn an_unextractable_key_is_likely_never_confirmed() {
        let outcome = evaluate(
            Vec::new(),
            vec![
                err(
                    cid(5),
                    EXCEEDED,
                    "trying to access contract data key outside of the footprint",
                ),
                host_fn_failed(EXCEEDED),
            ],
        );
        assert_eq!(outcome.candidate().unwrap().confidence, Confidence::Likely);
    }

    #[test]
    fn a_violation_that_did_not_end_the_invocation_is_only_possible() {
        let outcome = evaluate(
            Vec::new(),
            vec![
                footprint_violation(cid(5), block(2)),
                err(cid(5), ScError::Contract(4), ORIGIN_MARKER),
                host_fn_failed(ScError::Contract(4)),
            ],
        );
        assert_eq!(
            outcome.candidate().unwrap().confidence,
            Confidence::Possible
        );
    }

    #[test]
    fn storage_exceeded_limit_without_the_marker_is_not_a_footprint_failure() {
        let outcome = evaluate(
            Vec::new(),
            vec![
                err(cid(5), EXCEEDED, "some other storage limit"),
                host_fn_failed(EXCEEDED),
            ],
        );
        assert!(matches!(outcome, RuleOutcome::NoEvidence { .. }));
    }

    #[test]
    fn a_generic_trap_is_not_turned_into_a_footprint_failure() {
        assert!(matches!(
            evaluate(Vec::new(), Vec::new()),
            RuleOutcome::NoEvidence { .. }
        ));
        let input = soroban_failure(R::Trapped, Vec::new(), None);
        assert!(matches!(
            analyze_ctx(&input, |ctx| FootprintEntryMissing.evaluate(ctx)),
            RuleOutcome::NoEvidence { .. }
        ));
    }

    #[test]
    fn an_archived_entry_is_not_a_footprint_failure() {
        let input = soroban_failure(R::EntryArchived, Vec::new(), Some(Vec::new()));
        assert!(matches!(
            analyze_ctx(&input, |ctx| FootprintEntryMissing.evaluate(ctx)),
            RuleOutcome::NotApplicable { .. }
        ));
    }

    #[test]
    fn a_message_with_the_marker_on_a_non_storage_error_is_ignored() {
        let outcome = evaluate(
            Vec::new(),
            vec![
                err(cid(5), ScError::Contract(1), "outside of the footprint"),
                host_fn_failed(ScError::Contract(1)),
            ],
        );
        assert!(matches!(outcome, RuleOutcome::NoEvidence { .. }));
    }
}
