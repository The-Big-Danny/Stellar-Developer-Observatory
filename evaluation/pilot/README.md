# M5.2 population pilot data

The recorded output of the population feasibility pilot (#19), which measured
what mainnet population the M5 evaluation can draw from. The findings and every
derived number are in
[`docs/research/m5-population-pilot.md`](../../docs/research/m5-population-pilot.md).

**This is development data.** No transaction named in these files may ever
enter an evaluation dataset (protocol §13.1); that is what
[`../exclusions/pilot-v1.txt`](../exclusions/pilot-v1.txt) enforces, and a test
checks the list against these files.

**These files record observations only.** Each JSONL line carries a
transaction's hashes, its cluster keys, the raw facts the protocol's
eligibility checks read, and the raw terminal host error type. None carries a
cause class, verdict, confidence or rule id — a pilot that recorded SDO's own
opinion could quietly become the thing SDO is measured against.

| File | What it is |
|---|---|
| `census-1.jsonl.gz` | One record per failed Soroban transaction, 1,000 whole ledgers sampled across the retention window of 2026-09-22 |
| `census-1-ledgers.jsonl.gz` | One line per ledger attempted: its sequence, whether it was read completely, how many transactions it held, and the final cursor |
| `census-2.jsonl.gz` | The same, sampling the window about four times as densely; stopped by a time budget after part of its window |
| `census-2-ledgers.jsonl.gz` | Its ledger log |
| `attempt-2026-09-17.jsonl.gz` | An earlier census that a provider outage ended after 314 ledgers. Kept for completeness and for its hashes; **not** used as a population measurement |
| `census-*-report.txt` | The output of `pilot_report` for each census, the figures quoted in the research note |
| `verify-protocol-assumptions.txt` | The live checks of the protocol's RPC assumptions (retention, seed ledger hashes, whole-ledger reads, history archive) |

Every figure can be recomputed offline from these files:

```bash
cargo run --release -p soroban-failure-rpc --example pilot_report -- \
    --jsonl evaluation/pilot/census-1.jsonl.gz \
    --ledger-log evaluation/pilot/census-1-ledgers.jsonl.gz \
    --split 64501205 --trials 200
```
