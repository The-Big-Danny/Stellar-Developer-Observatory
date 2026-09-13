# Roadmap

A milestone is marked complete only when the work exists in the repository and
is covered by tests. Nothing below is aspirational marketing.

| Milestone | Status |
|---|---|
| **M0 — Feasibility** | ✅ **Complete** (2026-09-10) |
| **M1 — Foundation** | ✅ **Complete** (2026-09-10) |
| **M2 — Transaction & XDR engine** | ✅ **Complete** (2026-09-11) |
| **M3 — Contract error resolution** | ✅ **Complete** (2026-09-11) |
| **M4 — Failure classification** | 🟡 **Partly complete** (2026-09-12) — 3 of 6 categories validated on real data |
| M5 — Accuracy & reliability | ⬜ Planned |
| M6 — Developer experience | ⬜ Planned |
| M7 — Ecosystem integration | ⬜ Planned |
| M8 — Observatory expansion | ⬜ Future |

---

## ✅ M0 — Feasibility

**Question:** do public mainnet RPC providers actually return decodable
diagnostic events for real failed Soroban transactions? If not, the project as
designed is dead.

**Answer: yes.** 4 of 4 responding providers, 13 of 13 transactions, 437 of 437
diagnostic events decoded. Decision: **GO**.

Delivered:

- `sdo-probe` with `probe`, `scan` and `capture` subcommands
- Per-provider measurements across every free public mainnet endpoint
- 4 real recorded fixtures, including a negative control
- A preliminary failure taxonomy grounded in XDR result types
- [M0 report](docs/research/m0-report.md),
  [RPC measurements](docs/research/rpc-diagnostic-events.md),
  [taxonomy](docs/research/failure-taxonomy.md)

Findings that changed the design: current mainnet returns `TransactionMetaV4`
(diagnostic events moved out of `sorobanMeta`); failed Soroban transactions are
overwhelmingly **fee-bumped**, so the real result is in the inner result pair;
and `stellar-xdr` 28 removed the `curr` module that every tutorial uses.

## ✅ M1 — Foundation

A Rust workspace with clear boundaries, offline tests, and CI.

Delivered:

- `soroban-failure-analysis` — taxonomy, `Diagnosis`, `Evidence`, `Rule` trait,
  `RuleRegistry`, ranking. **No I/O. No rules yet.**
- `soroban-failure-rpc` — RPC client, pure decoding, fixture loading, handling
  both `TransactionMeta` V3 and V4
- `sdo` CLI with `explain`, including `--fixture` for fully offline use
- 40 tests, all passing, **none requiring network access**
- CI running fmt, clippy (`-D warnings`), and tests on stable and MSRV (1.88)
- README, CONTRIBUTING, SECURITY, CODE_OF_CONDUCT, LICENSE, architecture docs

The engine runs end to end and reports honestly that it cannot yet attribute a
cause.

---

## ✅ M2 — Transaction & XDR engine

**Goal:** determine *where* a transaction failed, from one canonical model.

Delivered — see [transaction-model.md](docs/architecture/transaction-model.md):

- `TransactionModel`: the single representation every rule works against
- **Fee-bump unwrapping** of both envelope and result; the wrapper is kept
  separately rather than mistaken for the cause
- `FailureStage` classification from the result, per issue #5, with exhaustive
  matching so a new `stellar-xdr` result code cannot be silently mismapped.
  Adds `FailureStage::Operation` for classic operation failures, which could
  previously only be reported as `Unknown`
- Operations, invocation (contract, function, arguments), auth entries, footprint
- **Declared** resources kept separate from **observed** consumption
  (`core_metrics` diagnostic events, and fees charged from the meta)
- Diagnostic events classified by topic, with the contract **call tree
  reconstructed** and the terminal error identified
- `sdo explain` shows all of it

**Done when** every fixture reports a correct, non-`Unknown` stage and the
`FailureStage` limitation is gone from `analyze`: ✅ both, asserted by tests.

