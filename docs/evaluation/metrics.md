# M5 evaluation metrics

> **Status: DRAFT — part of protocol version `1-draft`.** Frozen together with
> [protocol.md](protocol.md) as `eval-protocol-v1`.
>
> **This document contains no results.** The worked example in §13 is a
> hand-built illustration of the arithmetic. It is not data, and says nothing
> about SDO's accuracy.

Terms (reference label, tier, abstention, evaluated build) are defined in
[protocol.md §2](protocol.md#2-terminology).

---

## 1. Scope

This document defines how predictions are scored against reference labels:
every metric, every scoring rule, the reporting thresholds, the calibration
targets, and the confidence intervals. Every figure in a run report must be one
of the quantities defined here.

## 2. Inputs and notation

For each sample `x` in one dataset:

- **Final reference label** `ref(x)`, after agreement or adjudication
  ([protocol.md §8.6](protocol.md#86-agreement-and-adjudication)), with its
  `evidence_tier` and `label_certainty`.
- **Prediction** from the evaluated build
  ([protocol.md §11](protocol.md#11-prediction)):
  - `verdict(x)`: `Explained`, `InsufficientEvidence` or `Unsupported`;
  - `cands(x)`: the candidate causes in rank order, each with class and
    confidence (empty exactly when the verdict is not `Explained`);
  - `pred(x)`: the class of `cands(x)[0]`, defined only when `answered(x)`;
  - `conf(x)`: the confidence of `cands(x)[0]`, defined only when `answered(x)`.
- `answered(x)` ⇔ `verdict(x) = Explained`.
- `correct(x)` ⇔ `answered(x)` and `pred(x) = ref(x)`.

Sample sets:

| Set | Contains samples whose final reference label is |
|---|---|
| **N** | a cause class from [protocol.md §8.2](protocol.md#82-label-space) |
| **S_amb** | `AMBIGUOUS`, with candidate set `K(x)` |
| **S_ind** | `INDETERMINATE` |
| **S_oot** | `OUT_OF_TAXONOMY` |

Integrity errors ([protocol.md §11](protocol.md#11-prediction)) are handled by
§3.1. Unless §3.1 says otherwise, they belong to no set.

A proportion `k / n` with `n = 0` is **undefined** and reported as `n/a`, never
as 0 or 1.

## 3. Headline metrics

All computed over **N**.

| Metric | Definition |
|---|---|
| **Top-1 accuracy** | `|{x ∈ N : correct(x)}| / |N|` |
| **Coverage** | `|{x ∈ N : answered(x)}| / |N|` |
| **Selective accuracy** | `|{x ∈ N : correct(x)}| / |{x ∈ N : answered(x)}|` |
| **Wrong-answer rate** | `|{x ∈ N : answered(x) ∧ ¬correct(x)}| / |N|` |
| **Unknown rate** | `|{x ∈ N : ¬answered(x)}| / |N|` |
| **Insufficient-evidence rate** | `|{x ∈ N : verdict(x) = InsufficientEvidence}| / |N|` |
| **Unsupported rate** | `|{x ∈ N : verdict(x) = Unsupported}| / |N|` |

**Abstentions count as misses for top-1 accuracy.** An abstention is never
correct.

**Selective accuracy is never reported without coverage beside it.** Selective
accuracy alone rewards abstaining.

These always satisfy:

```
top-1 accuracy + wrong-answer rate + unknown rate = 1
unknown rate = insufficient-evidence rate + unsupported rate
```

A run report checks both identities and fails to generate if either is
violated.

### 3.1 Integrity errors

An **integrity error** is a sample that, at prediction time, fails to decode,
causes the analysis to panic, or yields `NotAFailure`. Every integrity error is
listed individually. Every figure in §3 is reported in two treatments:

| Treatment | Rule |
|---|---|
| **Excluding integrity errors** | Integrity errors are removed from N and from every sentinel set |
| **Integrity errors as wrong answers** | Each integrity error stays in the set its reference label belongs to. In N it counts as answered and not correct, so it adds to the wrong-answer rate and never to top-1 accuracy. This treatment therefore overstates coverage, and the report says so. In a sentinel set its outcome is **integrity error**, counted with sentinel wrong answers. |

Both identities above hold in both treatments. If any integrity error occurs,
the run is **flagged**, and the flag is quoted with every headline figure.

## 4. Ranking metrics

| Metric | Definition |
|---|---|
| **Top-3 accuracy** | `|{x ∈ N : ref(x) is the class of one of the first three entries of cands(x)}| / |N|`. Abstentions have no candidates and never count. |
| **Multi-candidate count** | `|{x ∈ N : |cands(x)| > 1}|`, reported with top-3 accuracy |
| **Ties at the top** | Samples in N ∪ S_amb ∪ S_ind ∪ S_oot where `cands(x)[0]` and `cands(x)[1]` have the same confidence. The top-1 prediction then depends only on rule registration order. Each tie is listed. |

If the multi-candidate count is small, top-3 accuracy adds little information,
and the report says so.

## 5. Per-class metrics

For each cause class `c`, over **N**:

| Metric | Definition |
|---|---|
| **Support** | `n_c = |{x ∈ N : ref(x) = c}|` |
| **Recall** | `|{x ∈ N : ref(x) = c ∧ correct(x)}| / n_c` |
| **Precision** | `|{x ∈ N : ref(x) = c ∧ correct(x)}| / |{x ∈ N : answered(x) ∧ pred(x) = c}|` |
| **False-positive rate** | `|{x ∈ N : answered(x) ∧ pred(x) = c ∧ ref(x) ≠ c}| / |{x ∈ N : ref(x) ≠ c}|` |

Predictions of `c` on sentinel samples are not part of these figures. They are
reported in §6.

**Reporting thresholds**, by support `n_c`:

| Support | Published for class `c` |
|---|---|
| `n_c ≥ 20` | Recall, precision and false-positive rate, each with a Wilson 95% interval (§11) |
| `5 ≤ n_c ≤ 19` | Counts only, marked **insufficient support** |
| `n_c < 5` | Each sample listed individually; no rates |
| `n_c = 0` | **Not measured** |

Even at `n_c = 20` an interval is wide, roughly ±15–20 points. The report states
this beside the figures.

## 6. Sentinel samples

Each sentinel sample receives exactly one **outcome**:

| Reference label | `Explained` | `InsufficientEvidence` | `Unsupported` |
|---|---|---|---|
| `INDETERMINATE` | `conf = Possible`: **hedged answer** · `conf ∈ {Likely, Confirmed}`: **overclaim** | correct abstention | correct abstention |
| `OUT_OF_TAXONOMY` | **wrong answer** | acceptable abstention | correct abstention |
| `AMBIGUOUS` with set `K` | `pred ∈ K` and `conf ≠ Confirmed`: **consistent** · `pred ∈ K` and `conf = Confirmed`: **overclaim** · `pred ∉ K`: **wrong answer** | correct abstention | acceptable abstention |

Reported per sentinel label: the count of each outcome, and every overclaim and
wrong answer listed individually.

| Metric | Definition |
|---|---|
| **Overclaim count** | Samples with outcome overclaim |
| **Hedged-answer count** | Samples with outcome hedged answer, each listed individually |
| **Correct-abstention count** | Samples with outcome correct abstention |
| **Sentinel wrong-answer count** | Samples in S_amb ∪ S_oot with outcome wrong answer, plus, in the second treatment of §3.1, integrity errors in any sentinel set |

`Confirmed` on an `AMBIGUOUS` sample is an overclaim: the evidence was judged
unable to separate the classes, so a confirmed single cause claims more than the
evidence supports.

`Possible` on an `INDETERMINATE` sample is a hedged answer, not an overclaim:
`Possible` means the cause is consistent with the evidence but not established,
which is what `INDETERMINATE` records. Hedged answers are not misses, but each
is listed.

## 7. Calibration

Calibration is computed over **N** only, always excluding integrity errors,
which have no confidence level. Sentinel samples have no single reference
class; overclaims on them are reported in §6 beside the calibration table.

For each confidence level `L` ∈ {`Confirmed`, `Likely`, `Possible`}:

- `n_L = |{x ∈ N : answered(x) ∧ conf(x) = L}|`
- `k_L = |{x ∈ N : answered(x) ∧ conf(x) = L ∧ correct(x)}|`
- **Observed accuracy** `k_L / n_L`, with a Wilson 95% interval `[lo_L, hi_L]`.

Confidence levels are defined by the evidence each rule requires
([docs/architecture/rules.md](../architecture/rules.md)). Calibration does not
redefine them, and **assigns no probability to any individual diagnosis**.

A **target** is a pre-registered minimum proportion of correct top-1 answers
among all answers given at one confidence level, on one dataset. It states what
that level's evidence requirements are expected to deliver **in aggregate**, not
the chance that any particular answer is correct.

**Pre-registered targets:**

| Level | Target |
|---|---|
| `Confirmed` | 0.95 |
| `Likely` | 0.70 |
| `Possible` | no target |

**Status**, applied in this order. Each status uses one side of a two-sided 95%
Wilson interval (§11), so each is a one-sided test at the 2.5% level:

| Status | Condition | Meaning |
|---|---|---|
| **insufficient support** | `n_L < 20` | Too few answers at this level to assess |
| **meets target** | `lo_L ≥ target` | The observed proportion supports the target even at the low end of its interval |
| **below target** | `hi_L < target` | The observed proportion is too low for the target to be plausible |
| **inconclusive** | otherwise | The interval contains the target: the data neither support nor contradict it. **Inconclusive is not a pass.** |

`Possible` receives only **insufficient support** or its observed proportion,
with no status against a target.

A level never meets its target on its point estimate alone. With `k = n`, the
Wilson lower bound is exactly `n / (n + z²)`, which first reaches 0.95 at
`n = 73` (72 gives 0.9494; 73 gives 0.9500). So **`Confirmed` meets 0.95 only
with at least 73 answers at that level and none wrong**. With one wrong answer
it needs at least 110 answers; with two, at least 142.

At the sample sizes M5 can reach, `Confirmed` cannot realistically meet its
target, and its informative outcome is **below target**: for example 17 or
fewer correct of 20, or 44 or fewer of 50. For `Likely`, meeting 0.70 needs at
least 19 correct of 20 or 42 of 50, and below target is 9 or fewer of 20 or 28
or fewer of 50. Reports present calibration as a test that can show a level
falls short, **never as certification** of a level.

Calibration is not expected calibration error, because the three levels are
ordinal, not probabilities.

## 8. Contract-error name accuracy

A secondary metric for milestone M3's error resolution. Over samples in **N**
with `ref(x) = ContractDefinedError` and a `contract_error_name` in the final
reference label:

- **resolved**: SDO's terminal contract-error report for `x` has a resolved
  name;
- **name accuracy** = `|resolved ∧ SDO's name = contract_error_name| / |resolved|`;
- **unresolved count**: the remaining samples, reported beside it.

It is computed independently of whether the top-1 prediction is correct.

## 9. Labeller agreement

Over samples with two independent labels, before adjudication:

- **Percentage agreement**: `agreements / n`, where agreement is defined in
  [protocol.md §8.6](protocol.md#86-agreement-and-adjudication).
- **Cohen's κ** on `reference_label` alone. An `AMBIGUOUS` label with its
  candidate set counts as one category per distinct set.

  ```
  p_o = (samples with the same reference_label) / n
  p_e = Σ_k (n_{X,k} / n) · (n_{Y,k} / n)
  κ   = (p_o − p_e) / (1 − p_e)        undefined when p_e = 1
  ```

  where `n_{X,k}` is the number of samples labeller X labelled `k`.

Both are reported overall and per final evidence tier.

## 10. Headline and secondary figures

The **headline figures** are exactly four, all on `mainnet-v1` over all of N:

1. top-1 accuracy;
2. coverage;
3. wrong-answer rate;
4. unknown rate.

Each is reported in both integrity-error treatments (§3.1).

**Every other figure is secondary** and descriptive: every breakdown below, and
every ranking, per-class, sentinel, calibration, name-accuracy, agreement and
`constructed-v1` figure. No correction for multiple comparisons is applied to
secondary figures, so a secondary figure that looks unusually good or bad may be
chance.

- A secondary figure is never quoted without the headline figures beside it.
- **No secondary figure or subset becomes a headline after results are known.**
  The headline set is fixed by this section and changes only in a later
  protocol version.

Every figure in §3–§8 is also reported as a secondary breakdown:

- **by evidence tier and source**: tier A `construction` (agreement with
  independent ground truth), tier A `replay` (agreement with replay-verified
  reference labels), tier B and tier C (agreement with expert reference labels).
  Tier A figures are never combined across the two sources;
- for `mainnet-v1`, also on the subset whose final `label_certainty` is
  `certain`;
- for `mainnet-v1`, also on the subset meeting the independence requirement,
  whenever not every sample meets it
  ([protocol.md §8.7](protocol.md#87-independence-requirement));
- **separately per dataset**. `mainnet-v1` and `constructed-v1` are never
  pooled.

## 11. Confidence intervals

Every proportion `k / n` with a reported interval uses the **Wilson score
interval** at 95%, `z = 1.959963984540054`:

```
p̂      = k / n
centre = (p̂ + z² / 2n) / (1 + z² / n)
half   = z · √( p̂(1 − p̂) / n + z² / 4n² ) / (1 + z² / n)
[lo, hi] = [max(0, centre − half), min(1, centre + half)]
```

It is undefined when `n = 0`.

Reference values, to four decimal places:

| `k / n` | `lo` | `hi` |
|---|---|---|
| 3 / 4 | 0.3006 | 0.9544 |
| 19 / 20 | 0.7639 | 0.9911 |
| 20 / 20 | 0.8389 | 1.0000 |
| 0 / 5 | 0.0000 | 0.4345 |
| 0 / 0 | undefined | undefined |

## 12. Reporting format

- Every proportion is shown as both `k / n` and a decimal to three places,
  e.g. `5 / 9 (0.556)`.
- Every headline figure states its `N`.
- Rates are never rounded into a threshold or target they do not meet.
- Undefined values are `n/a`.

## 13. Worked example

> **Illustration only.** The fifteen samples below are invented to exercise
> every rule in this document. They are not evaluation data, not fixtures, and
> say nothing about SDO's accuracy.

### 13.1 Samples

| Sample | Final reference label | Tier | Verdict | Candidates, in rank order |
|---|---|---|---|---|
| s1 | `FootprintEntryMissing` | C | Explained | FootprintEntryMissing (Confirmed) |
| s2 | `FootprintEntryMissing` | C | Explained | FootprintEntryMissing (Possible) |
| s3 | `ContractDefinedError` | B | Explained | ContractDefinedError (Confirmed) |
| s4 | `ContractDefinedError` | B | Explained | InvalidAuthorizationEntry (Likely), ContractDefinedError (Possible) |
| s5 | `InvalidAuthorizationEntry` | B | Explained | InvalidAuthorizationEntry (Likely) |
| s6 | `MissingAuthorizationEntry` | C | InsufficientEvidence | — |
| s7 | `ContractTrap` | B | Unsupported | — |
| s8 | `ResourceLimitExceeded` | A | Explained | ResourceLimitExceeded (Confirmed) |
| s9 | `FootprintEntryMissing` | C | Explained | ContractDefinedError (Confirmed), FootprintEntryMissing (Confirmed) |
| s10 | `INDETERMINATE` | C | InsufficientEvidence | — |
| s11 | `INDETERMINATE` | C | Explained | FootprintEntryMissing (Likely) |
| s12 | `AMBIGUOUS` {FootprintEntryMissing, InvalidAuthorizationEntry} | C | Explained | InvalidAuthorizationEntry (Likely) |
| s13 | `AMBIGUOUS` {ContractDefinedError, ContractTrap} | B | Explained | ContractDefinedError (Confirmed) |
| s14 | `OUT_OF_TAXONOMY` | B | Unsupported | — |
| s15 | `OUT_OF_TAXONOMY` | C | Explained | FootprintEntryMissing (Possible) |

So N = {s1 … s9}, |N| = 9; S_ind = {s10, s11}; S_amb = {s12, s13};
S_oot = {s14, s15}. s8's tier A label has `reference_label_source: replay`
(construction labels occur only in `constructed-v1`). No sample has an
integrity error, so both treatments of §3.1 give the same figures.

### 13.2 Headline metrics (§3)

- correct: s1, s2, s3, s5, s8 → **5**
- answered: s1, s2, s3, s4, s5, s8, s9 → **7**
- answered but wrong: s4, s9 → **2**
- abstained: s6 (InsufficientEvidence), s7 (Unsupported) → **2**

| Metric | Value |
|---|---|
| Top-1 accuracy | 5 / 9 (0.556) |
| Coverage | 7 / 9 (0.778) |
| Selective accuracy | 5 / 7 (0.714), shown beside coverage 7 / 9 (0.778) |
| Wrong-answer rate | 2 / 9 (0.222) |
| Unknown rate | 2 / 9 (0.222) |
| Insufficient-evidence rate | 1 / 9 (0.111) |
| Unsupported rate | 1 / 9 (0.111) |

Identity check: 5 + 2 + 2 = 9 ✓.

Note how abstaining differs between the two accuracies: s6 and s7 lower top-1
accuracy but do not affect selective accuracy, which is why selective accuracy
never appears without coverage.

### 13.3 Ranking metrics (§4)

- Top-3 hits: s1, s2, s3, s4 (ContractDefinedError is second), s5, s8, s9
  (FootprintEntryMissing is second) → **top-3 accuracy 7 / 9 (0.778)**.
- Multi-candidate count: s4, s9 → **2**.
- Ties at the top: **s9** (Confirmed, Confirmed). Its top-1 prediction was decided
  by registration order, and was wrong.

### 13.4 Per-class metrics (§5)

| Class | Support | Recall | Precision | False-positive rate |
|---|---|---|---|---|
| `FootprintEntryMissing` | 3 | 2 / 3 (0.667) | 2 / 2 (1.000) | 0 / 6 (0.000) |
| `ContractDefinedError` | 2 | 1 / 2 (0.500) | 1 / 2 (0.500) — s9 predicted it wrongly | 1 / 7 (0.143) |
| `InvalidAuthorizationEntry` | 1 | 1 / 1 (1.000) | 1 / 2 (0.500) — s4 predicted it wrongly | 1 / 8 (0.125) |
| `ResourceLimitExceeded` | 1 | 1 / 1 (1.000) | 1 / 1 (1.000) | 0 / 8 (0.000) |
| `MissingAuthorizationEntry` | 1 | 0 / 1 (0.000) | n/a (never predicted) | 0 / 8 (0.000) |
| `ContractTrap` | 1 | 0 / 1 (0.000) | n/a (never predicted) | 0 / 8 (0.000) |

**In a real report none of these rates would be published**: every class has
support below 5, so each sample would be listed individually instead (§5).

### 13.5 Sentinel outcomes (§6)

| Sample | Reference label | Prediction | Outcome |
|---|---|---|---|
| s10 | INDETERMINATE | InsufficientEvidence | correct abstention |
| s11 | INDETERMINATE | FootprintEntryMissing (Likely) | **overclaim** |
| s12 | AMBIGUOUS {FEM, IAE} | InvalidAuthorizationEntry (Likely) | consistent |
| s13 | AMBIGUOUS {CDE, ContractTrap} | ContractDefinedError (Confirmed) | **overclaim** |
| s14 | OUT_OF_TAXONOMY | Unsupported | correct abstention |
| s15 | OUT_OF_TAXONOMY | FootprintEntryMissing (Possible) | **wrong answer** |

Overclaim count **2** (s11, s13); hedged-answer count **0** (s11 answered
`Likely`, not `Possible`); correct-abstention count **2** (s10, s14);
sentinel wrong-answer count **1** (s15).

### 13.6 Calibration (§7)

| Level | Answered in N | Correct | Observed | Wilson 95% | Target | Status |
|---|---|---|---|---|---|---|
| Confirmed | s1, s3, s8, s9 | s1, s3, s8 | 3 / 4 (0.750) | [0.301, 0.954] | 0.95 | insufficient support |
| Likely | s4, s5 | s5 | 1 / 2 (0.500) | [0.095, 0.905] | 0.70 | insufficient support |
| Possible | s2 | s2 | 1 / 1 (1.000) | [0.207, 1.000] | — | insufficient support |

Every level has `n_L < 20`, so no status against a target is given, even though
`Confirmed`'s interval happens to contain 0.95. Beside the table: overclaims on
sentinel samples, 2 (s11 `Likely`, s13 `Confirmed`).

### 13.7 Contract-error name accuracy (§8)

Suppose s3 and s4 each carry a verified `contract_error_name`. SDO resolved s3's
terminal error to the same name, and could not resolve s4's (spec unavailable):
name accuracy **1 / 1 (1.000)**, unresolved **1**. s4's top-1 prediction was
wrong, but that does not affect this metric.

### 13.8 Per-tier breakdown (§10)

| Tier | Samples in N | Top-1 accuracy | Coverage |
|---|---|---|---|
| A, construction (independent ground truth) | — | n/a | n/a |
| A, replay (replay-verified reference labels) | s8 | 1 / 1 (1.000) | 1 / 1 (1.000) |
| B (expert reference labels) | s3, s4, s5, s7 | 2 / 4 (0.500) | 3 / 4 (0.750) |
| C (expert reference labels) | s1, s2, s6, s9 | 2 / 4 (0.500) | 3 / 4 (0.750) |

The tiers and sources partition N: 0 + 1 + 4 + 4 = 9 ✓.

### 13.9 Labeller agreement (§9)

Six samples, labellers X and Y, before adjudication:

| Sample | X | Y |
|---|---|---|
| 1 | FootprintEntryMissing | FootprintEntryMissing |
| 2 | FootprintEntryMissing | ContractDefinedError |
| 3 | ContractDefinedError | ContractDefinedError |
| 4 | INDETERMINATE | INDETERMINATE |
| 5 | InvalidAuthorizationEntry | InvalidAuthorizationEntry |
| 6 | ContractDefinedError | INDETERMINATE |

- `p_o` = 4 / 6 = 0.6667
- Label counts (X, Y): FootprintEntryMissing (2, 1), ContractDefinedError
  (2, 2), INDETERMINATE (1, 2), InvalidAuthorizationEntry (1, 1)
- `p_e` = (2·1 + 2·2 + 1·2 + 1·1) / 36 = 9 / 36 = 0.2500
- **κ** = (0.6667 − 0.2500) / (1 − 0.2500) = **0.5556**

Samples 2 and 6 go to adjudication.

## 14. Misses

A **miss** is any of:

- a sample in N that is not `correct`, whether it answered wrongly or
  abstained;
- a sentinel sample whose outcome is **overclaim** or **wrong answer**;
- an integrity error (§3.1).

Hedged answers (§6) are not misses; they are listed separately.

**Every miss is published** in `misses.json` and in `report.md`, with:

| Field | Content |
|---|---|
| `transaction_hash` | The sample |
| `reference_label` | With `candidate_classes` or `description` where applicable |
| `reference_label_source`, `evidence_tier`, `label_certainty` | From the final label |
| `rationale`, `evidence` | From the final label, or the adjudication |
| `verdict` | SDO's verdict |
| `candidates` | Every candidate: class, confidence, rule id |
| `rule_reports` | Every rule's status and reason |
| `miss_type` | `wrong_class`, `abstention`, `overclaim`, `sentinel_wrong_answer`, `integrity_error` |
| `tie_at_top` | Whether §4's tie condition held |
| `triage` | `rule_defect`, `reference_label_error`, `out_of_scope` or `untriaged` |
| `post_hoc_revision` | If the reference label was revised after predictions: the new value and justification. Headline figures still use the locked label. |

A miss triaged as `rule_defect` is followed up by a new issue requiring a new
development fixture ([protocol.md §13.2](protocol.md#132-rules-do-not-see-evaluation-data)).
The sample itself is never used to change a rule.
