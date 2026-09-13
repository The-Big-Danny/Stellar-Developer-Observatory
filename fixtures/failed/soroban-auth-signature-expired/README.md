# Fixture: soroban-auth-signature-expired

| | |
|---|---|
| Transaction hash | `1a24d6811454317f3337c0f66463d2b2df5b69632cdd0c0f8bdf728e3a0d81f1` |
| Network | Public Global Stellar Network ; September 2015 |
| Ledger | 64392368 |
| Captured | 2026-09-12 |
| RPC provider used | https://rpc.lightsail.network/ |
| Status | FAILED |
| Soroban | yes (invoke_host_function) |
| Transaction meta version | V4 |
| Fee bumped | yes |
| Diagnostic events | 26 (all decoded, found at top_level) |
| Probe verdict | `SUFFICIENT_FOR_ANALYSIS` |

## Failure category

`InvalidAuthorizationEntry` — expired signature. Terminal `Error(Auth, InvalidInput)` with host message *"signature has expired"*. The event data carries the ledger the host checked at (64392368) and the signature expiration ledger (64392366); the envelope's own auth entry for the same address declares `signature_expiration_ledger` 64392366.

See [docs/architecture/rules.md](../../../docs/architecture/rules.md).

## Why this fixture is useful

The corpus's first real authorization failure, and its first transaction with *address* credentials. The authorizing address is a contract account: the diagnostic events show its `__check_auth` being called and returning normally before the expiry check fails, so the signature itself verified and only its validity window was wrong. Because the envelope independently carries the same expiration ledger the host checked, this is the fixture that proves the `invalid_authorization_entry` rule's *Confirmed* path. It was found by `survey_failures`, which saw 4 transactions of this shape in 18,000 sampled.

## Fields available in the recorded response

`applicationOrder, createdAt, diagnosticEventsXdr, envelopeXdr, events, feeBump, latestLedger, latestLedgerCloseTime, ledger, oldestLedger, oldestLedgerCloseTime, resultMetaXdr, resultXdr, status, txHash`

## Files

- `rpc-response.json` — the `result` member of a `getTransaction` JSON-RPC response, recorded verbatim.
- `probe.json` — what `sdo-probe` observed about this response at capture time.

## Reproducing

```bash
cargo run -p sdo-probe -- capture --rpc https://rpc.lightsail.network/ --tx 1a24d6811454317f3337c0f66463d2b2df5b69632cdd0c0f8bdf728e3a0d81f1 --out fixtures/failed/soroban-auth-signature-expired
```

> Mainnet RPC retains roughly 7 days of history, so this transaction will not
> stay fetchable from a standard endpoint. That is why the response is committed.
