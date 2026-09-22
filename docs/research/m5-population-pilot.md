# M5.2 — population feasibility pilot

**Issue:** #19 · **Run on:** 2026-09-22 · **Network:** Stellar mainnet
(`Public Global Stellar Network ; September 2015`)

This note measures the population the M5 evaluation can actually draw from, and
fixes the open parameters of
[the evaluation protocol](../evaluation/protocol.md). It contains **no accuracy
figure**, and says nothing about whether SDO is right about anything: the
records it is built from carry no cause class, verdict, confidence or rule id.

---

## 1. What this found

1. **The protocol's RPC assumptions hold.** 40 of 40 live checks passed, twice,
   against two providers and an independent history archive (§3).
2. **The failing-contract population is small.** One full retention window
   (about 7 days) contained **28 distinct eligible `code_cluster`s**, with a
   lower bound of about 32 for the whole window. The ten largest clusters
   account for 95% of eligible transactions, and one WASM hash alone for 57%.
3. **One round cannot yield 100 samples.** Under the protocol's caps (one
   sample per `code_cluster`, three per `submitter_cluster`), a round sampling
   1,000 ledgers of a window yields about **27** samples. It is limited by how
   many distinct contracts fail, not by how many transactions fail.
4. **Later rounds add little.** Two consecutive half-windows, each about 3.5
   days, shared 20 of their 24 code clusters: the second contributed **4** new
   ones.
5. **Therefore `T = 100` is not reachable** in any round count this project can
   wait for. The parameters are set to collect what the population supports;
   the shortfall is pre-registered here, before any collection, and will be
   published with the run.

Nothing was padded, substituted or hand-picked to make a number look better,
and no cap was relaxed to raise the yield.

## 2. How the census was run

`survey_failures --ledgers` reads **every** transaction of each sampled ledger,
following the `getTransactions` cursor to the end of the ledger, and writes one
JSONL record per failed Soroban transaction: its hashes, its cluster keys, the
raw facts the protocol's eligibility checks read, and the raw terminal host
error type. A ledger that cannot be read completely is never used in part.

Ledgers are chosen by stratified sampling from a fixed window — one ledger from
each of `N` equal strata, at a deterministic offset seeded by `--seed`. This is
deliberately *not* the protocol's SHA-256 ledger selection (§6.3): the pilot
measures a population, it selects no evaluation sample.

| | Census 1 | Census 2 |
|---|---|---|
| Window | 64,441,525 – 64,560,884 | 64,443,170 – 64,561,029 |
| Ledgers requested | 1,000 | 4,000 (stopped after 1,676, §7) |
| Seed | `m5.2-pilot-2026-09-22` | `m5.2-pilot-2026-09-22-b` |
| Collection provider | `rpc.lightsail.network` | started on `mainnet.sorobanrpc.com`, resumed on `rpc.lightsail.network` |
| Fallback provider | `mainnet.sorobanrpc.com` | the other of the two |

Both windows are one full RPC retention window (120,960 ledgers, about 7.1
days) less a margin at each end, since the window slides while a census runs.

```bash
cargo run --release -p soroban-failure-rpc --example survey_failures -- \
    --ledgers 1000 --window 64441525..64560884 --seed m5.2-pilot-2026-09-22 \
    --rpc https://rpc.lightsail.network --fallback https://mainnet.sorobanrpc.com \
    --jsonl pilot.jsonl --ledger-log ledgers.jsonl
```

### Interruptions, and what was done about them

- An **earlier attempt on 2026-09-17** read 314 ledgers of the then-current
  window and stopped when the provider began closing connections
  (`unexpected end of file`) on every request. Those 102 ledgers are recorded
  as unreadable. That attempt is **not** used as a population measurement: it
  is incomplete, and its window has since left retention. Its records are kept
  (`evaluation/pilot/attempt-2026-09-17.jsonl.gz`), its hashes are on the
  exclusion list, and it is used only in §6 as an earlier, disjoint window for
  comparison.
