# The canonical transaction model (M2)

`soroban_failure_analysis::TransactionModel` is the one representation of a
transaction that the engine — and every future rule — works against. It exists
so that nothing downstream needs to understand `stellar-xdr` layout.

```rust
let model = TransactionModel::from_input(&input);

model.is_fee_bumped();              // fee bumps unwrapped once, here
model.stage();                      // Option<FailureStage>; None = succeeded
model.invocation();                 // contract, function, args
model.soroban?.footprint;           // declared read-only / read-write keys
model.soroban?.declared;            // declared limits and resource fee
model.observed;                     // what the host reports having used
model.diagnostics.calls;            // reconstructed contract call tree
model.diagnostics.terminal_error;   // what the invocation failed with
```

Construction is infallible: its inputs are already-decoded XDR. Anything
inconsistent — say, an envelope that is fee-bumped paired with a result that is
not — is recorded in `model.notes` rather than preventing analysis.

## Pipeline

```
TransactionEnvelope ──► unwrap fee bump ──► inner Transaction
                                              ├─ source account, fee
                                              ├─ operations ──► OperationKind
                                              └─ ext V1 ──► SorobanData
                                                             ├─ auth entries
                                                             ├─ footprint (RO / RW)
                                                             └─ declared resources
TransactionResult ────► unwrap fee bump ──► inner result ──► Outcome
                                                             ├─ failed operation
                                                             └─ FailureStage
DiagnosticEvent[] ────► classify by first topic ──► Diagnostics
                                                             ├─ call tree
                                                             ├─ terminal error
                                                             └─ core_metrics ─┐
TransactionMeta (V3/V4) ─► fees charged ──────────────────────► ObservedResources
```

| Module | Responsibility |
|---|---|
| `model/envelope.rs` | Fee-bump unwrapping, operations, invocation |
| `model/outcome.rs` | Result unwrapping and `FailureStage` classification |
| `model/soroban.rs` | Auth, footprint, declared and observed resources |
| `model/events.rs` | Diagnostic event classification and call tree |

## Fee bumps

A fee-bump envelope wraps an ordinary V1 transaction. Its result is a wrapper
too, whose outer arm — `TxFeeBumpInnerSuccess` or `TxFeeBumpInnerFailed` — says
nothing about **why** the inner transaction failed. The real result is inside
`InnerTransactionResultPair`.

Every Soroban failure sampled from mainnet during M0 was fee-bumped, so this is
the common case rather than an edge case. The model unwraps both the envelope
and the result, and keeps the wrapper separately (`FeeBump`, `FeeBumpResult`) so
nothing is lost. `outcome.result_code` is always the *inner* code.

## FailureStage vs CauseClass

These are separate on purpose, and M2 only implements the first.

| | `FailureStage` | `CauseClass` |
|---|---|---|
| Question | *Where* did it fail? | *Why* did it fail? |
| Nature | Observation | Interpretation |
| Source | The result code | A rule, reading evidence |
| Milestone | **M2 — implemented** | M4 — not implemented |

`FailureStage` mapping, following issue #5:

| Result | Stage |
|---|---|
| `TxMalformed`, `TxBadAuth`, `TxBadAuthExtra`, `TxSorobanInvalid`, `TxTooEarly`, `TxTooLate`, `TxMissingOperation`, `TxNoAccount`, `TxNotSupported`, `TxBadSponsorship`, `OpBadAuth`, `OpNoAccount` | `Validation` |
| `TxBadSeq`, `TxBadMinSeqAgeOrGap` | `Sequence` |
| `TxInsufficientFee`, `TxInsufficientBalance` | `Fee` |
| a classic operation's failure code | `Operation` *(added in M2)* |
| `InvokeHostFunction::Malformed` | `HostFunction` |
| `InvokeHostFunction::Trapped` | `ContractExecution` |
| `InvokeHostFunction::ResourceLimitExceeded` (and TTL/restore ops) | `ResourceLimit` |
| `InvokeHostFunction::EntryArchived` | `StateArchival` |
| `InvokeHostFunction::InsufficientRefundableFee` (and TTL/restore ops) | `ResourceFee` |
| `TxInternalError`, `TxFrozenKeyAccessed`, TTL/restore `Malformed`, generic op wrappers | `Unknown` |

