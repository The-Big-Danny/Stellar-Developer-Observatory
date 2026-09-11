# Contract fixtures

Recorded contract instances and WASM, so contract error codes can be named from
real specs **offline** (milestone M3).

```
fixtures/contracts/
└── <CONTRACT_ID>/
    ├── instance.json   getLedgerEntries result for the instance key, verbatim
    ├── code.json       getLedgerEntries result for the WASM code key, verbatim
    └── README.md       provenance
```

Read back by `soroban_failure_rpc::FixtureContractSource`, which decodes them
with the same functions used for live responses.

The rules from [`fixtures/README.md`](../README.md) apply: real, verbatim,
permanent, and no secrets.

## Current contracts

| Contract | Error enum | Used by |
|---|---|---|
| [`CBGSBKY…`](CBGSBKYMYO6OMGHQXXNOBRGVUDFUDVC2XLC3SXON5R2SNXILR7XCKKY3/) | `Error` (2 cases) | `soroban-trapped-feebump-49ev`, `-49ev-alt` |
| [`CDL74RF5…`](CDL74RF5BLYR2YBLCCI7F5FB6TPSCLKEJUBSD2RSVWZ4YHF3VMFAIGWA/) | `Errors` (15 cases) | all three Soroban transaction fixtures |

## Using them

```bash
cargo run -p sdo -- explain     --fixture fixtures/failed/soroban-trapped-feebump-49ev     --contracts fixtures/contracts
```

## Capturing one

```bash
cargo run -p soroban-failure-rpc --example capture_contract -- <CONTRACT_ID> fixtures/contracts
```

Capture only contracts a transaction fixture actually invokes, and confirm the
recorded WASM hash appears in that transaction's footprint — otherwise the spec
may describe a different version of the contract than the one that ran.