- Its failure is the reason `survey_failures` now implements the protocol's own
  retry procedure (§7.1: four attempts, waiting 1, 2 and 4 seconds, then one
  attempt on the fallback provider), records every ledger attempt in a log, and
  can resume. A dropped connection is a fact about a provider, not about the
  population, and is never allowed to look like one.
- **Census 2 began on `mainnet.sorobanrpc.com`** and read 245 ledgers in 25
  minutes; the same work runs at about three times that rate on
  `rpc.lightsail.network`, so it was stopped and resumed there from its ledger
  log. Both providers answered correctly; they differ in throughput, not in
  content (§3, check 4).

## 3. The protocol's RPC assumptions, checked live

`verify_protocol_assumptions` checks each assumption the protocol rests on and
exits non-zero if any fails. Full output:
[`evaluation/pilot/verify-protocol-assumptions.txt`](../../evaluation/pilot/verify-protocol-assumptions.txt).

| # | Assumption | Result |
|---|---|---|
| 1 | Both providers retain at least 120,000 ledgers (§6.1) | **PASS** — both report 120,960 |
| 2 | `getLedgers` `hash` is the real ledger header hash (§6.2) | **PASS** — it equals the hash in the ledger's own decoded `headerXdr`, and the next ledger's signed header names it as `previousLedgerHash` |
| 3 | Two providers report the same hash (§6.2) | **PASS** |
| 4 | Following the cursor reads a whole ledger (§6.3) | **PASS** on 6 ledgers: both providers return the same set; the set equals the transaction hashes in that ledger's own `LedgerCloseMeta`; and reading in pages of 7 gives the same set as pages of 200 |
| 5 | A history archive publishes the same hash (§6.2) | **PASS** against `history.stellar.org/prd/core-live/core_live_001`, which stellar-core writes independently of any RPC provider |

Check 4 is what the earlier review asked for: completeness is now verified
against the ledger itself, not merely against a second provider, and the
page-of-7 read forces the cursor across page boundaries *inside* a ledger.
Check 5 settles the open question of whether the protocol's second seed source
can be a history archive: it can, and the archive is genuinely independent.

The seed-ledger rule itself (§6.2) is therefore sound: the `hash` of a ledger
that has not yet closed cannot be known or chosen, and once it closes, two
independent sources can confirm it.

## 4. The eligibility funnel

Census 1, over 1,000 ledgers of one window:

| Stage | Count |
|---|---|
| Transactions read | 266,095 |
| `status = FAILED` | 54,067 |
| Failed and not Soroban (check 2) | 47,846 |
| **Failed Soroban** | **6,221** |
| Check 1, `decode_failed` | 0 |
| Check 3, `unexpected_operation_count` | 0 |
| Check 4, `no_diagnostic_events` | 0 |
| Check 6, `no_root_contract_invocation` | 0 |
| Check 7, `code_identity_*` | 0 |
| **Eligible** (before check 5) | **6,221** |
| Check 5, development data | 0 |

Three of these deserve comment, because zero is a strong claim:

- **Every** failed Soroban transaction decoded, had exactly one operation, had
  at least one diagnostic event, and was an `InvokeContract` call. No failed
  `ExtendFootprintTtl` or `RestoreFootprint` transaction occurred in 266,095
  transactions.
- **Every** root contract's instance was readable and its WASM hash was present
  in the transaction's own declared footprint, so no code identity was
  unresolved. This is the case the protocol most needed to be sure about, since
  an unresolved identity is never allowed to fall back to a contract ID.
- Check 5 can only be zero here: every fixture in `fixtures/failed/` was applied
  in a ledger older than this window.

No root contract was a Stellar Asset Contract, so the
`builtin:stellar-asset-contract` cluster was never used.

## 5. Concentration: what actually fails on mainnet

Eligible transactions, census 1:

| | Distinct | Top 10 hold | Seen once |
|---|---|---|---|
| `code_cluster` | 28 | 95.0% | 3 |
| `submitter_cluster` | 613 | 76.5% | 267 |

The largest code cluster is 3,566 of 6,221 eligible transactions (57%), all
from a single submitter. 75% of eligible transactions are fee-bumped, and one
fee-bump sponsor accounts for 4,357 of them.

