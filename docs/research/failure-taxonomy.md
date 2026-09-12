# Soroban failure taxonomy (preliminary)

**Status:** preliminary. Grounded in XDR result types and in the transactions
actually inspected during M0 — not in speculation about what failures *might*
exist.

This document is the prose companion to
[`crates/soroban-failure-analysis/src/taxonomy.rs`](../../crates/soroban-failure-analysis/src/taxonomy.rs),
where the same taxonomy exists as code. When the two disagree, the code wins and
this document is the bug.

## Two axes, kept separate

The engine distinguishes **stage** from **cause**, and the separation is load
bearing.

| | Stage (`FailureStage`) | Cause (`CauseClass`) |
|---|---|---|
| What it is | An observation | An interpretation |
| Derived from | `TransactionResult`, `TransactionMeta` | Evidence, via a rule |
| Can it be wrong? | It is either right or wrong | It is a ranked claim with confidence |
| Needs evidence? | No — it is read directly | **Always** |

Collapsing these into one enum is how a diagnostic tool turns into a guessing
machine. `Unknown` and `Undetermined` are first-class, correct answers.

## Stages

Grounded in the XDR. `InvokeHostFunctionResult` has exactly six arms — `Success`,
`Malformed`, `Trapped`, `ResourceLimitExceeded`, `EntryArchived`,
`InsufficientRefundableFee` — and most stages below map onto one of them.

| Stage | Observed from | Reached contract code? |
|---|---|---|
| `Validation` | `tx_malformed`, `tx_bad_auth`, `tx_soroban_invalid`, `op_bad_auth` | no |
| `Sequence` | `tx_bad_seq`, `tx_bad_min_seq_age_or_gap` | no |
| `Fee` | `tx_insufficient_fee` (inclusion fee, not resource fee) | no |
| `Operation` | a classic operation's failure code, e.g. `CreateClaimableBalance: NoTrust` *(added in M2)* | no |
| `HostFunction` | `invoke_host_function_malformed` | no |
| `ContractExecution` | `invoke_host_function_trapped` | yes |
| `Auth` | trap with authorization evidence in diagnostic events | yes |
| `Footprint` | a ledger key accessed but not declared | yes |
| `StateArchival` | `invoke_host_function_entry_archived` | yes |
| `ResourceLimit` | `invoke_host_function_resource_limit_exceeded` | yes |
| `ResourceFee` | `invoke_host_function_insufficient_refundable_fee` | — |
| `Unknown` | anything else, or insufficient data | — |

The full mapping as implemented, with the reasoning for each edge case, is in
[`docs/architecture/transaction-model.md`](../architecture/transaction-model.md).

