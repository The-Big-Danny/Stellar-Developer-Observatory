# M0 — Feasibility report

**Date:** 2026-09-10
**Decision:** ✅ **GO**

---

## Objective

[`00-validation.md`](00-validation.md) identified one risk capable of killing the
project outright:

> The entire product assumes diagnostic events are retrievable for arbitrary
> mainnet transactions. Diagnostic events are only returned if the node was
> started with `--enable-soroban-diagnostic-events` [...] Stellar's own guidance
> is to enable it only on *watcher* nodes.

Compounded by a second constraint: for a **failed** transaction,
`sorobanMeta.events` is not populated. Diagnostic events are therefore the only
execution-flow evidence available for exactly the transactions this project
analyses.

M0 asked one question: **do real public mainnet RPC providers actually return
decodable diagnostic events for real failed Soroban transactions?**

The instruction for M0 was explicit — do not manufacture a positive result.

## Method

A purpose-built probe, [`tools/rpc-probe`](../../tools/rpc-probe), was written
first. It uses only documented RPC methods and makes no assumptions about
response shape.

| Subcommand | Purpose |
|---|---|
| `scan` | Walk ledgers via `getTransactions`, decode each envelope, and select transactions that both failed *and* carry a Soroban operation |
| `probe` | Call `getTransaction` and report artifact-by-artifact what is present, what decodes, and what is absent |
| `capture` | Record a response verbatim to disk as an offline fixture |

Design choices that matter for trusting the result:

- **Absent, undecodable, and decoded are three distinct states.** A field present
  but unreadable is a far worse finding than a field simply missing, so they are
  never collapsed.
- **Diagnostic events are searched in all three known locations** — top-level
  `diagnosticEventsXdr`, `events.diagnosticEventsXdr`, and embedded in
  `resultMetaXdr` — rather than assuming one shape.
- **The probe's exit code carries the verdict**, so it is usable from scripts.
- **Every event is individually decoded.** "Present" is not accepted as
  "usable"; the report states how many of N decoded.

There is no code path in the probe that reports availability without having
decoded something.

## Providers tested