M2 also corrected an M0 finding. By result code, the three Soroban fixtures
were all "`ContractTrap`". By their diagnostic events, one is a storage access
outside the footprint and two are contract-defined errors — see the
[taxonomy correction](docs/research/failure-taxonomy.md#correction-after-m2-2026-09-11).

## ✅ M3 — Contract error resolution

**Goal:** turn `Error(Contract, #3)` into the name the contract declared.

Delivered — see [contract-errors.md](docs/architecture/contract-errors.md):

- Contract identification that handles several contracts and errors re-emitted
  by callers, reporting ambiguity instead of picking one
- Contract instance and WASM fetched via `getLedgerEntries`, behind a
  `ContractSource` trait that also reads recorded fixtures
- `contractspecv0` parsing with a depth limit, since contract WASM is
  attacker-deployable
- **Provenance check:** a spec is only used if its WASM hash is in the
  transaction's footprint, because contracts can be upgraded after a failure
- Seven explicit outcomes, from `Resolved` to `NotApplicable`; a name is only
  ever reported for `Resolved`
- Each distinct WASM fetched once per analysis; deliberately no persistent cache
- Two real mainnet contracts recorded under `fixtures/contracts/`

**Done when** `sdo explain` names a contract-defined error from a real
fixture: ✅ `Error(Contract, #2)` → `NoHarvestablePails`, offline and live.

Known limitation: Stellar Asset Contract errors are not named. The SAC has no
on-chain spec, and this project only takes names from a spec.

## 🟡 M4 — Failure classification

**Goal:** answer *what evidence explains why* a transaction failed.

Delivered — see [rules.md](docs/architecture/rules.md):

- A three-state rule interface (`Match` / `NoEvidence` / `NotApplicable`). Every
  rule's outcome and reason is kept in the diagnosis, and a verdict separates
  *explained*, *unknown for lack of evidence* and *unsupported*
- Ranked, evidence-citing candidate causes with remediation
- `sdo explain` shows the likely cause, its evidence, the next step, and which
  rules found no evidence
- `survey_failures`, which sampled 18,000 mainnet transactions and found two
  real authorization failures, now committed as fixtures

| Category | Rule | Real fixture | Status |
|---|---|---|---|
| Contract-defined error | `contract_defined_error` | `soroban-trapped-feebump-49ev`, `-49ev-alt` | ✅ validated |
| Footprint entry missing | `footprint_entry_missing` | `soroban-trapped-feebump-24ev` | ✅ validated |
| Invalid authorization entry | `invalid_authorization_entry` | `soroban-auth-signature-expired`, `soroban-auth-nonce-reused` | ✅ validated |
| Archived entry | `archived_entry` | none | 🟡 synthetic only |
| Resource limit exceeded | `resource_limit_exceeded` | none | 🟡 synthetic only |
| Insufficient resource fee | `insufficient_resource_fee` | none | 🟡 synthetic only |
| Missing authorization entry | — | none | ⬜ no rule |

**Done when** each rule has a fixture, fires on it, and stays silent on the
negative control. **Not yet met** for four categories, which is why M4 is
partly complete. The missing fixtures are tracked in
[issue #1](https://github.com/The-Big-Danny/Stellar-Developer-Observatory/issues/1).

> **Policy note.** This roadmap previously said a category without a fixture is
> not implementable. M4 applies a narrower rule. A rule whose *only* evidence is
> a protocol result code defined as that very cause (`EntryArchived`,
> `ResourceLimitExceeded`, `InsufficientRefundableFee`) may ship with synthetic
> tests, marked unvalidated. A rule that interprets diagnostic events still
> requires a real fixture — which is why *missing* authorization has no rule.

## ⬜ M5 — Accuracy & reliability

**Goal:** know how often we are right, and publish it.

- Grow the corpus to ~100 real failed transactions, deduplicated by contract
- Measure top-ranked-cause accuracy against hand-labelled ground truth
- **Publish the number including the misses** — this is itself a differentiator
- Calibrate confidence levels against observed accuracy
- Harden decoding: explicit XDR depth and length limits, fuzzing

## ⬜ M6 — Developer experience

- JSON output and a published `Diagnosis` schema
- Better CLI formatting; colour, quiet and verbose modes
- Published docs and usage examples
- First crates.io release

## ⬜ M7 — Ecosystem integration

Adoption by an existing tool is worth more than any UI we could build.

- WASM build so JavaScript tooling can use the engine
- A GitHub Action asserting failure classes in CI
- Approach the Stellar Lab, Erst, and OpenZeppelin teams about consuming the crate

## ⬜ M8 — Observatory expansion

Only after the analyzer is genuinely good. Possibly a thin web UI, aggregate
failure statistics, or historical analysis — each judged on whether it is
already well served elsewhere. See
[the abandoned-pillars list](docs/research/00-validation.md) before proposing
anything here.

---

## Non-goals

Deliberately out of scope, because each is already well served:

| Not building | Use instead |
|---|---|
| An indexer | [Mercury](https://mercurydata.app/), Stellar CDP |
| Monitoring and alerting | [OpenZeppelin Monitor](https://docs.openzeppelin.com/monitor) |
| Static / security analysis | [Scout](https://github.com/CoinFabrik/scout-soroban) |
| A block explorer or transaction dashboard | [Stellar Lab](https://lab.stellar.org), StellarExpert |
| Local transaction replay | [Erst](https://github.com/dotandev/hintents) |
