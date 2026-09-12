# Contract fixture: `CDL74RF5…`

| | |
|---|---|
| Contract | `CDL74RF5BLYR2YBLCCI7F5FB6TPSCLKEJUBSD2RSVWZ4YHF3VMFAIGWA` |
| Network | Public Global Stellar Network ; September 2015 (mainnet) |
| WASM hash | `db2c14290d4964e3805f2527dd132939ba5fb3fccac56b30bfab8fd091011627` |
| WASM size | 19492 bytes |
| Captured | 2026-09-11, latest ledger 64383309 |
| RPC provider used | https://mainnet.sorobanrpc.com |
| Error enum | `Errors`: 15 cases, `1 = HomesteadExists` … `15 = GapCountTooLow` |

## Role in the corpus

The **inner** contract. In the 49-event fixtures each call to its `harvest` fails with `Error(Contract, #9)` — named `PailMissing` in its spec — and is caught by the caller. It is also the only contract invoked in the 24-event fixture, which fails with a host storage error rather than a contract error, so resolution there is correctly *not applicable*.

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
cargo run -p soroban-failure-rpc --example capture_contract -- CDL74RF5BLYR2YBLCCI7F5FB6TPSCLKEJUBSD2RSVWZ4YHF3VMFAIGWA fixtures/contracts
```

Contract instances and code are persistent ledger entries rather than
transaction history, so unlike transactions they stay fetchable beyond RPC's
7-day window — until archived, or changed by an upgrade.
