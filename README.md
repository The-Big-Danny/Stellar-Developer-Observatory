# Stellar Developer Observatory

**An open-source engine that explains why Soroban transactions fail.**

[![CI](https://github.com/The-Big-Danny/Stellar-Developer-Observatory/actions/workflows/ci.yml/badge.svg)](https://github.com/The-Big-Danny/Stellar-Developer-Observatory/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

> ### ⚠️ Status: early development / experimental
>
> Milestones M0–M3 are complete and **M4 is partly complete**. For a failed
> Soroban transaction it tells you **where** it failed, reconstructs the
> contract call trace, names contract-defined errors from the contracts' own
> specs, and **ranks evidence-backed candidate causes** for footprint,
> contract-error and invalid-authorization failures — the categories that
> covered every failure in a one-week sample of 18,000 mainnet transactions.
>
> It says **unknown** when the evidence is not there. Missing authorization has
> no rule yet, and three result-code rules are validated only against synthetic
> data. See [the rules](docs/architecture/rules.md) and
> [ROADMAP.md](ROADMAP.md).
>
> Nothing in this README describes a capability that does not exist.

---

## The problem

When a Soroban transaction fails, the result code is usually
`invoke_host_function_trapped`. That one code covers a missing authorization
entry, an under-declared footprint, an archived ledger entry, an exhausted
resource limit, and a contract's own error — causes that have nothing to do with
each other and completely different fixes.

This is not a novel complaint.
[`stellar-core#3816`](https://github.com/stellar/stellar-core/issues/3816),
opened by Stellar's own team in 2023, asked for errors that name the missing auth
entry, the missing footprint entry, and the depleted resource. It was closed
without a linked implementation.

Our own corpus shows it. Five recorded mainnet failures all report the
identical result, `Trapped`. Their diagnostic events show a storage access
outside the declared footprint, a contract's own `NoHarvestablePails` error, an
expired authorization signature and a reused authorization nonce — unrelated
failures the result code cannot tell apart, and which M4 now separates.

## The approach

Existing Stellar tools — Stellar Lab, StellarExpert, the various explorers —
**render** execution data, and several do it very well. None of them state a
**cause**.

This project is the interpretation layer, and it is a **library first, not a
website**:

```
                    ┌──────────────────────────────┐
   Stellar RPC ───► │  soroban-failure-rpc         │  fetch, decode XDR
                    │  (the only I/O in the repo)  │
                    └─────────────┬────────────────┘
                                  │  typed AnalysisInput
                                  ▼
                    ┌──────────────────────────────┐
                    │ soroban-failure-analysis     │  NO I/O. Pure. Deterministic.
                    │   transaction model  (M2) ✓  │
                    │   error resolver     (M3) ✓  │
                    │   rule engine        (M4) ◐  │
                    │   ranker                     │
                    └─────────────┬────────────────┘
                                  │  Diagnosis
                    ┌─────────────┴────────────────┐
                    ▼                              ▼
                 sdo CLI                    JSON / other tools
```

A website would compete with Stellar Lab. A crate is something Lab — or any
explorer, or a CI pipeline — could **consume**. Being the dependency beats being
the alternative.

### Design commitments

- **Ranked candidates, never a single verdict.** Diagnostic events are unmetered
  and are *not part of consensus*. Claiming certainty we do not have would
  destroy trust the first time it is wrong.
- **Every claim cites evidence.** A cause points at the exact diagnostic event,
  auth entry, or footprint entry that supports it.
- **Absence of evidence is not evidence.** The engine distinguishes "this node
  emits no diagnostic events" from "this transaction had none", and rules may not
  conclude anything from silence.
- **The core crate performs no I/O**, so the whole rule corpus is testable from
  committed fixtures with no network.

## What this is *not*

- ❌ Not a block explorer
- ❌ Not an indexer — use [Mercury](https://mercurydata.app/) or Stellar's CDP
- ❌ Not a replacement for [Stellar Lab](https://lab.stellar.org), which already
  renders transactions well and which we would rather be used *by*
- ❌ Not an AI chatbot — this is a deterministic rule engine
- ❌ Not a monitoring or alerting platform — see
  [OpenZeppelin Monitor](https://docs.openzeppelin.com/monitor)

## Repository layout

| Path | What it is |
|---|---|
| [`crates/soroban-failure-analysis`](crates/soroban-failure-analysis) | The pure analysis engine. No I/O, ever. |
| [`crates/soroban-failure-rpc`](crates/soroban-failure-rpc) | RPC access, XDR decoding, fixture loading |
| [`crates/sdo`](crates/sdo) | The `sdo` command line interface |
| [`tools/rpc-probe`](tools/rpc-probe) | `sdo-probe`, the M0 research instrument |
| [`fixtures/`](fixtures) | Real recorded mainnet responses for offline tests |
| [`docs/research/`](docs/research) | Validation and measurement write-ups |

## Try it

Requires Rust 1.88 or newer.

```bash
git clone https://github.com/The-Big-Danny/Stellar-Developer-Observatory
cd Stellar-Developer-Observatory
cargo test --workspace
```

Analyse a committed fixture — no network needed:

```bash
cargo run -p sdo -- explain \
    --fixture fixtures/failed/soroban-trapped-feebump-49ev \
    --contracts fixtures/contracts
```

Abridged output (`…` marks elisions):

```
Transaction     6d597ca6a4d168770b91d89769b8343a6d5ef11d78201c606ffd7caecc3058bc
Fee-bumped      yes — fee source GA2JRQOF…, outer result TxFeeBumpInnerFailed
Result          TxFailed — operation 0: InvokeHostFunction Trapped
Failure stage   contract_execution — Soroban contract execution trapped
Likely cause    contract_defined_error (confirmed)
Diagnostic      49 events (30 execution, 19 core_metrics)

Call trace (from diagnostic events)
  CBGSBK…KKY3::harvest                     failed Error(Contract, #2)
    CDL74R…IGWA::harvest                   failed Error(Contract, #9)
    … four more identical inner calls …

Contract errors
  Error(Contract, #2)  →  NoHarvestablePails   [terminal]
  Error(Contract, #9)  →  PailMissing   [raised, then caught or superseded]

Diagnosis
  verdict: explained — the top cause is confirmed

  1. contract_defined_error [confirmed]
     Contract CBGSBKY…KKY3 ended the invocation with its declared error Error::NoHarvestablePails (Error(Contract, #2)).
     evidence:
       - `host_fn_failed` reports the invocation ended with Error(Contract, #2) (event 29)
       - raised by contract CBGSBKY…: the only contract whose error events carry this code (event 26)
       - the spec of CBGSBKY… declares #2 as Error::NoHarvestablePails; the spec's WASM 70fe4469… is loaded in this transaction's footprint (contract spec)
       - documented as: "Harvesting all pails results in 0 reward" (contract spec)
       - earlier, 5 call(s) by CDL74RF5… failed with Error(Contract, #9) (PailMissing) without ending the invocation (event 2)
     next step: The contract documents NoHarvestablePails as "Harvesting all pails results in 0 reward". Check whether that condition held for this invocation's arguments and the contract's state at the time.
```

Every name there comes from the contract's own spec, checked against the WASM
the transaction actually loaded. Every claim cites the event, auth entry or
footprint it rests on. When the evidence runs out, the output says `unknown`
and lists what each rule was missing; it never guesses.

What the committed fixtures produce:

| Fixture | Likely cause |
|---|---|
| `soroban-trapped-feebump-24ev` | `footprint_entry_missing` (confirmed) — key `vec[Block, 183188]` checked absent from the footprint |
| `soroban-trapped-feebump-49ev` | `contract_defined_error` (confirmed) — `NoHarvestablePails` |
| `soroban-auth-signature-expired` | `invalid_authorization_entry` (confirmed) — expiry corroborated by the envelope |
| `soroban-auth-nonce-reused` | `invalid_authorization_entry` (likely) — nonce reuse cannot be proven from the transaction |
| `classic-failed-no-diagnostics` | unsupported — not a Soroban failure |

Against the live network, `sdo explain <TX_HASH>` does the same, fetching only
the contract specs the transaction's errors need.

Measure an RPC endpoint yourself:

```bash
cargo run -p sdo-probe -- scan --want 5
cargo run -p sdo-probe -- probe --tx <TRANSACTION_HASH>
```

## Research

M0 settled the question that could have killed the project — whether public
mainnet RPC actually returns diagnostic events. It does: 4 of 4 responding
providers, 13 of 13 transactions, 437 of 437 events decoded.

- [Pre-build validation](docs/research/00-validation.md) — competitor analysis
  and why this is library-first
- [M0 feasibility report](docs/research/m0-report.md) — method, results, and the
  **GO** decision
- [RPC diagnostic-event availability](docs/research/rpc-diagnostic-events.md) —
  per-provider measurements
- [Failure taxonomy](docs/research/failure-taxonomy.md) — including which
  failure categories the corpus still has no fixture for
- [The canonical transaction model](docs/architecture/transaction-model.md) and
  [contract error resolution](docs/architecture/contract-errors.md)
- [Failure classification rules](docs/architecture/rules.md) — what each rule
  requires, its confidence levels, and what is not classified yet

## Contributing

Contributions are genuinely wanted, and the architecture exists to make them
possible: a new failure mode should be **one rule file, one fixture, one test**.

Start with [CONTRIBUTING.md](CONTRIBUTING.md) and the
[`good first issue`](https://github.com/The-Big-Danny/Stellar-Developer-Observatory/labels/good%20first%20issue)
label.

## License

[MIT](LICENSE).