`Auth` and `Footprint` are the two stages **not** directly readable from a result
code. Both surface as a generic `Trapped`, and separating them requires reading
diagnostic events. That is precisely the gap this project exists to close — and
it is the same gap
[`stellar-core#3816`](https://github.com/stellar/stellar-core/issues/3816)
described in 2023.

## Cause classes

| Cause | Stage it usually sits at |
|---|---|
| `MissingAuthorizationEntry` | `Auth` |
| `InvalidAuthorizationEntry` | `Auth` |
| `FootprintEntryMissing` | `Footprint` |
| `ArchivedEntryRequiresRestore` | `StateArchival` |
| `ResourceLimitExceeded` | `ResourceLimit` |
| `InsufficientResourceFee` | `ResourceFee` |
| `ContractDefinedError` | `ContractExecution` |
| `ContractTrap` | `ContractExecution` |
| `MalformedHostFunction` | `HostFunction` |
| `Undetermined` | `Unknown` |

## What M0 actually observed

This is the honest part, and it constrains M4 more than anything else here.

**14 real mainnet transactions were inspected. The Soroban failures among them
collapse into a single category.**

| Category | Count | Notes |
|---|---|---|
| `ContractTrap` | 10 | All fee-bumped, all `invoke_host_function_trapped`. A further 3 Soroban failures were confirmed present-and-decodable but their result codes were not re-read after the fee-bump fix. |
| Classic (non-Soroban) | 1 | Negative control; no diagnostic events, as expected |
| Everything else | **0** | Not observed |

Three ledger clusters appeared, distinguished only by diagnostic-event count
(24 vs 49), and transactions within a cluster came from adjacent ledgers. They are almost
certainly the same arbitrage bots failing repeatedly.

### What this means

1. **The sample is heavily biased.** Ledger-adjacent failures on mainnet are
   dominated by automated trading, not by the hand-written contract bugs a
   developer-facing tool is meant to explain. The corpus is not representative of
   the *developer* failure population.
2. **Five of the six V1 rule categories have no real fixture yet.** Auth,
   footprint, archival, resource limit and resource fee were all absent.
3. **`Trapped` is confirmed to be exactly as opaque as reported.** Ten
   transactions, one result code, and no way to tell them apart without reading
   diagnostic events. This is direct evidence for the problem statement.

### Correction after M2 (2026-09-11)

The table above classified transactions by **result code**, which was all M0
could read. M2 decodes the diagnostic events, and they tell a different story
about the three Soroban fixtures. The M0 observation is left as written above;
this is what the events show:

| Fixture | Result code | What the diagnostic events show |
|---|---|---|
| `soroban-trapped-feebump-24ev` | `Trapped` | Terminal `Error(Storage, ExceededLimit)`, host message *"trying to access contract data key outside of the footprint"* |
| `soroban-trapped-feebump-49ev` | `Trapped` | Terminal `Error(Contract, #2)` = `NoHarvestablePails`, after five caught `Error(Contract, #9)` = `PailMissing` |
| `soroban-trapped-feebump-49ev-alt` | `Trapped` | Same shape as `-49ev` |

So the corpus holds real **evidence** for two cause categories, not zero:

- **`FootprintEntryMissing`** — the 24-event fixture. The host's message is
  consistent with it. Confirming the classification, and naming the key, is a
  rule's job; M2 only exposes the evidence.
- **`ContractDefinedError`** — both 49-event fixtures, now resolved to names
  from the contracts' real specs by M3.

Three identical `Trapped` results turned out to be two different kinds of
failure. That is the problem this project exists to solve, now observed
directly in its own corpus.

Still with **no** fixture: missing or invalid authorization, archived entry,
resource limit exceeded, insufficient resource fee.

### After M4 (2026-09-12)

A spread-out survey of 18,000 mainnet transactions (`survey_failures`) found
219 failed Soroban transactions in four shapes, and two new real authorization
failures were captured as fixtures:

| Count | Terminal error | Category | Rule |
|---|---|---|---|
| 175 | `Error(Storage, ExceededLimit)`, "outside of the footprint" | `FootprintEntryMissing` | `footprint_entry_missing` |
| 37 | `Error(Contract, #N)` | `ContractDefinedError` | `contract_defined_error` |
| 4 | `Error(Auth, InvalidInput)`, "signature has expired" | `InvalidAuthorizationEntry` | `invalid_authorization_entry` |
| 3 | `Error(Auth, ExistingValue)`, "nonce already exists for address" | `InvalidAuthorizationEntry` | `invalid_authorization_entry` |

Real fixtures per category now:

| Category | Real fixtures |
|---|---|
| `ContractDefinedError` | 2 |
| `FootprintEntryMissing` | 1 |
| `InvalidAuthorizationEntry` | 2 |
| `ArchivedEntryRequiresRestore` | **0** — rule validated synthetically only |
| `ResourceLimitExceeded` | **0** — rule validated synthetically only |
| `InsufficientResourceFee` | **0** — rule validated synthetically only |
| `MissingAuthorizationEntry` | **0** — no rule |

The survey found no `EntryArchived`, `ResourceLimitExceeded` or
`InsufficientRefundableFee` results at all, most likely because simulation stops
such transactions before submission. Those fixtures still have to be produced
deliberately on testnet; that work is tracked in
[issue #1](https://github.com/The-Big-Danny/Stellar-Developer-Observatory/issues/1).
See [`docs/architecture/rules.md`](../architecture/rules.md).

## Adding to this taxonomy

A new category needs, in one PR:

1. A variant in `FailureStage` or `CauseClass` with documentation.
2. A row in this table.
3. **A real fixture exhibiting it.** Not a hypothetical.

If you cannot produce a fixture, the category is not ready. The one narrow
exception, adopted in M4, is for *rules*: a rule whose only evidence is a
protocol result code defined as that exact cause may ship with synthetic tests,
marked unvalidated — see [ROADMAP.md](../../ROADMAP.md). It does not relax the
fixture requirement for adding a category, or for rules that interpret
diagnostic events.
