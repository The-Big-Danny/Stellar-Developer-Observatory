//! Structured diagnostic events.
//!
//! The host emits diagnostic events with a symbol as the first topic. The
//! shapes below were read off real mainnet transactions (see `fixtures/`), not
//! assumed:
//!
//! | First topic | Remaining topics | Data | Emitted by |
//! |---|---|---|---|
//! | `fn_call` | callee contract (32 bytes), function symbol | arguments | the caller (none at top level) |
//! | `fn_return` | function symbol | return value | the returning contract |
//! | `error` | the `ScError` | message string, or `[message, args…]` | the frame where it happened |
//! | `host_fn_failed` | the `ScError` | void | nobody — this is the terminal error |
//! | `log` | — | message, or `[message, args…]` | the logging contract |
//! | `core_metrics` | metric name symbol | `u64` | nobody |
//!
//! Diagnostic events are unmetered and **not part of consensus**. Their
//! message strings come from the host implementation and may change between
//! host versions. Everything here is an observation of what one node reported.

use std::collections::BTreeMap;

use stellar_xdr::{
    ContractEventBody, ContractEventType, ContractId, DiagnosticEvent, Hash, ScError, ScVal,
};

/// Whether diagnostic events can be interpreted at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticAvailability {
    /// The source was not known to emit diagnostic events. An empty event list
    /// carries **no information**: nothing may be concluded from it.
    NotEmitted,
    /// The source emits diagnostic events. An empty list means this transaction
    /// genuinely produced none.
    Emitted,
}

/// What a diagnostic event says, classified by its first topic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    /// A contract function was called.
    FnCall {
        /// The contract called.
        callee: ContractId,
        /// The function called.
        function: String,
    },
    /// A contract function returned normally.
    FnReturn {
        /// The function that returned.
        function: String,
    },
    /// An error was raised in the emitting frame.
    Error {
        /// The error.
        error: ScError,
        /// The host's message, when the data carries one.
        message: Option<String>,
    },
    /// The host function as a whole failed with this error. Terminal.
    HostFnFailed {
        /// The error the invocation failed with.
        error: ScError,
    },
    /// A log line.
    Log {
        /// The message, when the data carries one.
        message: Option<String>,
    },
    /// A resource metric reported by the host.
    CoreMetric {
        /// The metric name, e.g. `cpu_insn`.
        name: String,
        /// The value, when the data was a `u64`.
        value: Option<u64>,
    },
    /// Anything else. Retained, not dropped.
    Other {
        /// The first topic, when it was a symbol.
        name: Option<String>,
    },
}

/// One diagnostic event, classified, with its raw parts retained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagEvent {
    /// Zero-based index in the diagnostic event list. Stable: evidence cites it.
    pub index: u32,
    /// The contract whose frame emitted the event, if any.
    pub contract: Option<ContractId>,
    /// The raw `inSuccessfulContractCall` flag.
    pub in_successful_contract_call: bool,
    /// The raw event type.
    pub event_type: ContractEventType,
    /// The classified meaning.
    pub kind: EventKind,
    /// Topics, verbatim.
    pub topics: Vec<ScVal>,
    /// Data, verbatim.
    pub data: ScVal,
}

/// How a contract call ended, as far as the events show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallOutcome {
    /// A `fn_return` was seen for this frame.
    Returned,
    /// The frame ended after raising this error.
    Failed {
        /// The last error the frame raised.
        error: ScError,
        /// The event that raised it.
        event_index: u32,
    },
    /// The events do not show how the frame ended.
    Unknown,
}

/// One contract call, reconstructed from `fn_call` events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallFrame {
    /// The `fn_call` event that opened this frame.
    pub event_index: u32,
    /// Nesting depth; 0 is the call made by the transaction itself.
    pub depth: u32,
    /// The contract called.
    pub contract: ContractId,
    /// The function called.
    pub function: String,
    /// How the frame ended.
    pub outcome: CallOutcome,
}

