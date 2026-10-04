# Failure classification rules (M4)

M2 answers *where* a transaction failed. M3 names contract errors. M4 answers
the question the project exists for: **what evidence explains why it failed?**

The classifier is deterministic and rule-based. It would rather say "unknown"
than name a cause the evidence does not support.

## How a diagnosis is built

Every rule returns one of three outcomes:

| Outcome | Meaning |
|---|---|
| `Match(CandidateCause)` | The evidence supports a cause, at a stated confidence |
| `NoEvidence { reason }` | The rule could apply, but what it needs is absent or inconclusive |
| `NotApplicable { reason }` | The rule does not concern this transaction |

Every outcome is kept in `Diagnosis::rule_reports`. Candidates are ranked by
confidence, with ties broken by registration order. `Diagnosis::verdict()`
then gives the overall answer:

| Verdict | When |
|---|---|
| `Explained(confidence)` | at least one rule matched |
| `InsufficientEvidence` | some rule could apply, none found enough evidence — **unknown** |
| `Unsupported` | no implemented rule concerns this kind of failure |
| `NotAFailure` | the transaction succeeded |

`InsufficientEvidence` and `Unsupported` are different answers on purpose. The
first says "we looked and could not tell"; the second says "we have no rule for
this".

### Confidence

| | Means |
|---|---|
| `Confirmed` | The host or protocol states the cause **and** the transaction's own data independently corroborates it — or the protocol result code itself is defined as that cause |
| `Likely` | The host states the cause, but no independent corroboration is available |
| `Possible` | Consistent with the evidence, but did not end the invocation, or the evidence conflicts |

## The rules

| Rule id | Cause | Fires on | Confirmed when | Real fixtures |
|---|---|---|---|---|
| `archived_entry` | `ArchivedEntryRequiresRestore` | result code `EntryArchived` | always (protocol-defined code) | **none** |
| `resource_limit_exceeded` | `ResourceLimitExceeded` | result code `ResourceLimitExceeded` | always (protocol-defined code) | **none** |
| `insufficient_resource_fee` | `InsufficientResourceFee` | result code `InsufficientRefundableFee` | always (protocol-defined code) | **none** |
| `contract_defined_error` | `ContractDefinedError` | terminal `Error(Contract, #N)` | name resolved **and** spec WASM is in the footprint | `soroban-trapped-feebump-49ev`, `-49ev-alt` |
| `invalid_authorization_entry` | `InvalidAuthorizationEntry` | host auth error: expired signature or reused nonce | expired signature corroborated by the envelope's own expiration ledger | `soroban-auth-signature-expired`, `soroban-auth-nonce-reused` |
| `footprint_entry_missing` | `FootprintEntryMissing` | host "outside of the footprint" storage error | accessed key extracted **and** absent from the footprint | `soroban-trapped-feebump-24ev` |
| `contract_trap` | `ContractTrap` | terminal `Error(WasmVm, InvalidAction)` with the `UnreachableCodeReached` host message | never (host text and error type, no source location) | `soroban-trapped-testnet-22ev` (real testnet, deliberately caused) |
| `missing_authorization_entry` | `MissingAuthorizationEntry` | host auth error `Unauthorized function call for address` | the error ended the invocation **and** no authorization entry carries that address | `soroban-auth-missing-testnet` (real testnet, deliberately caused) |

Each rule's module documentation (`crates/soroban-failure-analysis/src/rules/`)
states exactly what triggers it, what prevents it, and its confidence table.

### `contract_defined_error`

- **Evidence:** the `host_fn_failed` event; the M3 report for that code
  (identification, resolution, provenance); caught contract errors, cited as
  evidence and never as a second cause.
- **Prevents it:** a terminal host error; no diagnostic events; no terminal event.
- **False-positive risk:** low for the *class* — contracts cannot raise
  non-`Contract` error types. The *name* relies on M3's origin attribution, which
  uses a host message to break ties between emitters.
- **Deliberately does not say** why the contract raised the error. A name says
  what the contract reported, not who is at fault.

### `footprint_entry_missing`

- **Evidence:** an `Error(Storage, …)` event with the host message; the key
  extracted from its `[message, address, key]` data; the declared footprint.
- **Prevents it:** a storage error without the marker message, since
  `ExceededLimit` alone also covers other limits; `EntryArchived`; no diagnostic
  events.
- **Conflicting evidence:** if the footprint *does* declare the key, the rule
  reports `Possible` and cites the declaring entry rather than confirming.
- **False-positive risk:** durability is not in the event, so a key declared
  under the other durability counts as declared. That can only lower
  confidence, never raise it.

### `contract_trap`

- **Evidence:** the `host_fn_failed` event with `Error(WasmVm, InvalidAction)`;
  the error event whose host message is `VM call trapped: UnreachableCodeReached`,
  which is a WebAssembly `unreachable` instruction. The emitting contract and
  the function symbol come from that event, so the candidate names the frame.
- **Confidence: `Likely`, never `Confirmed`.** The message is a host diagnostic
  string, and no field names the source line that trapped. Confirming it would
  mean claiming a panic site the evidence does not show.
- **Prevents it:** any other `WasmVm` trap (out-of-bounds memory, division by
  zero and so on), which returns `NoEvidence`. A declared contract error, and
  host errors such as `Error(Auth, …)`, which return `NotApplicable`. No
  diagnostic events.
- **False-positive risk:** a `WasmVm` `unreachable` trap is not always a panic
  written by the contract author. The rule therefore claims only "the contract
  trapped on `unreachable`", and its remediation lists the panic sites to check.

