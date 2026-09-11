# The purity rule

> `soroban-failure-analysis` performs no I/O. Ever.

No network. No filesystem. No clock. No environment variables. No randomness. No
global mutable state.

This document exists because the rule looks like fussiness until you see what it
buys, and because `crates/soroban-failure-analysis/Cargo.toml` points here.

## Why

**Because it is what makes the project contributable.**

A rule is a claim about why a transaction failed. To trust a rule you must be
able to run it against a known transaction and check the answer. If the engine
fetched its own data, then testing a rule would require:

- a working RPC endpoint,
- that endpoint still retaining the transaction (mainnet keeps ~7 days),
- the network being up, and
- rate limits not being hit.

A contributor would have to own infrastructure to fix a typo in a remediation
message, and CI would be flaky through no fault of the code.

With decoding separated out, a fixture is a file, a rule is a function, and a
test is `assert_eq!`. Everything runs offline in milliseconds. **That is the
entire reason the corpus in `fixtures/` exists.**

There are three further benefits:

1. **Determinism.** The same `AnalysisInput` always produces the same
   `Diagnosis`. A test asserts this. Without it, "why did this report change?"
   becomes unanswerable.
2. **Embeddability.** The strategic goal is for other tools — a CI action, an
   explorer, possibly Stellar Lab — to consume this crate. A library that opens
   sockets is far harder to adopt than one that transforms data.
3. **Auditability.** A rule that cannot reach the network cannot leak the
   transaction you are debugging.

## Where the boundary sits

| Crate | I/O | Notes |
|---|---|---|
| `soroban-failure-analysis` | **none** | Enforced by having no I/O-capable dependency |
| `soroban-failure-rpc::decode` | none | Pure JSON → typed input |
| `soroban-failure-rpc::fixture` | filesystem read | Loading recorded responses |
| `soroban-failure-rpc::client` | **network** | The only networking in the product crates |
| `sdo` | via the above | Presentation |
| `tools/rpc-probe` | network | A research instrument, not part of the product (`publish = false`) |

The split inside `soroban-failure-rpc` is deliberate: `decode` is pure so that
fixtures and live responses travel the identical code path. A fixture test
therefore exercises real decoding, not a mock.

## How it is enforced

Today, by dependency discipline and review. `soroban-failure-analysis` depends
only on `stellar-xdr` and optionally `serde` — neither can perform I/O — and the
manifest carries a comment saying so.

This is honest but not airtight: a future dependency could smuggle I/O in. A CI
check that inspects the resolved dependency tree for the core crate is tracked
in [issue #3](https://github.com/The-Big-Danny/Stellar-Developer-Observatory/issues/3).

## If you need data the engine does not have

The temptation is to fetch it inside a rule. Don't.

Instead, add it to `AnalysisInput` and have `soroban-failure-rpc` populate it.
`AnalysisInput` is `#[non_exhaustive]` with a builder precisely so that new
inputs can be added without breaking downstream code.

This is exactly how M3 works. The engine reports which contracts it needs
(`contracts_needing_specs`), `soroban-failure-rpc` fetches their specs via
`getLedgerEntries`, and they come back in through
`AnalysisInput::contract_specs`. The engine resolves `Error(Contract, #2)` to
`NoHarvestablePails` without ever knowing an RPC endpoint exists — which is also
why the same resolution runs offline from `fixtures/contracts/`. See
[contract-errors.md](contract-errors.md).