Raw terminal host error of eligible transactions — an observation about the
population, not a classification:

| Terminal error | Count |
|---|---|
| `Storage/ExceededLimit` | 3,756 |
| `Contract` (a contract's own error code) | 1,474 |
| `Auth/ExistingValue` | 864 |
| `WasmVm/InvalidAction` | 80 |
| `Budget/ExceededLimit` | 28 |
| `Auth/InvalidInput` | 14 |
| none | 4 |
| `Auth/InvalidAction` | 1 |

The four with no terminal host error were checked by hand against RPC: each is
an `InvokeHostFunction` result of `resource_limit_exceeded`, a failure the host
reports in the result code without an error event.

## 6. Yield: how many samples a round can produce

Yield is computed by running the protocol's own selection (§6.4) over the
census records: candidates in a random order, one sample per `code_cluster`,
three per `submitter_cluster`. 200 random subsets of the census ledgers per
value of `K`.

| `K` (ledgers sampled) | mean yield | p5 | p95 |
|---|---|---|---|
| 50 | 12.7 | 9 | 17 |
| 100 | 16.3 | 13 | 20 |
| 200 | 19.3 | 15 | 23 |
| 300 | 21.4 | 19 | 24 |
| 500 | 24.1 | 22 | 26 |
| 750 | 25.8 | 24 | 27 |
| 1,000 | 27.0 | 27 | 27 |

The curve flattens because it is bounded by the number of distinct code
clusters that fail at all. At `K = 1,000` the yield is 27 against 28 observed
clusters: the one lost sample is the `submitter_cluster` cap biting, because
the dominant fee-bump sponsor submits into four different code clusters and may
supply only three samples.

**How many clusters exist that the census did not see?** Using ledgers as
sampling units, 4 clusters appeared in exactly one sampled ledger and 2 in
exactly two, giving a Chao2 lower bound of **32** for the window, and an
expected 30–32 distinct clusters at two or three times the ledgers sampled.
Chao2 is a lower bound, and it cannot see a contract that fails only a handful
of times per week: for that, census 2 samples the window four times as densely
(§7).

**Do later rounds help?** Splitting census 1's window into two consecutive
halves of about 3.5 days each, and applying the caps cumulatively:

| | Ledgers | Eligible | Accepted |
|---|---|---|---|
| Round 1 (first half) | 502 | 3,429 | 23.0 |
| Round 2 (second half) | 498 | 2,792 | 4.0 new |

24 clusters appeared in each half and 20 in both. Against the earlier,
now-expired window of 2026-09-17 (a smaller sample: 314 ledgers, 18 clusters),
15 of its 18 clusters recur in this window.

So a round adds roughly **4 new code clusters** after the first, and the first
yields about **25**, or nearer 30 at the sampling density chosen in §8.
Reaching 100 samples would need on the order of 19 further
rounds — about 70 days of consecutive windows — and only if new contracts keep
appearing at the rate observed over one week, which this pilot cannot promise.

## 7. Census 2: sampling four times as densely

Census 1 samples 0.8% of the window's ledgers, so a contract that fails a few
times a week can be missed entirely. Census 2 asked for 4,000 ledgers of the
same window. It was **stopped by a time budget after 1,676 ledgers**, having
worked through the window in ascending order, so it covers
**64,443,173 – 64,492,943**: 49,771 ledgers, about 2.9 days, sampled at 3.4%.
Only that range is treated as measured. Within it the sample is a complete
stratified one, so restricting census 1 to the same range compares like with
like:

| Over ledgers 64,443,173 – 64,492,943 | Census 1 | Census 2 |
|---|---|---|
| Ledgers read | 418 | 1,676 |
| Share of the range sampled | 0.84% | 3.4% |
| Transactions read | 117,342 | 478,496 |
| Failed Soroban | 2,983 | 8,438 |
| Distinct eligible code clusters | 23 | **28** |
| Chao2 lower bound | 33.0 | 40.5 |
| Yield at `K` = 300 | 20.5 | 20.3 |
| Yield at `K` = 1,500 | — | 26.5 |

Four times the density found **five more code clusters**, a 22% increase, and
the Chao2 bound rose with it: there is a tail of rarely failing contracts, and
it is reached slowly. Extrapolating census 2's own curve, sampling this range
at 10% would be expected to find about 35 distinct clusters rather than 28.

Density is therefore a real but sharply diminishing lever: it is worth setting
`K` above the density census 1 used, and it is not worth reading a whole
window. Nothing about the shape of the population changed — the same handful of
contracts still produce almost all failures.

### An outage that both providers shared

During census 2, 41 consecutively sampled ledgers (64,478,219 – 64,479,386)
could not be read: four attempts on the collection provider and one on the
fallback all returned `unexpected end of file`, the first preceded by a
provider-side `timeout` inside an otherwise valid JSON response. Both ledgers
at the ends of that burst were read successfully from **both** providers
afterwards, so the failure was transient and not a property of the data.

Two things follow, and both are recorded rather than smoothed over:

- A fallback provider does not help against whatever this was. 2.4% of census
  2's sampled ledgers were lost in one burst, and because a census reads
  ledgers in ascending order, the loss is a contiguous stretch of chain rather
  than a scatter.
- Under §6.3 each such ledger is `ledger_unreadable` and contributes nothing.
  The protocol does not forbid attempting a ledger again later in the same
  round, and the collector (#21) should do exactly that before recording one as
  unreadable, since a later attempt evidently succeeds.

## 8. The parameters, and why

| Parameter | Value | Why |
|---|---|---|
| `Δ` | 60,000 ledgers (about 3.5 days) | Leaves 59,979 ledgers — over three days — between a window closing and its data ageing out of a 120,000-ledger retention, so one outage cannot cost a round. Round yield is governed by calendar span, not by window size, so a larger `Δ` buys nothing but risk. |
| `K` | 3,000 ledgers | 5% of a 60,000-ledger window: above the density at which census 2 found its extra clusters (§7), and past the point where more scanning pays for itself. Census 2 read 1,676 ledgers in about 2.6 hours, so a round costs a few hours of scanning inside a scan slot of over three days. |
| `R_max` | 8 | Eight consecutive windows span about 28 days. At the measured turnover this projects about 45–60 samples. More rounds would add about 4 samples each; the maintainer can pre-register a larger `R_max` before the freeze if the calendar allows. |
| `T` | 100 | Unchanged as the protocol's target, and **not expected to be reached**: see §9. Setting it lower would cap a dataset that the population might yet exceed; the stopping rule (§6.5) publishes the shortfall. |
| `L_seed` | 65,000,000 | Comfortably more than 1,000 ledgers after the freeze commit: at the measured mean close time of 5.091 s it closes about 2026-10-18, roughly 26 days after this pilot. Round 1's window then closes about 2026-10-22 and must be scanned by about 2026-10-25. |
| Collection provider | `https://rpc.lightsail.network` | Retains 120,960 ledgers; read 1,000 whole ledgers with zero failed requests, about three times faster than the alternative. |
| Fallback provider | `https://mainnet.sorobanrpc.com` | Retains 120,960 ledgers; answers identically (§3, check 4). It closed connections for a sustained period on 2026-09-17 and is the slower of the two, which is why it is the fallback rather than the collection provider. |

**`L_seed` presumes a schedule.** Round 1 can only be scanned in a fixed
interval of about three days, roughly a month from now, and only by a tagged
`eval-build-v1` (#25). If #20–#24 are not finished by then, round 1 is recorded
as `round_missed` and one of the eight rounds is spent for nothing. `L_seed` is
the one parameter that should be revisited before the merge that freezes this
protocol, because afterwards it cannot change without a protocol version 2.

## 9. Is 100 samples feasible? No

Under protocol v1 as written, with one sample per `code_cluster`:

- one round yields about 25 to 30;
- each further round adds about 4;
- eight rounds project about **45–60 samples**, collected over about a month.

This is a property of mainnet, not of the tooling: in a whole week, only about
30 distinct contract codes fail at all, and two thirds of failures come from
ten of them. No amount of extra scanning changes that, and the pilot did not
try to work around it. Specifically, the yield was **not** raised by relaxing a
cap, by counting one contract twice, by treating a contract ID as a code
identity, or by including transactions the funnel excludes.

What this means for the published figures, stated now rather than after the
results are known:

- With 50 answers, the half-width of a Wilson 95% interval is about 13
  percentage points at a proportion of 0.5 and about 9 at 0.9. Every headline
  figure will carry an interval that wide, and the run will report it.
- `Confirmed` cannot **meet** its 0.95 target on this dataset:
  [metrics.md §7](../evaluation/metrics.md) requires at least 73 answers at a
  level for that, so its calibration status will at best be *inconclusive*, and
  inconclusive is not a pass.
- Per-class figures need 20 answers for a full report; most classes will have
  fewer and will be published as counts only.

Three ways to get a larger dataset exist, and **none is taken here**, because
each changes the methodology rather than a parameter, and that decision is the
maintainer's, before the freeze:

1. **Raise the `code_cluster` cap** from 1 to, say, 3. This roughly triples the
   yield, at the cost of a dataset in which one popular contract can dominate
   the headline figure.
2. **Collect over months** — `R_max` of 20 or more.
3. **Lean on `constructed-v1`** (#26), where cases are built rather than found,
   and labels are genuine independent ground truth. It cannot replace mainnet
   evidence, but it measures classes mainnet rarely produces.

## 10. Limitations

- The census samples ledgers, so a contract that fails a handful of times a
  week is unlikely to be seen at all. Every figure here is about contracts that
  fail often enough to be sampled; the true number of distinct failing codes is
  higher than 28, and Chao2's 32 is a lower bound, not an estimate of the tail.
- One week of mainnet is one week. Turnover is estimated from two half-windows
  and one earlier window, not from months of history.
- The yield projection assumes future rounds resemble these. A new popular
  contract, or a protocol upgrade, would change it in either direction.
- Cluster keys were resolved from instance entries read **at census time**, not
  at the time each transaction ran. A contract upgraded in between would show
  as `code_identity_not_in_footprint`; none did, so this did not arise here,
  but a round scanned days after its window could see some.
- Census 2 was stopped by a time budget, not by its own stopping rule; §7 says
  exactly how much of its window it covered and treats only that sub-window as
  measured. Its 41 unreadable ledgers were not attempted again before the stop.
- Both censuses ran from one machine on one connection. A failure seen on both
  providers at once (§7) cannot be attributed to either provider from here.

## 11. Reproducing this

```bash
# 1. Check the protocol's live assumptions (network).
cargo run --release -p soroban-failure-rpc --example verify_protocol_assumptions

# 2. Re-run a census (network, hours). Any window inside retention will do;
#    the exact windows above have since aged out.
cargo run --release -p soroban-failure-rpc --example survey_failures -- \
    --ledgers 1000 --window <LO>..<HI> --seed <TEXT> \
    --rpc https://rpc.lightsail.network --fallback https://mainnet.sorobanrpc.com \
    --jsonl pilot.jsonl --ledger-log ledgers.jsonl

# 3. Recompute every number in §4 to §7 from the committed census (offline).
cargo run --release -p soroban-failure-rpc --example pilot_report -- \
    --jsonl evaluation/pilot/census-1.jsonl.gz \
    --ledger-log evaluation/pilot/census-1-ledgers.jsonl.gz \
    --split 64501205 --trials 200

# 4. Regenerate the exclusion list (offline).
cargo run --release -p soroban-failure-rpc --example pilot_report -- \
    --exclusions evaluation/pilot/*.jsonl.gz > evaluation/exclusions/pilot-v1.txt
```

The committed census files under `evaluation/pilot/` are the data these numbers
come from. They are development data: no transaction they name may ever enter
an evaluation dataset, which is what `evaluation/exclusions/pilot-v1.txt`
enforces.
