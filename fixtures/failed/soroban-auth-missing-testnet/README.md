# Fixture: soroban-auth-missing-testnet

| | |
|---|---|
| Transaction hash | `6c3a1a7e05a3b82de8b0082082ebe50dba742d5829428f8bebb86795b81b62e9` |
| Network | Test SDF Network ; September 2015 (testnet) |
| Ledger | 5004662 |
| Captured | 2026-10-03 |
| RPC provider used | https://soroban-testnet.stellar.org |
| Status | FAILED |
| Soroban | yes (invoke_host_function) |
| Transaction meta version | V4 |
| Fee bumped | no |
| Outer result | `tx_failed` |
| Operation results | `invoke_host_function_trapped` |
| Failed in contract execution | yes |
| Diagnostic events | 24 (all decoded, found at top_level) |
| Fee charged | 8116 stroops |
| Probe verdict | `SUFFICIENT_FOR_ANALYSIS` |

## How the failure was caused

This is a **real failure on Stellar testnet, deliberately caused**, as
[fixtures/README.md](../../README.md) permits. The contract source is in
`fixtures/contract-sources/contract-auth/`.

1. Built the contract (`wasm32v1-none`, release, sha256
   `0560ddba80bb8991125b901dfbcc6cc25a2933b8ab39e1ed2e400bbe9fa92537`). Its
   `guarded(who)` calls `who.require_auth()`.
2. Deployed it to testnet as contract `CCLS3NCTSKVP3S3IPKE55KOHFC4IK757GJDFEDB46O2BPTA7WZWW4GGN`.
3. Invoked `guarded` with a second identity, `GCPT5IKE74AUKY5ZZIZISOG4YJJ6ABYGAU2I5HTUFDJSIDQVQLKD7FXF`,
   as `who`. The CLI signed the required authorization entry with that identity's
   key. That successful call is `7539470334cc5967341fb6b71d4fde436c29a359c4c60c25a2a368113eb61c34`
   (`tx_success`).
4. Took that envelope, emptied its `auth` list, advanced the sequence number,
   and re-signed it with the source account. Submitted it without simulation.
   The network executed it, and it failed for the missing authorization.

The CLI printed a client-side "transaction submission failed" message after
submitting. The ledger is authoritative: the RPC reports this transaction as
`FAILED`, included at ledger 5004662, with fee charged. The second identity's
account exists on testnet (checked via Horizon).

## What the fixture shows

The host raises `Error(Auth, InvalidAction)` for the address
`GCPT5IKE…KD7FXF` with the message `Unauthorized function call for address`,
and escalates it to a VM trap at `require_auth`. The envelope carries **no**
authorization entry for that address. This is the missing-authorization shape.

No prior fixture showed it. The mainnet authorization fixtures are an expired
signature and a reused nonce, which are invalid entries, not missing ones.

It is the reference case for the `missing_authorization_entry` rule, which
classifies it as **confirmed**. Confirmation rests on the structured error and
on verifying from the envelope that no entry carries that address's credentials.

## Why this fixture is useful

Missing authorization was the one planned category with no rule and no real
example. This fixture supplies the real evidence shape: the event's address
argument, and the envelope's empty authorization list.

## Files

- `rpc-response.json` — the `result` member of a `getTransaction` JSON-RPC response, recorded verbatim.
- `probe.json` — what `sdo-probe` observed about this response at capture time.

## Reproducing

The committed `rpc-response.json` is the durable copy. Testnet retention is
finite, so the transaction may later be unfetchable.

```bash
cargo run -p sdo-probe -- capture --rpc https://soroban-testnet.stellar.org --tx 6c3a1a7e05a3b82de8b0082082ebe50dba742d5829428f8bebb86795b81b62e9 --out fixtures/failed/soroban-auth-missing-testnet
```