Notes on decisions:

- **`Operation` is new.** A classic operation failure (the negative-control
  fixture fails with `CreateClaimableBalance: NoTrust`) would otherwise only
  have been reportable as `Unknown`, which is inaccurate: the result *does*
  identify the stage. `FailureStage` is `#[non_exhaustive]`, so adding it is not
  a breaking change for matching code.
- **`Auth` and `Footprint` are never produced here.** Both surface as a plain
  `Trapped`; telling them apart needs diagnostic events, which is rule work (M4).
- **`TxFrozenKeyAccessed` is `Unknown`.** It was introduced in `stellar-xdr` 28
  and has not been observed in a real transaction, so its stage is not guessed.
- **Result unions are matched exhaustively.** A future `stellar-xdr` release
  adding a result code will fail to compile here, forcing a deliberate mapping
  rather than a silent wildcard.

## Diagnostic events

Events are classified by their first topic. These shapes were read off the real
mainnet fixtures:

| First topic | Other topics | Data | Emitting contract |
|---|---|---|---|
| `fn_call` | callee (32 bytes), function | arguments | the caller (none at top level) |
| `fn_return` | function | return value | the returning contract |
| `error` | `ScError` | message, or `[message, args…]` | the frame where it happened |
| `host_fn_failed` | `ScError` | void | none — this is the terminal error |
| `log` | — | message, or `[message, args…]` | the logging contract |
| `core_metrics` | metric name | `u64` | none |

Unrecognised events are kept as `EventKind::Other`, never dropped, and every
event keeps its raw topics and data.

**Availability is tracked separately from content.** `DiagnosticAvailability`
is `NotEmitted` when the source was not known to emit diagnostic events — then
an empty list carries *no information* — and `Emitted` when an empty list
genuinely means none were produced.

### Call tree

`fn_call` events carry no explicit parent, so the tree is reconstructed from the
fact that each event is emitted by the currently executing frame:

- `fn_call` from X to Y: unwind until X is on top, then push Y.
- `fn_return`: the top frame returned.
- Any other event from X: unwind until X is on top. Frames unwound this way
  ended without returning, so they failed with the last error they raised. This
  is what a caught `try_call` looks like.
- An emitter not on the stack does not unwind it.

On the 49-event fixture this recovers the real shape of the failure: one
`harvest` call into the outer contract, five nested `harvest` calls into the
farm contract that each failed with `#9` and were caught, then the outer frame
failing with `#2`.

## Declared vs observed resources

| | Source | Type |
|---|---|---|
| Declared | `SorobanTransactionData` | `DeclaredResources` — instructions, disk read bytes, write bytes, resource fee, archived-entry indexes |
| Observed | `core_metrics` events, meta fee fields | `ObservedResources` — every metric by name, plus `ChargedFees` |

They are different types because a resource rule's whole job is comparing one
against the other. Observed values are `Option`: absent when not reported, never
zero and never estimated. They come from diagnostic events, so they are the
reporting node's measurement rather than consensus data.

`disk_read_bytes` was called `read_bytes` before protocol 23. The model does not
claim that any `core_metrics` value corresponds to it; only `cpu_insn` has an
established mapping to a declared limit (`instructions`).

## Authorization and footprint

Exposed for M4 rules, with no rule applied:

- `AuthEntry` — credentials (`SourceAccount`, or an address with nonce and
  signature expiry; `Address`, `AddressV2` and `AddressWithDelegates` share a
  shape), the root contract and function, and the raw entry.
- `Footprint` — read-only and read-write `LedgerKey`s, `access(key)`, and
  `contract_code_hashes()`, which M3 uses to check contract specs.

All real fixtures authorize via the source account, so they carry no auth
entries. Address-credential extraction is covered by synthetic unit tests.

## For rule authors

`FailureContext` now carries `model: &TransactionModel`. Prefer it over
`ctx.input`: every quirk described on this page is already handled.
