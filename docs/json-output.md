# `sdo explain --json`

Machine-readable output for `sdo explain`, so a script, CI check, or explorer
can consume a diagnosis without parsing the human-readable report.

## Invocation

```
sdo explain --fixture <dir> [--contracts <dir>] --json
sdo explain <tx-hash> [--rpc <endpoint>] --json
```

`--json` is an output-mode flag only. It changes nothing about how the
transaction is fetched, decoded, or analysed — see `docs/architecture/rules.md`
for that. Without `--json`, output is the existing human-readable report,
unchanged.

The JSON document is written to stdout as a single pretty-printed object,
terminated by a trailing newline. Nothing else is written to stdout in
`--json` mode. Exit codes are unchanged: `0` on success (including a
successfully produced "unknown" or "unsupported" diagnosis), non-zero only
when the transaction could not be fetched or decoded at all.

## What it is

`--json` serialises the same [`Diagnosis`][diagnosis] the human report is
built from — not a second representation of it. There is one analysis engine
(`crates/soroban-failure-analysis`); this flag only changes how its output is
printed. The one thing it adds on top of `Diagnosis`'s own fields is
`verdict`, which today is computed on demand by `Diagnosis::verdict()` rather
than stored.

[diagnosis]: ../crates/soroban-failure-analysis/src/diagnosis.rs

## Schema

The document is one flat JSON object:

| Field | Type | Meaning |
|---|---|---|
| `verdict` | [Verdict](#verdict) | The overall answer: explained, unknown, unsupported, or not a failure |
| `transaction_hash` | string \| `null` | Hex transaction hash, when known |
| `stage` | string \| `null` | Where execution stopped (`FailureStage`); `null` means the transaction succeeded |
| `candidate_causes` | array of [CandidateCause](#candidatecause) | Ranked most-plausible first; empty if none matched |
| `contract_errors` | array of [ContractErrorReport](#contracterrorreport) | Every `Error(Contract, #N)` seen in diagnostic events, terminal first |
| `rule_reports` | array of [RuleReport](#rulereport) | Every evaluated rule's outcome, including ones that did not match |
| `limitations` | array of string | Facts the engine could not establish, stated plainly |
| `rules_evaluated` | number | Count of rules evaluated to produce this diagnosis |

All enum-like string fields use `SCREAMING_SNAKE_CASE` (for example
`CONTRACT_EXECUTION`, `CONFIRMED`). This matches `FailureStage`, `CauseClass`,
and `Confidence`'s existing Rust naming, mapped mechanically by serde —
nothing is renamed by hand.

### `Verdict`

An adjacently tagged object: `{"kind": ..., "confidence": ...}`. The
`confidence` key is present only for `EXPLAINED`.

| `kind` | `confidence` present? | Meaning |
|---|---|---|
| `NOT_A_FAILURE` | no | The transaction succeeded |
| `EXPLAINED` | yes | At least one rule produced a cause; `confidence` is that top cause's confidence |
| `INSUFFICIENT_EVIDENCE` | no | Some rule could apply, but none found enough evidence — the honest answer is "unknown" |
| `UNSUPPORTED` | no | No implemented rule concerns this kind of failure |

A consumer that only cares whether a usable answer exists should check for
`kind == "EXPLAINED"`. `INSUFFICIENT_EVIDENCE` and `UNSUPPORTED` are
deliberately distinct — the first means "we looked and could not tell", the
second "we have no rule for this" — but both mean **no cause is being
claimed**. Treat them the same as "no cause" unless the distinction itself is
useful to you (for example, to decide whether retrying with
`--contracts`/diagnostic events present might help — that only applies to
`INSUFFICIENT_EVIDENCE`).

### `CandidateCause`

| Field | Type | Meaning |
|---|---|---|
| `class` | string | The cause class, e.g. `CONTRACT_DEFINED_ERROR` |
| `confidence` | string | `POSSIBLE`, `LIKELY`, or `CONFIRMED` |
| `summary` | string | One-sentence human summary of the claim |
| `evidence` | array of `{source, observation}` | Pointers into the transaction supporting the claim; `source` is a tagged object identifying where (e.g. `{"source": "diagnostic_event", "index": 3}`) |
| `remediation` | string \| `null` | What to check or change next, when known |
| `rule_id` | string | The rule that produced this candidate, e.g. `contract_defined_error` |

See `docs/architecture/rules.md` for what each `rule_id` means and what
confidence table it uses.

### `ContractErrorReport`

| Field | Type | Meaning |
|---|---|---|
| `code` | number | The code in `Error(Contract, code)` |
| `terminal` | boolean | Whether this is the error the invocation as a whole failed with |
| `event_indexes` | array of number | Diagnostic event indexes carrying this error value |
| `identification` | tagged object | Which contract raised it — `{"kind": "unique", ...}`, `{"kind": "ambiguous", ...}`, or `{"kind": "unidentified"}` |
| `resolution` | tagged object | Whether a name could be found — see below |

`resolution.kind` is one of `resolved`, `code_not_in_spec`,
`ambiguous_in_spec`, `spec_unavailable`, `spec_version_mismatch`,
`contract_not_identified`, or `not_applicable`. Only `resolved` carries a
name (`enum_name`, `case_name`, `doc`). A consumer must not infer a name from
any other `kind` — that is exactly the guess this project refuses to make.

**WASM hashes are lowercase hex strings**, not arrays of numbers. This
applies to `provenance.wasm_hash` (inside a `resolved` or `code_not_in_spec`
resolution's `matches_footprint` provenance) and to
`resolution.spec_wasm_hash` (inside `spec_version_mismatch`). Contract
addresses (`ContractId`, e.g. `"CBGSBK...KKY3"`) were already correct before
this flag existed — they serialise through `stellar-xdr`'s own strkey
`Display` impl.

### `RuleReport`

| Field | Type | Meaning |
|---|---|---|
| `rule_id` | string | The rule's stable identifier |
| `status` | string | `MATCHED`, `NO_EVIDENCE`, or `NOT_APPLICABLE` |
| `reason` | string \| `null` | Why it did not match; `null` when it matched |

## Confidence, in one sentence

`CONFIRMED` means the protocol result code or the host's own diagnostic data
states the cause and nothing in the transaction's own data contradicts it;
`LIKELY` means the host states it but nothing independently corroborates it;
`POSSIBLE` means the evidence is consistent with the cause but does not rule
out others or did not end the invocation. See
`docs/architecture/rules.md#confidence` for the full table.

## Handling `INSUFFICIENT_EVIDENCE` and `UNSUPPORTED`

Neither is an error. Both are legitimate, honest answers from an engine that
would rather say "I don't know" than guess — see
`crates/soroban-failure-analysis/src/diagnosis.rs`. A consumer should:

- Treat them as "no cause determined", not as a tool failure. The exit code
  is `0`.
- Surface `limitations` to the end user when present — they explain *why*
  (most commonly, the RPC node did not return diagnostic events).
- Not retry automatically. More network calls will not turn `UNSUPPORTED`
  into an answer; there is no rule for that failure category yet.

## Example

`sdo explain --fixture fixtures/failed/soroban-auth-signature-expired --json`:

```json
{
  "verdict": {
    "kind": "EXPLAINED",
    "confidence": "CONFIRMED"
  },
  "transaction_hash": "1a24d6811454317f3337c0f66463d2b2df5b69632cdd0c0f8bdf728e3a0d81f1",
  "stage": "CONTRACT_EXECUTION",
  "candidate_causes": [
    {
      "class": "INVALID_AUTHORIZATION_ENTRY",
      "confidence": "CONFIRMED",
      "summary": "The authorization entry for CASSU3MWIEQV755RQN2G5UHM6ICPVY6YPYPWIUKGXL44S3UPDASQL6KC carried a signature that had expired (valid until ledger 64392366, checked at ledger 64392368).",
      "evidence": [
        {
          "source": { "source": "diagnostic_event", "index": 3 },
          "observation": "the host raised Error(Auth, InvalidInput) for CASSU3MWIEQV755RQN2G5UHM6ICPVY6YPYPWIUKGXL44S3UPDASQL6KC: \"signature has expired\""
        },
        {
          "source": { "source": "authorization_entry", "index": 0 },
          "observation": "authorization entry 0 carries credentials for that address (nonce 4755168772314208784, signature valid until ledger 64392366)"
        }
      ],
      "remediation": "Sign a new authorization for CASSU3MWIEQV755RQN2G5UHM6ICPVY6YPYPWIUKGXL44S3UPDASQL6KC with a signature expiration ledger that leaves enough margin for the time between signing and inclusion, then resubmit.",
      "rule_id": "invalid_authorization_entry"
    }
  ],
  "contract_errors": [],
  "rule_reports": [
    {
      "rule_id": "contract_defined_error",
      "status": "NOT_APPLICABLE",
      "reason": "the invocation ended with Error(Auth, InvalidInput), a host error rather than a contract-defined one"
    },
    { "rule_id": "invalid_authorization_entry", "status": "MATCHED", "reason": null }
  ],
  "limitations": [],
  "rules_evaluated": 6
}
```

(Two of six `rule_reports` entries are shown above for brevity; a real
invocation lists all of them.)

A contract-defined-error fixture's `wasm_hash` field, for contrast:

```json
"provenance": {
  "kind": "matches_footprint",
  "wasm_hash": "70fe44694c9fe6b0abc69a6da4858fc2aaba04fa10492a466a1d426d04ca8560"
}
```

## Stability

This schema is new and has not shipped in a release yet. Before M6 closes it
is expected to be stable: field names and enum tag values will not be
renamed or removed without a documented migration, because M7 ecosystem
integration is meant to build on it. Expect only additive changes (new
optional fields, new enum variants under `#[non_exhaustive]` types such as
`FailureStage`, `CauseClass`, and `EvidenceSource`) going forward. A consumer
should therefore:

- Deserialise unknown enum variants permissively (don't `match` exhaustively
  without a fallback arm) — `CauseClass` and `EvidenceSource` are
  `#[non_exhaustive]` on the Rust side for exactly this reason.
- Not assume `candidate_causes` has at most one entry; ranking order is
  stable (most-plausible first) but the list can grow if evidence ever
  supports more than one explanation.
