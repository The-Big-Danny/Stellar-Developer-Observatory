# M5 evaluation protocol

> **Status: DRAFT — protocol version `1-draft`.**
>
> This document is not frozen. It becomes version `1` when issue #19 fills in
> the [open parameters](#18-open-parameters) from the population pilot, and the
> merge commit is tagged `eval-protocol-v1`. No evaluation data may be collected
> before then.
>
> **This document contains no results.** Nothing here states, estimates or
> predicts SDO's accuracy.

Metric definitions and worked arithmetic are in [metrics.md](metrics.md), which
is part of this protocol and is frozen with it.

---

## 1. Purpose

M5 answers one question: **how often is SDO right, and how often is it wrong?**
The answer is published with every miss.

The evaluation measures how well SDO's top-ranked cause, and its decisions not
to answer, agree with reference labels. Those labels are assigned blind to SDO,
from real failed Soroban transactions, with the analysis code fixed at a tagged
commit.

Its limits must be stated wherever results are quoted:

- For **tier A** labels (construction or replay) it measures agreement with
  independent ground truth.
- For **tier B and C** labels it measures agreement with **expert reference
  labels**. The labellers interpret some of the same evidence SDO reads, so
  agreement there is not proof of what happened on chain.
- It measures SDO at one build. A later build needs a new run.

## 2. Terminology

| Term | Meaning |
|---|---|
| **SDO** | Stellar Developer Observatory: `soroban-failure-analysis`, `soroban-failure-rpc` and the `sdo` CLI |
| **Evaluated build** | The commit tagged `eval-build-v1`, from which collection and prediction run |
| **Development data** | Anything used to design or test rules: `fixtures/failed/`, `fixtures/contracts/`, the M4 survey output, and the population pilot (#19). Never used to evaluate. |
| **Evaluation data** | A dataset under `evaluation/datasets/`. Never used to design, tune or test rules. |
| **Sample** | One failed transaction in an evaluation dataset |
| **Reference label** | The cause assigned to a sample by the labelling procedure in §8. Never "gold" or "ground truth" without qualification. |
| **Independent ground truth** | A reference label with `evidence_tier: A` |
| **Expert reference label** | A reference label with `evidence_tier: B` or `C` |
| **Prediction** | What SDO's evaluated build outputs for a sample (§11) |
| **Abstention** | A prediction whose verdict is `InsufficientEvidence` or `Unsupported` |
| **Headline figure** | A figure computed on `mainnet-v1` as defined in [metrics.md](metrics.md) |

SDO's output semantics are used exactly as implemented:

- **Verdict** (`Diagnosis::verdict()`): `Explained(confidence)` when at least
  one rule matched; `InsufficientEvidence` when some rule could apply but none
  found enough evidence; `Unsupported` when no implemented rule concerns the
  failure; `NotAFailure` when the transaction succeeded.
- **Confidence**: `Confirmed`, `Likely`, `Possible`.
- `Explained` always carries the top-ranked candidate's confidence. An
  abstention always has no candidates.

## 3. Datasets

| Dataset | Source | Reference labels | Role |
|---|---|---|---|
| `mainnet-v1` | Stellar mainnet, sampled per §6 | Blind double labelling, §8 | **Headline figures** |
| `constructed-v1` | Testnet transactions built to fail in a known way, §10 | By construction (tier A) | Classes absent from mainnet; checks for false confirmations |

The two datasets are **never pooled**. No figure combines them.

Protocol v1 defines no other dataset. In particular, v1 has no uncapped
"natural distribution" set.

## 4. Sample qualification

### 4.1 Funnel

Every transaction observed during collection is counted in exactly one place
in `funnel.json`:

```
scanned
└─ status is FAILED                                  otherwise: not_failed
   └─ passes the eligibility checks (§4.2)           otherwise: an exclusion reason
      └─ eligible
         └─ accepted by selection (§6.4)             otherwise: a selection reason
            └─ captured (§7)                         otherwise: capture_failed
               └─ sample
```

**Nothing is excluded silently.** Every exclusion has exactly one reason code,
from the lists below.

### 4.2 Eligibility checks

Checks run in this order. A transaction is recorded under the **first** check
it fails, so the funnel is unambiguous.

| # | Check | Exclusion reason |
|---|---|---|
| 1 | The `getTransaction` result decodes with the evaluated build's decoder | `decode_failed` |
| 2 | The inner transaction, after fee-bump unwrapping, contains a Soroban operation (`InvokeHostFunction`, `ExtendFootprintTtl` or `RestoreFootprint`) | `not_soroban` |
| 3 | The inner transaction has exactly one operation | `unexpected_operation_count` |
| 4 | At least one diagnostic event decodes, from any location the decoder supports | `no_diagnostic_events` |
| 5 | The transaction hash (outer and inner) is not development data (§13.1) | `development_data` |
| 6 | The operation is `InvokeHostFunction` whose host function is `InvokeContract` | `no_root_contract_invocation` |
| 7 | The root contract's code identity resolves (§5.1) | `code_identity_instance_unavailable`, `code_identity_not_in_footprint`, `code_identity_unsupported_executable` |

A transaction passing all seven is **eligible**.

### 4.3 Selection and capture reasons

| Reason | Meaning |
|---|---|
| `cap_code_cluster` | Accepting it would put a second sample in its `code_cluster` |
| `cap_submitter_cluster` | Accepting it would put a fourth sample in its `submitter_cluster` |
| `not_selected_target_reached` | Eligible, but the target sample count was already reached |
| `capture_failed` | Accepted, but its transaction could not be recorded (§7) |

## 5. Deduplication

### 5.1 `code_cluster`

A sample's `code_cluster` identifies the code its root contract runs. It is
determined **only** by these steps, in order:

1. **Root contract.** The contract address in the envelope's
   `InvokeHostFunction` → `InvokeContract` (inner transaction). A transaction
   with any other host function has already been excluded as
   `no_root_contract_invocation` (§4.2, check 6).
2. **Instance.** Read the root contract's instance ledger entry
   (`ContractData`, key `LedgerKeyContractInstance`, persistent) with
   `getLedgerEntries` during scanning.
   - If the entry cannot be read (not found, archived, lookup failure after
     retries) → **unresolved**, `code_identity_instance_unavailable`.
3. **Executable.**
   - `ContractExecutable::Wasm(h)`: if `h` appears as a `ContractCode` key in
     the inner transaction's declared footprint (read-only ∪ read-write), then
     `code_cluster = "wasm:" + lowercase_hex(h)`. Otherwise → **unresolved**,
     `code_identity_not_in_footprint`. This covers contracts upgraded since the
     transaction ran, and transactions with no Soroban transaction data.
   - `ContractExecutable::StellarAsset`:
     `code_cluster = "builtin:stellar-asset-contract"`. Every Stellar Asset
     Contract instance runs the same built-in host code, so one shared cluster
     keeps the one-code-cluster cap meaningful. No footprint check applies,
     because there is no `ContractCode` entry.
   - Any other executable → **unresolved**,
     `code_identity_unsupported_executable`.

**An unresolved code identity never falls back to the contract ID.** Such a
transaction is excluded from every evaluation dataset, counted in the funnel
under its reason, and its hash is listed in `candidates.jsonl`. The population
it represents stays visible, and deduplication stays sound.

### 5.2 `submitter_cluster`

- If the transaction is fee-bumped: the fee-bump fee source, as a strkey.
- Otherwise: the inner transaction's source account, as a strkey.

A muxed account (`M…`) is reduced to its underlying `G…` account, so one account
cannot evade the cap with many muxed IDs.

### 5.3 Caps

Caps apply cumulatively across all rounds of a dataset:

- at most **1** sample per `code_cluster`;
- at most **3** samples per `submitter_cluster`.

Both keys are derived from the envelope and ledger entries only, never from
SDO's diagnosis.

## 6. Sampling

### 6.1 Rounds and windows

A dataset is collected in up to `R_max` rounds ([§18](#18-open-parameters)).

- At the start of round `r`, call `getHealth` and set the window
  `lo_r = oldestLedger + 500`, `hi_r = latestLedger − 20`. The margins keep the
  scan clear of the ledger range sliding out of retention.
- Round `r > 1` may start only when `lo_r > hi_(r−1)`. Rounds therefore never
  overlap. With RPC's roughly 7-day retention, rounds are about a week apart.
- The RPC provider, `getVersionInfo`, `getNetwork` and `getHealth` responses,
  and the start and end time of each round are recorded.

### 6.2 Seed

The dataset seed is the 32-byte hash of mainnet ledger `L_seed`. `L_seed` is a
ledger sequence number fixed in protocol v1, chosen to be **later than the
latest ledger at the time of freezing**. The seed is therefore unknown to
everyone when the protocol is frozen and cannot be picked to favour an outcome.

### 6.3 Page selection

Round `r` scans `P` pages ([§18](#18-open-parameters)) of `getTransactions`,
each with `limit = page_size`. Page `i` (0-based) starts at:

```
start(r, i) = lo_r + ( U64_BE( SHA-256( seed ‖ "page" ‖ U32_BE(r) ‖ U32_BE(i) )[0..8] )
                       mod (hi_r − lo_r + 1) )
```

- `seed` is 32 raw bytes, `"page"` is its ASCII bytes, and `U32_BE`/`U64_BE` are
  big-endian integers.
- Transactions appearing on several pages are counted once, by hash.
- Failed transactions are recorded in `candidates.jsonl` with their cluster
  keys and eligibility outcome.

### 6.4 Selection

Selection is a pure function of the candidates, the seed, the caps, the target
and the exclusion lists. Within each round, in round order:

1. Sort the round's eligible candidates by
   `SHA-256( seed ‖ "order" ‖ transaction_hash )` ascending, comparing bytes.
   The ordering does not depend on scan order.
2. Walk them in that order. For each candidate:
   - if the dataset already holds `T` samples → `not_selected_target_reached`;
   - else if its `code_cluster` already has a sample → `cap_code_cluster`;
   - else if its `submitter_cluster` already has 3 samples →
     `cap_submitter_cluster`;
   - else **accept** it and capture it immediately (§7). If capture fails,
     record `capture_failed`. The candidate does not count toward any cap, and
     the walk continues with the next candidate.

Selection never reads a rule outcome, cause class, verdict, confidence or
contract-error resolution. Candidates are **not** stratified by failure shape.

### 6.5 Stopping rule

Collection stops when the dataset holds `T` samples, or after round `R_max`,
whichever comes first.

**The actual sample count is published, including any shortfall from `T`.**
Samples are never added, replaced or dropped outside this procedure to reach a
number.

## 7. Capture

For each accepted candidate, in the same round:

- `rpc-response.json`: the `getTransaction` `result`, verbatim. If it cannot be
  fetched → `capture_failed`.
- Contract ledger entries, verbatim `getLedgerEntries` results, under
  `contracts/<CONTRACT_ID>/` (`instance.json`, and `code.json` when WASM
  backed). The set of contracts is:
  - the root contract, and
  - every contract ID attached to a diagnostic event whose first topic is the
    symbol `error`.

  This set is read from raw events, not from SDO's contract identification. A
  contract whose entries cannot be recorded is **not** an exclusion. Its status
  is recorded in `sample.json` as `instance_unavailable`, `code_unavailable` or
  `stellar_asset_contract`, and SDO's behaviour without that spec is part of
  what is measured.
- `sample.json`, and the dataset's `manifest.json` and `funnel.json`, per the
  schemas defined in #20.

## 8. Reference labels

### 8.1 Label record

Each labeller writes one record per sample:

| Field | Values |
|---|---|
| `reference_label` | A cause class from §8.2, or `AMBIGUOUS`, `INDETERMINATE`, `OUT_OF_TAXONOMY` |
| `candidate_classes` | Required iff `AMBIGUOUS`: two or more cause classes |
| `description` | Required iff `OUT_OF_TAXONOMY`: the cause, in words |
| `reference_label_source` | `construction`, `replay`, `source_analysis`, `raw_evidence` |
| `evidence_tier` | `A`, `B`, `C`, determined by the source (§8.3) |
| `label_certainty` | `certain`, `probable` |
| `contract_error_name` | Optional, only for `ContractDefinedError`: the error name, verified from contract source or the on-chain spec and cited |
| `evidence` | Citations: `{artefact, locator, note}`, with `artefact` one of `envelope`, `result`, `meta`, `diagnostic_event`, `contract_spec`, `contract_source`, `replay_log`, `construction_plan`, `host_source`, `documentation` |
| `rationale` | Why the evidence establishes the label |
| `labeller` | GitHub handle |
| `labeller_involvement` | `m4_rule_author`, `sdo_contributor`, `external` |
| `labelled_at`, `guide_version`, `protocol_version` | Provenance |

### 8.2 Label space

The cause classes available as reference labels in protocol v1:

`MissingAuthorizationEntry`, `InvalidAuthorizationEntry`,
`FootprintEntryMissing`, `ArchivedEntryRequiresRestore`,
`ResourceLimitExceeded`, `InsufficientResourceFee`, `ContractDefinedError`,
`ContractTrap`, `MalformedHostFunction`.

`CauseClass::Undetermined` is **not** a valid reference label; use
`INDETERMINATE`. A cause class added to SDO after the freeze is not part of v1's
label space.

The three sentinels:

| Sentinel | Use when |
|---|---|
| `AMBIGUOUS` | The evidence supports two or more classes and cannot separate them |
| `INDETERMINATE` | The evidence does not establish any class |
| `OUT_OF_TAXONOMY` | The evidence clearly establishes a cause that no class in §8.2 describes |

These are legitimate reference labels, not labelling failures. Each is scored
by its own rules ([metrics.md §6](metrics.md#6-sentinel-samples)).

### 8.3 Sources and tiers

A labeller records the **strongest single source** that establishes the label
on its own.

| `reference_label_source` | `evidence_tier` | Definition |
|---|---|---|
| `construction` | **A** | The transaction was built deliberately to fail this way, its intended label was committed before submission, and the recorded result matches the construction (§10) |
| `replay` | **A** | Re-executing the recorded transaction against its pre-execution ledger state, with an implementation other than SDO, reproduces the failure, **and** a minimal counterfactual change that removes the claimed cause makes that failure disappear. Replay that only reproduces the failure is not tier A. |
| `source_analysis` | **B** | The contract's source code, shown to correspond to the executed WASM hash, together with raw evidence establishes the cause |
| `raw_evidence` | **C** | The raw envelope, result, meta and diagnostic events, interpreted from host semantics, establish the cause |

Only tier A is described as **independent ground truth**. Tiers B and C are
**expert reference labels** everywhere they are reported.

### 8.4 Acceptable evidence

Acceptable:

- the recorded `rpc-response.json` and contract ledger entries, viewed through
  the neutral labelling view (#22) or any decoder independent of SDO's analysis;
- contract source code, with how it corresponds to the executed WASM;
- `soroban-env-host` source at a pinned commit, and Stellar documentation;
- replay logs, with tool, version, ledger state and command;
- construction plans committed before submission.

**Never acceptable**, as evidence or as the basis of a label:

- any SDO output: `sdo explain`, a `Diagnosis`, `TransactionModel` summaries,
  contract identification or error resolution;
- `docs/architecture/rules.md` and the rule modules in
  `crates/soroban-failure-analysis/src/rules/`;
- `failure_category` values in fixture metadata;
- output of any automated classifier, including language models, as the label
  or its sole basis.

### 8.5 Blind double labelling

For `mainnet-v1`:

1. Two labellers label every sample independently, each blind to SDO's output
   and to the other's labels, following the labelling guide (#23).
2. **Commit-reveal.** Before any label file is committed, each labeller posts on
   the labelling issue the SHA-256 of their label bundle: the bytes of their
   label files concatenated in ascending transaction-hash order. Label files
   are committed only after both digests are posted, and must match them.
3. Each labeller declares `labeller_involvement`.

### 8.6 Agreement and adjudication

Two labels **agree** when `reference_label`, the set of `candidate_classes`,
`reference_label_source`, `evidence_tier` and `contract_error_name` are all
identical.

- If they agree, the final label is that label. Its `label_certainty` is
  `probable` if either labeller recorded `probable`.
- Otherwise a third person, who is neither labeller, adjudicates. The
  adjudication records the final label and its rationale, and both original
  labels are preserved.

Agreement is published as raw percentage agreement and Cohen's κ
([metrics.md §9](metrics.md#9-labeller-agreement)).

### 8.7 Independence requirement

Each `mainnet-v1` sample should have at least one labeller whose
`labeller_involvement` is not `m4_rule_author`.

- If every sample meets this, the run's labels are described as independently
  labelled.
- Otherwise the report states how many samples do not meet it, and every
  headline figure is also reported on the subset that does.

### 8.8 Locking

After adjudication, `labels-lock.json`, the digest of the final labels, is
committed. **No prediction may be generated for a dataset before its labels are
locked.**

## 9. Evidence and SDO

This protocol never uses SDO's diagnosis as a reference label or as evidence
for one. The rules in §8.4 and the neutral view exist to keep SDO's conclusions
out of the labelling process entirely.

## 10. Constructed dataset

`constructed-v1` follows §4–§8 with these differences:

- **Source.** Testnet transactions from minimal contracts and scripts committed
  under `evaluation/constructed/`.
- **Labels before submission.** Each case's plan, committed before its
  transaction is submitted, records the intended `reference_label`
  (`reference_label_source: construction`, `evidence_tier: A`) and an
  `expected_raw_signal`: the result code or raw host error type the
  construction must produce.
- **Construction mismatch.** If the recorded result does not show the
  `expected_raw_signal`, the case is excluded as `construction_mismatch`. It is
  never relabelled to fit what happened. This check compares raw evidence only.
- **Deduplication.** Caps (§5.3) do not apply. Constructed cases deliberately
  reuse contracts.
- **Labelling.** A single construction label is sufficient, because it is
  tier A. §8.5–§8.7 do not apply; §8.8 does.
- **Relationship to development fixtures.** #1 produces development fixtures in
  `fixtures/failed/`. Techniques may be shared, but no transaction appears in
  both.

## 11. Prediction

- Predictions are generated by `sdo-eval predict` at the evaluated build
  (`eval-build-v1`), only after the dataset's labels are locked (§8.8).
- Each sample is decoded with the build's decoder. Contract specs come from the
  sample's recorded `contracts/`, requested the way SDO requests them.
- Recorded per sample:
  - the verdict;
  - every candidate cause in rank order, with class, confidence and rule id;
  - contract-error reports;
  - rule reports.
- The **top-1 prediction** is the first candidate's class and confidence when
  the verdict is `Explained`, and an abstention of the named kind otherwise.
- If a sample fails to decode, or yields `NotAFailure`, at prediction time, it
  is an **integrity error**. It is listed in the report, excluded from every
  metric's denominator, and the run is flagged for investigation. This should be
  impossible for an eligible sample at the same build.

## 12. Scoring

All metrics, scoring rules, reporting thresholds, calibration targets and
intervals are defined in [metrics.md](metrics.md). In particular:

- **abstentions count as misses** for headline top-1 accuracy;
- **selective accuracy is always shown with coverage**;
- results are reported per evidence tier;
- the two datasets are never pooled.

## 13. Leakage and selection-bias controls

### 13.1 Development data is never evaluation data

Before a transaction can be eligible, its outer and inner hashes are checked
against:

- every transaction in `fixtures/failed/`;
- `evaluation/exclusions/pilot-v1.txt` (#19);
- any further list under `evaluation/exclusions/` committed before the freeze.

A CI guard (#20) fails if any evaluation sample's hash appears under `crates/`
or `fixtures/`.

### 13.2 Rules do not see evaluation data

- No production rule may be created, changed or tuned using an evaluation
  sample.
- No evaluation sample may be copied into `fixtures/failed/`.
- A rule defect found through evaluation is filed as a new issue that requires a
  **new development fixture**. The fix is measured in a later run on new
  samples, never on the sample that revealed it.

### 13.3 Summary

| Risk | Control |
|---|---|
| Rules fitted to the test data | Held-out datasets; development-data exclusion; collection only after the build is frozen; CI leak guard |
| Labellers know the rules | Guide built from host semantics (#23); neutral view (#22); blind labelling; involvement recorded; independence requirement |
| Labels adjusted after results | Commit-reveal; labels locked before predictions; post-hoc revisions recorded separately |
| One bot or contract dominates | `code_cluster` cap 1; `submitter_cluster` cap 3; no contract-ID fallback |
| Sample tilted toward recognised failures | Selection blind to SDO output; no stratification by failure shape |
| Seed chosen to favour an outcome | Seed is the hash of a ledger that did not exist at freeze time |
| One time window unrepresentative | Several non-overlapping rounds |
| Survivorship | Every exclusion in the funnel |
| Abstaining inflates accuracy | Abstentions are misses at top-1; selective accuracy only beside coverage |
| Shared evidence inflates agreement | Every figure reported per tier |

## 14. Publication

- Run results are committed under `evaluation/runs/<run-id>/`.
- **Every miss is published** with the fields defined in
  [metrics.md §14](metrics.md#14-misses).
- Every adjudication and the agreement statistics are published.
- A reference label revised after predictions exist is a **post-hoc revision**.
  It is published with its original value and a justification, and headline
  figures use the locked labels.
- The actual sample count of each dataset, the exclusion funnel, and any
  shortfall from `T` are published.
- Failure classes with no samples are stated as **not measured**.
- README, ROADMAP or any other document may quote only figures that appear in a
  published run report. Selective accuracy is never quoted without coverage,
  and tier B and C results are never described as ground truth.

## 15. Reproducibility

Committed for each dataset and run:

- this protocol, `metrics.md` and the labelling guide, with versions;
- the `eval-protocol-v1` and `eval-build-v1` tags;
- the seed ledger, the seed, and every round's `getHealth`, `getVersionInfo` and
  `getNetwork` responses, window, and page start ledgers;
- `candidates.jsonl`, `funnel.json`, `manifest.json` with the SHA-256 of every
  file;
- every sample's verbatim RPC responses and contract ledger entries;
- label files, commit-reveal digests, adjudications, agreement statistics and
  `labels-lock.json`;
- predictions, metrics, misses and the report;
- the Rust toolchain version used for prediction.

## 16. What runs offline

**Online** (collection tooling only): scanning, selection with its eligibility
lookups, capture, replay used for tier A labels, and constructing testnet
cases.

**Offline and deterministic** (everything after collection): dataset
validation, the labelling view, prediction, scoring and report generation.
They read only committed files. Running them twice on the same inputs produces
byte-identical output, and CI regenerates each published run and fails on any
difference.

The analysis crate remains free of I/O throughout.

## 17. Freeze and versioning

| Freeze | When | What it fixes |
|---|---|---|
| `eval-protocol-v1` | When #19 merges, with the open parameters filled | This protocol and `metrics.md` |
| `eval-build-v1` | Before the first `mainnet-v1` collection round (#25); must include #2 | The code that collects, decodes and predicts |

- After `eval-protocol-v1`, **editorial errata** that change no decision (typos,
  broken links) may be made. Each is listed in the changelog below.
- Any **substantive** change creates protocol version `2`. It applies only to a
  later run, never retroactively.
- `main` may continue to change after `eval-build-v1`. Run 1 always evaluates
  the tagged build.

## 18. Open parameters

To be fixed by #19 from the population pilot, before the freeze:

| Parameter | Meaning | Value |
|---|---|---|
| `T` | Target sample count for `mainnet-v1` (at most 100) | *to be set by #19* |
| `R_max` | Maximum number of collection rounds | *to be set by #19* |
| `P` | Pages scanned per round | *to be set by #19* |
| `page_size` | `getTransactions` limit per page | *to be set by #19* (proposed: 200) |
| `L_seed` | Mainnet ledger whose hash is the seed | *to be set by #19* |
| RPC provider | Endpoint used for collection | *to be set by #19* |

## 19. Methodological rules

These rules are binding. No other section may be read as relaxing them.

- Never use SDO's own diagnosis as ground truth or as a reference label.
- Never use an evaluation sample to create or tune a production rule.
- Never copy an evaluation sample into `fixtures/failed/`.
- Never silently exclude a sample; maintain the exclusion funnel.
- Do not manufacture a 100-sample mainnet dataset if the population cannot
  support it; publish the actual count, including any shortfall.
- Abstentions count as misses for headline top-1 accuracy.
- Selective accuracy is always shown with coverage.
- Publish every miss.
- Preserve the `Confirmed` / `Likely` / `Possible` confidence levels and the
  `Explained` / `InsufficientEvidence` / `Unsupported` / `NotAFailure` verdict
  semantics.
- Claim no accuracy until an evaluation run is complete and published.
- The analysis crate stays free of I/O.
- Prediction and scoring are deterministic and offline after collection.

## Changelog

| Version | Change |
|---|---|
| `1-draft` | Initial draft (#18) |