### `missing_authorization_entry`

- **Evidence:** an `Error(Auth, InvalidAction)` event with the host message
  `Unauthorized function call for address`, whose data names the address that
  did not authorize; the envelope's authorization list.
- **Confirmed** when the error ended the invocation and no authorization entry
  in the envelope carries credentials for that address. The absence is checked
  in the transaction, not inferred from the message.
- **Likely** when the error did not end the invocation, or the address cannot be
  read from the event.
- **Prevents it:** the transaction's own source account, which authorizes its
  invocations implicitly (`NotApplicable`); an address that does have an entry
  (`NoEvidence`, see below); any other authorization error.
- **Deliberately does not say** that the caller forgot to sign. The transaction
  cannot show whether the entry was never built or was built and then removed.

### `invalid_authorization_entry`

- **Evidence:** an `Error(Auth, …)` event with a recognised host message; the
  address in its data; the matching authorization entry.
- **Expired signature → Confirmed** only when the entry's own
  `signature_expiration_ledger` equals the expiry the host checked.
- **Reused nonce → Likely.** Whether a nonce was already consumed is ledger
  state that the transaction data cannot prove, so it is never confirmed.
- **Prevents it:** a missing entry, which is `missing_authorization_entry`'s
  message ("Unauthorized function call for address"); any authorization error
  whose message this rule does not recognise, which returns `NoEvidence`. The
  rule does not guess that an unknown failure is an invalid entry.

### Missing or invalid: how the evidence tells them apart

The two rules read different host messages, so one authorization failure is
never claimed by both:

| Host message (`Error(Auth, …)`) | Meaning | Rule | Confidence |
|---|---|---|---|
| `Unauthorized function call for address` | no entry for the address | `missing_authorization_entry` | `Confirmed` when the error ended the invocation and the envelope has no credentials for the address; otherwise `Likely` |
| `signature has expired` | an entry exists, but its signature is out of date | `invalid_authorization_entry` | `Confirmed` when the error ended the invocation and the entry's expiry matches the one the host checked; `Likely` when it ended without that match; `Possible` when it did not end the invocation |
| `nonce already exists for address` | an entry exists, but its nonce was already used | `invalid_authorization_entry` | `Likely` when the error ended the invocation and the address has an entry; otherwise `Possible`. Never `Confirmed` |
| any other message | unknown | neither | `NoEvidence` |

One case is claimed by neither rule: an authorization error whose address *has*
an entry in the envelope but whose message is not one of the two above. Both
rules return `NoEvidence` for it. It is reported as unknown, not as a missing or
invalid entry.

### Result-code rules

`archived_entry`, `resource_limit_exceeded` and `insufficient_resource_fee`
fire on protocol result codes whose definition *is* the cause, so the code
alone confirms the class. None of them names the specific entry or resource
dimension: the result code does not carry it, and the rules only attach
declared and observed values as observations.

`insufficient_resource_fee` explicitly refuses `TxInsufficientFee`, a
transaction-level inclusion-fee rejection that is a different failure with a
different fix.

**These three have no real fixture.** They are validated only by synthetic
tests. A mainnet survey of 18,000 transactions found none of these result
codes, most likely because simulation stops such transactions before
submission.

## What is not classified yet

| Category | Why not |
|---|---|
| `MalformedHostFunction` | Outside the initial six |
| Any authorization failure other than expired signature, reused nonce or missing entry | Returns `NoEvidence` rather than guessing |
| Classic operation failures | Not Soroban; reported as `Unsupported` |

## Host messages are not a stable API

The footprint and authorization rules match host diagnostic messages:

- `"outside of the footprint"`
- `"signature has expired"`
- `"nonce already exists for address"`

They are paired with *structured* checks: the error type, the key checked
against the footprint, the address matched to an auth entry, and the expiry
compared with the envelope. If a host release rewords a message, the rule stops
matching and degrades to `NoEvidence`. It never produces a wrong diagnosis. Diagnostic events are
also not part of consensus.

## Evidence from mainnet

`survey_failures` sampled 18,000 mainnet transactions across the RPC retention
window (2026-09-12). Of 219 failed Soroban transactions:

| Count | Result | Terminal error | Rule |
|---|---|---|---|
| 175 | `Trapped` | `Error(Storage, ExceededLimit)`, "outside of the footprint" | `footprint_entry_missing` |
| 37 | `Trapped` | `Error(Contract, #N)` | `contract_defined_error` |
| 4 | `Trapped` | `Error(Auth, InvalidInput)`, "signature has expired" | `invalid_authorization_entry` |
| 3 | `Trapped` | `Error(Auth, ExistingValue)`, "nonce already exists" | `invalid_authorization_entry` |

Every failure in the sample fell into a category that now has a rule. It is one
sample in one week, dominated by automated traffic, so it is not a general
accuracy measurement. That is M5.

```bash
cargo run -p soroban-failure-rpc --example survey_failures -- --pages 90
```

## Adding a rule

1. Create `crates/soroban-failure-analysis/src/rules/<rule>.rs` implementing
   `Rule`, with module docs stating what triggers it, what prevents it, its
   confidence table, and its fixtures.
2. Return `NotApplicable` or `NoEvidence` **with a reason** whenever the
   evidence is not there. Never return a low-confidence guess instead.
3. Register it in `RuleRegistry::builtin()` and update the tripwire test that
   lists rule ids.
4. Add synthetic tests for every confidence level and every non-match path.
5. Add a real fixture and a test in `crates/sdo/tests/classification.rs`. If no
   real fixture exists, say so in the rule's docs and in this file.
