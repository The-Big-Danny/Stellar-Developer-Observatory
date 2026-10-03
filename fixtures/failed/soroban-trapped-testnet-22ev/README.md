# Fixture: soroban-trapped-testnet-22ev

| | |
|---|---|
| Transaction hash | `1f020cba1c06bc09422c9223546e5da5c2327b6bf380eac50fb43e7f57eaa8ad` |
| Network | Test SDF Network ; September 2015 (testnet) |
| Ledger | 5004398 |
| Captured | 2026-10-03 |
| RPC provider used | https://soroban-testnet.stellar.org |
| Status | FAILED |
| Soroban | yes (invoke_host_function) |
| Transaction meta version | V4 |
| Fee bumped | no |
| Outer result | `tx_failed` |
| Operation results | `invoke_host_function_trapped` |
| Failed in contract execution | yes |
| Diagnostic events | 22 (all decoded, found at top_level) |
| Fee charged | 2929 stroops |
| Probe verdict | `SUFFICIENT_FOR_ANALYSIS` |

## How the failure was caused

This is a **real failure on Stellar testnet, deliberately caused**, as
[fixtures/README.md](../../README.md) permits for a failure mode that does not
occur naturally. The contract source is in
`fixtures/contract-sources/contract-trap/`.

1. Built the contract to wasm (`wasm32v1-none`, release, sha256
   `550e13906441ef91aadf8abff5819d3a296e717c0cde49de07b6233e446b230b`).
   Its `trigger()` calls `unwrap()` on `None`.
2. Deployed it to testnet as contract `CC5JULCIO5LSKKGECT5TK2TXNZHWH5ZNMKIAXARABVJUA7BV22XLOTK7`.
3. Simulation refuses to send `trigger()`, so it can never reach the ledger
   through the normal path. To get a failure **on-chain**, the envelope was
   built from a successful `ok()` call on the same contract (which takes the
   same footprint and fee, because `trigger()` traps before any ledger access).
   Its function name was changed to `trigger`, the sequence number was advanced
   by one, the signature was replaced, and the envelope was submitted directly.
   The network executed it and recorded the trap; the transaction was not
   simulated before submission.

The successful `ok()` transaction is `7e7014b4cd0e60f94389342cccb2b5ae3587de300b5281e6227b0015885da3ae`.
An earlier submission of the same trigger envelope was rejected with
`TxBadAuthExtra` (a duplicated signature) and was never included in a ledger.

## What the fixture shows

The host reports the invocation ended with `Error(WasmVm, InvalidAction)`, with
the message `VM call trapped: UnreachableCodeReached`. That is a WebAssembly
`unreachable` instruction, which a Rust panic produces. It is **not** a
contract-declared `#[contracterror]` code, so `contract_defined_error` does not
apply. No footprint violation occurred, so `footprint_entry_missing` does not
apply either.

It is the reference case for the `contract_trap` rule, which classifies it as
**likely** `ContractTrap`. The rule never reaches `confirmed`, because the host
message does not identify the panic site.

## Why this fixture is useful

The corpus had no fixture whose failure is a plain panic. The three mainnet
fixtures labelled `ContractTrap` are not panics: `soroban-trapped-feebump-49ev`
and `-49ev-alt` end in a contract-declared `Error(Contract, #N)`, and
`soroban-trapped-feebump-24ev` ends in a footprint violation. This is the only
fixture that ends in a WebAssembly trap with no declared error.

## Files

- `rpc-response.json` — the `result` member of a `getTransaction` JSON-RPC response, recorded verbatim.
- `probe.json` — what `sdo-probe` observed about this response at capture time.

## Reproducing

The transaction is on testnet for roughly the testnet retention window. The
committed `rpc-response.json` is the durable copy.

```bash
cargo run -p sdo-probe -- capture --rpc https://soroban-testnet.stellar.org --tx 1f020cba1c06bc09422c9223546e5da5c2327b6bf380eac50fb43e7f57eaa8ad --out fixtures/failed/soroban-trapped-testnet-22ev
```