Every free public mainnet endpoint in the [Stellar RPC provider
list](https://developers.stellar.org/docs/data/apis/rpc/providers). Liquify was
excluded because the published URL embeds a third party's API key.

Full results: [`rpc-diagnostic-events.md`](rpc-diagnostic-events.md).

| | Endpoints |
|---|---|
| Answered with diagnostic events | **4** — sorobanrpc.com, Gateway, Lightsail Quasar, Ankr |
| Answered without diagnostic events | **0** |
| Did not answer usefully | 3 — Nodies (HTTP 500), OnFinality (HTTP 429), Lightsail archive (`NOT_FOUND`) |

## Transactions tested

14 real mainnet transactions, found by scanning rather than hand-picked:

- **13 failed Soroban transactions** across three separated ledger regions:
  8 at ledger 64365919–20, 2 at ledger 64308924, 3 at ledger 64358920–21.
- **1 failed classic transaction**, retained as a negative control.

Of the 13 Soroban failures, **10 were additionally re-probed** after the
fee-bump unwrapping fix landed, and their operation-level result codes
confirmed. The other 3 were confirmed for artifact availability only.

No transaction was synthesised. No result was hand-edited.

## Results

### Diagnostic-event availability

| Measure | Result |
|---|---|
| Soroban failures probed | 13 |
| Returned diagnostic events | **13 / 13 (100%)** |
| Every event decoded against `stellar-xdr 28.0.0` | **13 / 13 (100%)** |
| Total events decoded | 437 / 437 |
| Cross-provider agreement on count | 4 / 4 endpoints agreed exactly |

Event counts were 24 or 49 depending on ledger region — i.e. genuinely varying
with the transaction, not a constant artifact.

### Decoding

Everything decoded. Envelope, result, and metadata all decoded cleanly on every
sample; all 437 diagnostic events decoded individually with zero errors.

Two structural facts were discovered in the process, both of which would have
caused silent failures later:

**1. Current mainnet returns `TransactionMetaV4` (protocol 23), and diagnostic
events moved.** In V3 they live inside `sorobanMeta`; in V4 they were lifted to
the top level of the meta, and `SorobanTransactionMetaV2` no longer carries
events at all. An analyzer written against the widely-documented V3 shape finds
nothing on today's mainnet.

**2. Failed Soroban transactions on mainnet are overwhelmingly fee-bumped.** All
10 re-probed samples were. The outer result is `TxFeeBumpInnerFailed`, which carries no
cause information; the real result sits in `InnerTransactionResultPair`. Reading
only the outer union reports nothing useful. Before this was handled the probe
reported `tx_other` for every sample — visibly wrong, which is how it was caught.

A third, smaller finding: **`stellar-xdr` 28.0.0 removed the `curr` module**.
Every existing tutorial uses `stellar_xdr::curr::*`; that no longer compiles.

### Failure taxonomy

See [`failure-taxonomy.md`](failure-taxonomy.md). Summary of what was *observed*:

| Category | Count |
|---|---|
| `ContractTrap` (`invoke_host_function_trapped`) | 10 confirmed (of 13 Soroban failures) |
| Classic / non-Soroban | 1 |
| Auth, Footprint, Archival, ResourceLimit, ResourceFee | **0** |

> **Correction, 2026-09-11 (M2).** This table classifies by result code only.
> Once M2 decoded the diagnostic events, the three Soroban fixtures proved to be
> one footprint-shaped storage failure and two contract-defined errors, which
> means real evidence for two categories rather than zero. The table above is left
> as the original M0 record; see the
> [taxonomy correction](failure-taxonomy.md#correction-after-m2-2026-09-11).

## Limitations

What this exercise did **not** establish:

1. **The sample is badly biased.** Every Soroban failure whose result code was read is
   `invoke_host_function_trapped`, clustered in adjacent ledgers, almost
   certainly the same arbitrage bots failing repeatedly. This is the failure
   population of *mainnet bots*, not of *developers*.
2. **Five of the six planned V1 rule categories have no fixture.** They were not
   observed at all.
3. **Provider comparison used a single transaction.**
4. **Three providers remain untested**, including an archive endpoint that
   unexpectedly returned `NOT_FOUND` for a transaction only ~3000 ledgers old.
5. **Paid tiers and testnet were not measured.**
6. **Point-in-time.** Providers may reconfigure at any moment.
7. **Diagnostic events are not part of consensus.** Cross-provider agreement was
   observed on one transaction; it is not guaranteed in general.

## Conclusion

### ✅ GO

The gating risk did not materialise. Diagnostic events are available and fully
decodable from every public mainnet endpoint that responded, and the fallback
plan of self-hosting a watcher node is not required. The library-first
architecture in `00-validation.md` stands unchanged.

The `Trapped` opacity documented in `stellar-core#3816` was also directly
confirmed: ten transactions, one result code, no way to distinguish them without
reading diagnostic events. That is the problem this project exists to solve, and
it is still real.

**This is a GO on feasibility, not on readiness.** The corpus is not yet good
enough to write rules against, and that is now the critical path.

## Recommendation

**M1 — Foundation.** Proceed as planned. Two changes forced by M0 findings:

- Decoding must handle `TransactionMeta` V3 *and* V4, and must unwrap fee-bumped
  results. Both are implemented and tested.
- The core crate must distinguish "node emits no diagnostics" from "this
  transaction had none". `AnalysisInput::diagnostics_enabled` does this, so rules
  cannot conclude anything from silence.

**M2 — Transaction & XDR engine.** Implement `FailureStage` classification from
result codes. The mapping is now known and grounded in real data. Unwrapping fee
bumps is a hard requirement, not an edge case.

**Before M4 — fix the corpus. This is now the project's biggest risk.**
Feasibility is settled; fixture *diversity* is not. Rules cannot be written
against a corpus containing one category. Deliberately produce each failure mode
on testnet and capture it. Treat any category without a fixture as not
implementable.
