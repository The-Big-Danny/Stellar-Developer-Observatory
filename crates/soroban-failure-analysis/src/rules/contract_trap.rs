//! Rule: the contract trapped by executing a WebAssembly `unreachable`
//! instruction, the way a Rust panic does.
//!
//! **Fires on:** the invocation ended with `Error(WasmVm, InvalidAction)`, and
//! an error event carries the host message [`UNREACHABLE_MARKER`]
//! (`"VM call trapped: UnreachableCodeReached"`).
//!
//! **Confidence:**
//!
//! | Evidence | Confidence |
//! |---|---|
//! | terminal `Error(WasmVm, InvalidAction)` and the `UnreachableCodeReached` message | `Likely` |
//!
//! `Confirmed` is deliberately out of reach. The host message is a diagnostic
//! string, and nothing in the transaction names the contract line that trapped.
//! The rule therefore never claims *which* panic occurred, only that the
//! contract's own code trapped.
//!
//! **Does not fire on:**
//!
//! - any other `WasmVm` trap (out-of-bounds memory, division by zero and so on).
//!   Those reach [`NoEvidence`](crate::rule::RuleOutcome::NoEvidence): the rule
//!   refuses to call every generic `function-trapped` transaction a panic.
//! - a contract-declared error, `Error(Contract, #N)`. That is
//!   [`super::ContractDefinedError`]'s evidence, and the rule stays silent.
//! - host errors such as `Error(Auth, …)`, `Error(Storage, …)` or
//!   `Error(Budget, …)`, which have their own rules or none.
//! - a transaction with no diagnostic events. Without them the trap cannot be
//!   distinguished from any other.
//!
//! **Deliberately does not claim:** which `unwrap`, `expect`, `panic!` or
//! failed assertion caused the trap. The remediation names the candidates; the
//! diagnostics cannot pick one.
//!
//! The message is a host diagnostic string, not a stable API. If a host release
//! rewords it, this rule degrades to "no evidence", never to a false diagnosis.
//!
//! **Shape, from the fixture:** the terminal event is an `error` diagnostic
//! whose topics are `[error, Error(WasmVm, InvalidAction)]` and whose data is
//! `["VM call trapped: UnreachableCodeReached", Symbol("trigger")]`. The
//! emitting contract and the function name come from that event, so the
//! candidate names the frame that trapped.
//!
//! **Fixtures:** `soroban-trapped-testnet-22ev` (real Soroban testnet). The
//! transaction invoked a contract whose `trigger` function calls `unwrap()` on
//! `None`; the source is in `fixtures/contract-sources/contract-trap/`.

use stellar_xdr::{ScError, ScVal};

use crate::diagnosis::{CandidateCause, Confidence, Evidence, EvidenceSource};
use crate::model::{error_label, DiagnosticAvailability};
use crate::rule::{FailureContext, Rule, RuleOutcome};
use crate::taxonomy::{CauseClass, FailureStage};

/// The host message for a WebAssembly `unreachable` instruction, as observed on
/// testnet: `VM call trapped: UnreachableCodeReached`.
pub const UNREACHABLE_MARKER: &str = "UnreachableCodeReached";

/// The function whose frame trapped. The recorded error event carries it as the
/// second data element, after the message: `[message, Symbol(function)]`.
fn trap_frame(data: &ScVal) -> Option<String> {
    let ScVal::Vec(Some(items)) = data else {
        return None;
    };
    match items.get(1)? {
        ScVal::Symbol(name) => Some(String::from_utf8_lossy(name.as_slice()).into_owned()),
        _ => None,
    }
}

/// See the [module documentation](self).
pub struct ContractTrap;

impl Rule for ContractTrap {
    fn id(&self) -> &'static str {
        "contract_trap"
    }

    fn description(&self) -> &'static str {
        "The contract trapped on a WebAssembly `unreachable` instruction, as a panic does"
    }

    fn evaluate(&self, ctx: &FailureContext<'_>) -> RuleOutcome {
        let m = ctx.model;
        let Some(stage) = m.stage() else {
            return RuleOutcome::not_applicable("the transaction succeeded");
        };
        if stage != FailureStage::ContractExecution {
            return RuleOutcome::not_applicable(format!(
                "the transaction failed at stage `{stage}`; a panic ends contract execution \
                 with a trap"
            ));
        }
        if m.diagnostics.availability == DiagnosticAvailability::NotEmitted {
            return RuleOutcome::no_evidence(
                "diagnostic events were not available, and the trap reason is only visible in \
                 them",
            );
        }
        let Some(terminal) = &m.diagnostics.terminal_error else {
            return RuleOutcome::no_evidence(
                "no `host_fn_failed` event states which error ended the invocation",
            );
        };
        match terminal.error {
            ScError::WasmVm(_) => {}
            ScError::Contract(code) => {
                return RuleOutcome::not_applicable(format!(
                    "the invocation ended with the contract-defined error #{code}, not a trap"
                ));
            }
            ref other => {
                return RuleOutcome::not_applicable(format!(
                    "the invocation ended with {}, a host error rather than a contract trap",
                    error_label(other)
                ));
            }
        }

        let Some((event, _, message)) = m.diagnostics.errors().find(|(_, e, msg)| {
            **e == terminal.error && msg.is_some_and(|s| s.contains(UNREACHABLE_MARKER))
        }) else {
            return RuleOutcome::no_evidence(format!(
                "the invocation ended with {}, but no event identifies it as an `unreachable` \
                 trap, so the trap is not attributed to a panic",
                error_label(&terminal.error)
            ));
        };

        let evidence = vec![
            Evidence::new(
                EvidenceSource::DiagnosticEvent {
                    index: terminal.event_index,
                },
                format!(
                    "`host_fn_failed` reports the invocation ended with {}",
                    error_label(&terminal.error)
                ),
            ),
            Evidence::new(
                EvidenceSource::DiagnosticEvent { index: event.index },
                format!(
                    "host message \"{}\": a WebAssembly `unreachable` instruction executed",
                    message.unwrap_or_default()
                ),
            ),
        ];

        let frame = trap_frame(&event.data);
        let where_ = match (&event.contract, &frame) {
            (Some(c), Some(f)) => format!(" in contract {c}, function `{f}`"),
            (Some(c), None) => format!(" in contract {c}"),
            (None, _) => String::new(),
        };

        RuleOutcome::Match(CandidateCause {
            class: CauseClass::ContractTrap,
            confidence: Confidence::Likely,
            summary: format!(
                "The contract trapped{where_} by executing a WebAssembly `unreachable` \
                 instruction, as a panic does. The trap is not a contract-declared error, and \
                 the diagnostics do not identify the panic site."
            ),
            evidence,
            remediation: Some(
                "Find the panic sites in the contract that this invocation can reach: `unwrap()` \
                 or `expect()` on an `Option` or `Result`, `panic!`, a failed `assert!`, or an \
                 out-of-range index. Reproduce the call against a local or testnet build, with \
                 the same arguments and ledger state, to see which one fires."
                    .to_string(),
            ),
            rule_id: self.id().into(),
        })
    }
}