/// The error the host function as a whole failed with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalError {
    /// The error, from the `host_fn_failed` event.
    pub error: ScError,
    /// Index of the `host_fn_failed` event.
    pub event_index: u32,
}

/// All diagnostic information for a transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostics {
    /// Whether an empty list means anything.
    pub availability: DiagnosticAvailability,
    /// Every event, in order.
    pub events: Vec<DiagEvent>,
    /// The contract call tree, in call order, flattened with depths.
    pub calls: Vec<CallFrame>,
    /// The terminal error, when a `host_fn_failed` event was present.
    pub terminal_error: Option<TerminalError>,
}

impl Diagnostics {
    pub(crate) fn from_events(raw: &[DiagnosticEvent], enabled: bool) -> Self {
        let events: Vec<DiagEvent> = raw
            .iter()
            .enumerate()
            .map(|(i, e)| classify(u32::try_from(i).unwrap_or(u32::MAX), e))
            .collect();
        let terminal_error = events.iter().find_map(|e| match &e.kind {
            EventKind::HostFnFailed { error } => Some(TerminalError {
                error: error.clone(),
                event_index: e.index,
            }),
            _ => None,
        });
        let calls = call_tree(&events, terminal_error.as_ref());
        Self {
            availability: if enabled {
                DiagnosticAvailability::Emitted
            } else {
                DiagnosticAvailability::NotEmitted
            },
            events,
            calls,
            terminal_error,
        }
    }

    /// Every `core_metrics` value, keyed by name. Exposed publicly as
    /// [`crate::model::ObservedResources::core_metrics`], the single place
    /// consumers read resource usage from.
    pub(crate) fn core_metrics(&self) -> BTreeMap<String, u64> {
        self.events
            .iter()
            .filter_map(|e| match &e.kind {
                EventKind::CoreMetric {
                    name,
                    value: Some(v),
                } => Some((name.clone(), *v)),
                _ => None,
            })
            .collect()
    }

    /// Every error event, in order.
    pub fn errors(&self) -> impl Iterator<Item = (&DiagEvent, &ScError, Option<&str>)> {
        self.events.iter().filter_map(|e| match &e.kind {
            EventKind::Error { error, message } => Some((e, error, message.as_deref())),
            _ => None,
        })
    }

    /// Number of events that are not `core_metrics` — the ones describing
    /// execution rather than accounting.
    pub fn execution_event_count(&self) -> usize {
        self.events
            .iter()
            .filter(|e| !matches!(e.kind, EventKind::CoreMetric { .. }))
            .count()
    }
}

/// Render an `ScError` the way the Soroban host and CLI print it:
/// `Error(Contract, #2)` or `Error(Storage, ExceededLimit)`.
pub fn error_label(error: &ScError) -> String {
    let code = match error {
        ScError::Contract(n) => return format!("Error(Contract, #{n})"),
        ScError::WasmVm(c)
        | ScError::Context(c)
        | ScError::Storage(c)
        | ScError::Object(c)
        | ScError::Crypto(c)
        | ScError::Events(c)
        | ScError::Budget(c)
        | ScError::Value(c)
        | ScError::Auth(c) => c,
    };
    format!("Error({}, {})", error.name(), code.name())
}

/// Render an `ScVal` compactly for evidence text, e.g. `vec[Block, 183188]`.
///
/// Bounded in depth, width and text length: values come from untrusted
/// transaction data, and evidence must stay readable whatever they contain.
pub fn value_label(v: &ScVal) -> String {
    let mut out = String::new();
    write_value(&mut out, v, 0);
    out
}

const LABEL_MAX_DEPTH: usize = 3;
const LABEL_MAX_ITEMS: usize = 8;
const LABEL_MAX_TEXT: usize = 64;

fn clipped(s: &str) -> String {
    if s.chars().count() <= LABEL_MAX_TEXT {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(LABEL_MAX_TEXT).collect::<String>())
    }
}

