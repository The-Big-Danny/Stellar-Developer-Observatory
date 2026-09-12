# Contract fixture: `CBGSBKYM…`

| | |
|---|---|
| Contract | `CBGSBKYMYO6OMGHQXXNOBRGVUDFUDVC2XLC3SXON5R2SNXILR7XCKKY3` |
| Network | Public Global Stellar Network ; September 2015 (mainnet) |
| WASM hash | `70fe44694c9fe6b0abc69a6da4858fc2aaba04fa10492a466a1d426d04ca8560` |
| WASM size | 1713 bytes |
| Captured | 2026-09-11, latest ledger 64383308 |
| RPC provider used | https://mainnet.sorobanrpc.com |
| Error enum | `Error`: `1 = NoPailsProvided`, `2 = NoHarvestablePails` |

## Role in the corpus

The **outer** contract in both 49-event fixtures. Its `harvest` function calls the farm contract repeatedly via `try_call`, then fails with `Error(Contract, #2)` — the terminal error — which its spec names `NoHarvestablePails`.

## Provenance check

This spec was captured *after* the transactions that use it. Contracts can be
upgraded, so that alone would not prove it describes the code that ran. It is
proved instead by the transactions' own footprints: this WASM hash appears as a
`ContractCode` key in the footprint of every fixture that invokes the
contract. `crates/sdo/tests/contract_resolution.rs` asserts this, so the
fixture can never silently drift out of step.

## Files

- `instance.json` — `getLedgerEntries` result for the contract's instance key, verbatim.
- `code.json` — `getLedgerEntries` result for the WASM code key, verbatim.

Both are public ledger state, recorded the same way as the transaction fixtures.

## Reproducing

```bash
cargo run -p soroban-failure-rpc --example capture_contract -- CBGSBKYMYO6OMGHQXXNOBRGVUDFUDVC2XLC3SXON5R2SNXILR7XCKKY3 fixtures/contracts
```

Contract instances and code are persistent ledger entries rather than
transaction history, so unlike transactions they stay fetchable beyond RPC's
7-day window — until archived, or changed by an upgrade.
