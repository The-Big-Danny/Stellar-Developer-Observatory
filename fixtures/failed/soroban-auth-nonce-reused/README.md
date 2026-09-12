# Fixture: soroban-auth-nonce-reused

| | |
|---|---|
| Transaction hash | `cb94259e03b10cede018ee24ef43b42d73e8461c4cfe4cc9750fe58f05fb53f5` |
| Network | Public Global Stellar Network ; September 2015 |
| Ledger | 64324119 |
| Captured | 2026-09-12 |
| RPC provider used | https://rpc.lightsail.network/ |
| Status | FAILED |
| Soroban | yes (invoke_host_function) |
| Transaction meta version | V4 |
| Fee bumped | no |
| Diagnostic events | 24 (all decoded, found at top_level) |
| Probe verdict | `SUFFICIENT_FOR_ANALYSIS` |

## Failure category

`InvalidAuthorizationEntry` — reused nonce. Terminal `Error(Auth, ExistingValue)` with host message *"nonce already exists for address"*; the event names the address, which matches authorization entry 0.

See [docs/architecture/rules.md](../../../docs/architecture/rules.md).

## Why this fixture is useful

The corpus's first real Soroban failure that is **not fee-bumped**, so it guards against logic that only works on unwrapped fee bumps. It exercises the `invalid_authorization_entry` rule's *Likely* path: whether a nonce was previously consumed is ledger state that the transaction data cannot independently prove, so the rule deliberately does not report it as confirmed. It was found by `survey_failures`, which saw 3 transactions of this shape in 18,000 sampled.

## Fields available in the recorded response

`applicationOrder, createdAt, diagnosticEventsXdr, envelopeXdr, events, feeBump, latestLedger, latestLedgerCloseTime, ledger, oldestLedger, oldestLedgerCloseTime, resultMetaXdr, resultXdr, status, txHash`

## Files

- `rpc-response.json` — the `result` member of a `getTransaction` JSON-RPC response, recorded verbatim.
- `probe.json` — what `sdo-probe` observed about this response at capture time.

## Reproducing

```bash
cargo run -p sdo-probe -- capture --rpc https://rpc.lightsail.network/ --tx cb94259e03b10cede018ee24ef43b42d73e8461c4cfe4cc9750fe58f05fb53f5 --out fixtures/failed/soroban-auth-nonce-reused
```

> Mainnet RPC retains roughly 7 days of history, so this transaction will not
> stay fetchable from a standard endpoint. That is why the response is committed.