fn write_value(out: &mut String, v: &ScVal, depth: usize) {
    use core::fmt::Write;
    let _ = match v {
        ScVal::Bool(b) => write!(out, "{b}"),
        ScVal::Void => write!(out, "void"),
        ScVal::U32(n) => write!(out, "{n}"),
        ScVal::I32(n) => write!(out, "{n}"),
        ScVal::U64(n) => write!(out, "{n}"),
        ScVal::I64(n) => write!(out, "{n}"),
        ScVal::Symbol(s) => write!(out, "{}", clipped(&s.0.to_utf8_string_lossy())),
        ScVal::String(s) => write!(out, "\"{}\"", clipped(&s.0.to_utf8_string_lossy())),
        ScVal::Bytes(b) => {
            let hex: String = b.iter().take(8).map(|x| format!("{x:02x}")).collect();
            let more = if b.len() > 8 { "…" } else { "" };
            write!(out, "0x{hex}{more} ({} bytes)", b.len())
        }
        ScVal::Address(a) => write!(out, "{a}"),
        ScVal::Error(e) => write!(out, "{}", error_label(e)),
        ScVal::Vec(Some(items)) if depth < LABEL_MAX_DEPTH => {
            out.push_str("vec[");
            for (i, item) in items.iter().take(LABEL_MAX_ITEMS).enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_value(out, item, depth + 1);
            }
            if items.len() > LABEL_MAX_ITEMS {
                out.push_str(", …");
            }
            write!(out, "]")
        }
        ScVal::Map(Some(entries)) if depth < LABEL_MAX_DEPTH => {
            out.push_str("map{");
            for (i, e) in entries.iter().take(LABEL_MAX_ITEMS).enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_value(out, &e.key, depth + 1);
                out.push_str(": ");
                write_value(out, &e.val, depth + 1);
            }
            if entries.len() > LABEL_MAX_ITEMS {
                out.push_str(", …");
            }
            write!(out, "}}")
        }
        ScVal::Vec(Some(_)) => write!(out, "vec[…]"),
        ScVal::Map(Some(_)) => write!(out, "map{{…}}"),
        other => write!(out, "<{}>", other.name()),
    };
}

fn symbol(v: &ScVal) -> Option<String> {
    match v {
        ScVal::Symbol(s) => Some(s.0.to_utf8_string_lossy()),
        _ => None,
    }
}

/// The message in an `error`/`log` event's data: either a bare string, or the
/// first element of a vector.
fn message(data: &ScVal) -> Option<String> {
    match data {
        ScVal::String(s) => Some(s.0.to_utf8_string_lossy()),
        ScVal::Vec(Some(v)) => match v.first()? {
            ScVal::String(s) => Some(s.0.to_utf8_string_lossy()),
            _ => None,
        },
        _ => None,
    }
}

fn classify(index: u32, e: &DiagnosticEvent) -> DiagEvent {
    let ContractEventBody::V0(body) = &e.event.body;
    let topics = body.topics.to_vec();
    let data = body.data.clone();

    let first = topics.first().and_then(symbol);
    let kind = match (first.as_deref(), topics.get(1), topics.get(2)) {
        (Some("fn_call"), Some(ScVal::Bytes(callee)), Some(func)) => {
            match (<[u8; 32]>::try_from(callee.as_slice()), symbol(func)) {
                (Ok(id), Some(function)) => EventKind::FnCall {
                    callee: ContractId(Hash(id)),
                    function,
                },
                _ => EventKind::Other { name: first },
            }
        }
        (Some("fn_return"), Some(func), _) => match symbol(func) {
            Some(function) => EventKind::FnReturn { function },
            None => EventKind::Other { name: first },
        },
        (Some("error"), Some(ScVal::Error(error)), _) => EventKind::Error {
            error: error.clone(),
            message: message(&data),
        },
        (Some("host_fn_failed"), Some(ScVal::Error(error)), _) => EventKind::HostFnFailed {
            error: error.clone(),
        },
        (Some("log"), _, _) => EventKind::Log {
            message: message(&data),
        },
        (Some("core_metrics"), Some(name), _) => match symbol(name) {
            Some(name) => EventKind::CoreMetric {
                name,
                value: match &data {
                    ScVal::U64(v) => Some(*v),
                    _ => None,
                },
            },
            None => EventKind::Other { name: first },
        },
        _ => EventKind::Other { name: first },
    };

    DiagEvent {
        index,
        contract: e.event.contract_id.clone(),
        in_successful_contract_call: e.in_successful_contract_call,
        event_type: e.event.type_,
        kind,
        topics,
        data,
    }
}

