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

### `invalid_authorization_entry`

- **Evidence:** an `Error(Auth, …)` event with a recognised host message; the
  address in its data; the matching authorization entry.
- **Expired signature → Confirmed** only when the entry's own
  `signature_expiration_ledger` equals the expiry the host checked.
- **Reused nonce → Likely.** Whether a nonce was already consumed is ledger
  state that the transaction data cannot prove, so it is never confirmed.
- **Prevents it:** any other authorization error. An unrecognised
  `Error(Auth, …)` returns `NoEvidence`, because without knowing the failure a
  *missing* entry cannot be told from an *invalid* one.

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
| `MissingAuthorizationEntry` | No example in the corpus or the mainnet survey, so its evidence shape is unknown |
| `ContractTrap` (panic without a declared error) | Outside the initial six; evidence shape not yet studied |
| `MalformedHostFunction` | Outside the initial six |
| Any authorization failure other than expired signature or reused nonce | Returns `NoEvidence` rather than guessing |
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