/// Reconstruct the call tree.
///
/// Events are emitted by whichever frame is executing, so the emitting contract
/// tells us where on the stack we are:
///
/// * `fn_call` from contract X to Y: unwind until X is on top, then push Y.
///   A `fn_call` with no emitter is the transaction's own call: reset.
/// * `fn_return`: the top frame returned.
/// * Any other event from contract X: unwind until X is on top. A frame that is
///   unwound this way ended without returning, so it failed with the last
///   error it raised — this is what a caught `try_call` failure looks like.
///
/// Frames still open at the end failed with their last error, or with the
/// terminal error when they raised none themselves.
fn call_tree(events: &[DiagEvent], terminal: Option<&TerminalError>) -> Vec<CallFrame> {
    struct Open {
        frame: usize,
        last_error: Option<(ScError, u32)>,
    }

    fn end(frames: &mut [CallFrame], open: Open) {
        frames[open.frame].outcome = match open.last_error {
            Some((error, event_index)) => CallOutcome::Failed { error, event_index },
            None => CallOutcome::Unknown,
        };
    }

    fn unwind_to(frames: &mut [CallFrame], stack: &mut Vec<Open>, contract: &ContractId) {
        // Only unwind if the emitter is actually on the stack; an unexpected
        // emitter must not wipe the stack.
        if !stack.iter().any(|o| &frames[o.frame].contract == contract) {
            return;
        }
        while let Some(top) = stack.last() {
            if &frames[top.frame].contract == contract {
                break;
            }
            let o = stack.pop().expect("checked non-empty");
            end(frames, o);
        }
    }

    let mut frames: Vec<CallFrame> = Vec::new();
    let mut stack: Vec<Open> = Vec::new();

    for e in events {
        match (&e.kind, &e.contract) {
            (EventKind::FnCall { callee, function }, caller) => {
                match caller {
                    Some(c) => unwind_to(&mut frames, &mut stack, c),
                    None => {
                        while let Some(o) = stack.pop() {
                            end(&mut frames, o);
                        }
                    }
                }
                frames.push(CallFrame {
                    event_index: e.index,
                    depth: u32::try_from(stack.len()).unwrap_or(u32::MAX),
                    contract: callee.clone(),
                    function: function.clone(),
                    outcome: CallOutcome::Unknown,
                });
                stack.push(Open {
                    frame: frames.len() - 1,
                    last_error: None,
                });
            }
            (EventKind::FnReturn { .. }, _) => {
                if let Some(o) = stack.pop() {
                    frames[o.frame].outcome = CallOutcome::Returned;
                }
            }
            (kind, Some(c)) => {
                unwind_to(&mut frames, &mut stack, c);
                if let (EventKind::Error { error, .. }, Some(top)) = (kind, stack.last_mut()) {
                    if &frames[top.frame].contract == c {
                        top.last_error = Some((error.clone(), e.index));
                    }
                }
            }
            _ => {}
        }
    }

    while let Some(mut o) = stack.pop() {
        if o.last_error.is_none() {
            o.last_error = terminal.map(|t| (t.error.clone(), t.event_index));
        }
        end(&mut frames, o);
    }

    frames
}

#[cfg(test)]
mod tests {
    //! Synthetic events built in code, to pin down the classification and
    //! call-tree rules. Real mainnet events are covered by the fixture tests.

    use super::*;
    use crate::testutil::{call, cid, err, ev, host_fn_failed as failed, sym};

    #[test]
    fn empty_input_distinguishes_not_emitted_from_none_emitted() {
        assert_eq!(
            Diagnostics::from_events(&[], false).availability,
            DiagnosticAvailability::NotEmitted
        );
        assert_eq!(
            Diagnostics::from_events(&[], true).availability,
            DiagnosticAvailability::Emitted
        );
    }

    #[test]
    fn fn_call_decodes_callee_bytes_as_a_contract_id() {
        let d = Diagnostics::from_events(&[call(None, cid(9), "work")], true);
        assert_eq!(
            d.events[0].kind,
            EventKind::FnCall {
                callee: cid(9),
                function: "work".into()
            }
        );
    }

    #[test]
    fn malformed_fn_call_is_retained_as_other_not_dropped() {
        let bad = ev(None, vec![sym("fn_call"), sym("not-bytes")], ScVal::Void);
        let d = Diagnostics::from_events(&[bad], true);
        assert!(matches!(d.events[0].kind, EventKind::Other { .. }));
        assert!(d.calls.is_empty());
    }

    #[test]
    fn caught_inner_failure_and_terminal_outer_failure_are_both_reconstructed() {
        // A calls B; B fails with #9; A catches it, then fails with #2.
        // This is the shape of the real 49-event mainnet fixtures.
        let (a, b) = (cid(1), cid(2));
        let d = Diagnostics::from_events(
            &[
                call(None, a.clone(), "harvest"),
                call(Some(a.clone()), b.clone(), "harvest"),
                err(
                    b.clone(),
                    ScError::Contract(9),
                    "failing with contract error",
                ),
                err(a.clone(), ScError::Contract(9), "contract try_call failed"),
                err(
                    a.clone(),
                    ScError::Contract(2),
                    "failing with contract error",
                ),
                failed(ScError::Contract(2)),
            ],
            true,
        );

        assert_eq!(d.calls.len(), 2);
        assert_eq!((d.calls[0].depth, &d.calls[0].contract), (0, &a));
        assert_eq!((d.calls[1].depth, &d.calls[1].contract), (1, &b));
        assert!(matches!(
            d.calls[1].outcome,
            CallOutcome::Failed {
                error: ScError::Contract(9),
                ..
            }
        ));
        assert!(matches!(
            d.calls[0].outcome,
            CallOutcome::Failed {
                error: ScError::Contract(2),
                ..
            }
        ));
        assert_eq!(d.terminal_error.unwrap().error, ScError::Contract(2));
    }

    #[test]
    fn a_frame_that_raised_nothing_takes_the_terminal_error() {
        let d = Diagnostics::from_events(
            &[call(None, cid(1), "f"), failed(ScError::Contract(4))],
            true,
        );
        assert!(matches!(
            d.calls[0].outcome,
            CallOutcome::Failed {
                error: ScError::Contract(4),
                ..
            }
        ));
    }

    #[test]
    fn fn_return_marks_the_frame_returned() {
        let ret = ev(Some(cid(2)), vec![sym("fn_return"), sym("g")], ScVal::Void);
        let d = Diagnostics::from_events(
            &[
                call(None, cid(1), "f"),
                call(Some(cid(1)), cid(2), "g"),
                ret,
            ],
            true,
        );
        assert_eq!(d.calls[1].outcome, CallOutcome::Returned);
    }

    #[test]
    fn an_emitter_not_on_the_stack_does_not_unwind_it() {
        let d = Diagnostics::from_events(
            &[
                call(None, cid(1), "f"),
                err(cid(99), ScError::Contract(1), "stray"),
                failed(ScError::Contract(3)),
            ],
            true,
        );
        // The stray error must not be attributed to, or end, frame 1.
        assert!(matches!(
            d.calls[0].outcome,
            CallOutcome::Failed {
                error: ScError::Contract(3),
                ..
            }
        ));
    }

    #[test]
    fn core_metrics_are_collected_by_name() {
        let m = ev(
            None,
            vec![sym("core_metrics"), sym("cpu_insn")],
            ScVal::U64(42),
        );
        let d = Diagnostics::from_events(&[m], true);
        assert_eq!(d.core_metrics().get("cpu_insn"), Some(&42));
        assert_eq!(d.execution_event_count(), 0);
    }
}
