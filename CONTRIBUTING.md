# Contributing

Thank you for considering a contribution. This project is built so that you
should not need to ask the maintainer how anything works.

If something here is unclear or wrong, **that is a bug**. Open an issue saying
so. Documentation gaps are real issues.

## Set up

Requires **Rust 1.88 or newer** (`rustup update stable`).

```bash
git clone https://github.com/The-Big-Danny/Stellar-Developer-Observatory
cd Stellar-Developer-Observatory
cargo test --workspace
```

That is the whole setup. You need **no RPC endpoint, no API key, no funded
account and no node.** If a test ever requires network access, that is a bug.

## Before you open a pull request

Run what CI runs. These are the same checks, so a green local run means a green
CI run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --no-deps --all-features   # with RUSTDOCFLAGS="-D warnings"
bash scripts/check-analysis-purity.sh
```

On Windows, run the last command from Git Bash, or any Unix-style shell.

`cargo-deny` runs in CI too. To run it locally:

```bash
cargo install cargo-deny --locked --version 0.20.2
cargo deny --locked check
```

## Understanding the workspace

| Crate | Responsibility | May it do I/O? |
|---|---|---|
| `crates/soroban-failure-analysis` | The analysis engine: transaction model, contract error resolution, rules, verdicts | **Never** |
| `crates/soroban-failure-rpc` | RPC access, XDR decoding, fixture loading, contract spec fetching | Only in `client.rs` (network) and `fixture.rs` (files) |
| `crates/sdo` | The `sdo` command line tool, and the integration tests in `crates/sdo/tests/` | Presentation only |
| `tools/rpc-probe` | `sdo-probe`, the measurement and fixture-capture tool | Yes. It exists to measure endpoints. |

Read [docs/architecture](docs/architecture/README.md) for the pipeline. In short:
RPC data is decoded into an `AnalysisInput`, the engine analyses that input, and
the CLI prints the resulting `Diagnosis`.

### The one rule that matters

**`soroban-failure-analysis` performs no I/O.** No network, no filesystem, no
clock, no environment variables, no randomness, no global state.

This is what lets every rule be tested from a committed fixture with no network,
so you can contribute without running a node or spending a rate limit. It is
enforced in CI by `scripts/check-analysis-purity.sh`, which fails if the crate's
dependency tree contains a crate that is not on
[`.github/analysis-dependency-allowlist.txt`](.github/analysis-dependency-allowlist.txt).

If you need data the engine does not have, do not fetch it inside a rule. Add it
to `AnalysisInput` and populate it in `soroban-failure-rpc`. The section "If you
need data the engine does not have" in [purity.md](docs/architecture/purity.md)
explains the pattern.

## Fixtures

A **fixture** is a verbatim recording of a `getTransaction` RPC response, stored
under `fixtures/failed/<name>/`.

1. **Fixtures are real.** Recorded from an actual network. Never hand-written,
   never edited to make a test pass. If you need a failure that does not exist
   yet, cause it on testnet and capture it.
2. **Fixtures are verbatim.** The response is stored exactly as returned.
   Reshaping it would make our tests a test of our own recorder.
3. **Fixtures are permanent.** Mainnet RPC keeps about seven days of history.
   After that the committed file is the only copy. Deleting one destroys evidence.
4. **Fixtures never contain secrets.** `getTransaction` returns public keys and
   signatures only. The test `no_fixture_contains_a_secret` enforces this.

### Adding a fixture

Capture it:

```bash
cargo run -p sdo-probe -- capture \
    --tx <TRANSACTION_HASH> \
    --out fixtures/failed/<short-descriptive-name> \
    --failure-category <CATEGORY> \
    --purpose "<WHY THIS FIXTURE IS USEFUL>"
```

Then write `fixtures/failed/<name>/README.md`. Copy the shape of an existing
fixture README, such as `soroban-auth-missing-testnet`. It must state:

- the transaction hash, network, ledger and capture date
- the RPC provider used
- the failure category, and the result codes observed
- **how the failure was caused**, if it was deliberately caused on testnet. The
  contract source goes in `fixtures/contract-sources/<name>/`, and the README
  must say the failure is not a mainnet occurrence.
- **why the fixture is useful**: what it covers that existing fixtures do not

A fixture that repeats an existing failure mode adds little. A fixture for a
category we have never seen is very valuable. See
[the taxonomy](docs/research/failure-taxonomy.md) for the gaps.

## Adding a failure rule

Rules are the main unit of contribution. A new rule touches three things: the
rule file, its fixture, and its tests. Read one existing rule, such as
[`rules/missing_auth.rs`](crates/soroban-failure-analysis/src/rules/missing_auth.rs),
and [rules.md](docs/architecture/rules.md) before you write your own.

### 1. The rule

Create `crates/soroban-failure-analysis/src/rules/<your_rule>.rs`:

```rust
use crate::diagnosis::{CandidateCause, Confidence, Evidence, EvidenceSource};
use crate::model::DiagnosticAvailability;
use crate::rule::{FailureContext, Rule, RuleOutcome};
use crate::taxonomy::{CauseClass, FailureStage};

pub struct MyRule;

impl Rule for MyRule {
    fn id(&self) -> &'static str { "my_rule" }

    fn description(&self) -> &'static str {
        "One line: the failure this detects"
    }

    fn evaluate(&self, ctx: &FailureContext<'_>) -> RuleOutcome {
        // Say why the rule does not concern this transaction...
        if ctx.model.stage() != Some(FailureStage::ContractExecution) {
            return RuleOutcome::not_applicable("only applies to contract execution traps");
        }
        // ...and why it could, but cannot decide. Absence of evidence is not
        // evidence of absence.
        if ctx.model.diagnostics.availability == DiagnosticAvailability::NotEmitted {
            return RuleOutcome::no_evidence("diagnostic events were not available");
        }

        // ... look for the evidence; if it is not there, return no_evidence ...

        RuleOutcome::Match(CandidateCause {
            class: CauseClass::Undetermined, // your class
            confidence: Confidence::Likely,
            summary: "One sentence stating the claim.".into(),
            evidence: vec![Evidence::new(
                EvidenceSource::DiagnosticEvent { index: 0 },
                "what was observed, not what it means",
            )],
            remediation: Some("What to check or change next.".into()),
            rule_id: self.id().into(),
        })
    }
}
```

The snippet is a skeleton. It will not compile until you add a real
`CauseClass` variant and real evidence. Work from `ctx.model`, the canonical
`TransactionModel`, not from raw XDR. Then export the rule from `rules/mod.rs`,
register it in `RuleRegistry::builtin()` in `rule.rs`, and update the tripwire
test `builtin_registry_ships_exactly_the_m4_rules`, which lists every rule id.

### 2. A real fixture

A rule that interprets diagnostic events needs a **real** fixture. Its evidence
shape cannot be known otherwise, and it will not be merged without one. The one
exception is a rule whose only evidence is a protocol result code defined as that
exact cause. It may ship with synthetic tests if its documentation says it is
unvalidated.

A real fixture need not be a mainnet occurrence. If a failure does not happen
naturally, cause it on testnet and capture the result, as the `contract_trap` and
`missing_authorization_entry` fixtures were. Testnet evidence shows the shape of a
failure, not how often it occurs on mainnet, so it does not support a frequency claim.

`cargo run -p soroban-failure-rpc --example survey_failures -- --pages <N>`
samples mainnet and groups failed Soroban transactions by how they failed. The
mainnet authorization fixtures were found this way.

### 3. Tests: it fires, and it stays quiet

- Synthetic unit tests in the rule file for **every** confidence level and every
  `NotApplicable` and `NoEvidence` path.
- A real-fixture test in `crates/sdo/tests/classification.rs`.
- An assertion that the rule does not fire on other rules' fixtures, or on the
  negative control `fixtures/failed/classic-failed-no-diagnostics`.

### What makes a rule good

- **Return `NoEvidence` rather than guessing.** An honest "unknown" is correct. A
  confident wrong answer is the worst thing this project could ship.
- **Reserve `Confirmed` for corroborated claims.** The host stating a cause is
  `Likely`. The host stating it and the transaction's own data corroborating it
  is `Confirmed`.
- **Cite evidence.** Point at the diagnostic event or authorization entry index.
  Anything above `Confidence::Possible` must carry evidence.
- **State observations, not conclusions, in evidence.** Good: "authorization entry
  list is empty". Bad: "the developer forgot to sign".
- **Never depend on another rule.** Rules are independent by design.
- **Write remediation a developer can act on.** "Check the authorization entries
  for account G..." is useful. "Authorization failed" is not.

## Tests and documentation expectations

- **Tests** must assert behaviour worth protecting. Ten meaningful tests are better
  than a hundred that check getters. No test may require network access.
- **Documentation** changes ship in the same pull request as the behaviour they
  describe. If your change removes a documented limitation, remove the limitation
  too. Stale disclaimers are as bad as overclaiming.
- If you change the `Diagnosis` shape or the `--json` output, update
  [docs/json-output.md](docs/json-output.md).
- If you add a rule, update the rule table in [rules.md](docs/architecture/rules.md)
  and the table in the README.

## Coding standards

- `cargo fmt` and `clippy -D warnings` are enforced by CI.
- Public items need doc comments. The analysis and RPC crates warn on `missing_docs`.
- `unsafe` is forbidden across the workspace.
- Prefer small modules. If a file is getting long, it is probably two things.
- Explain *why* in comments, not *what*. The code says what.

## How CI validates a pull request

The workflow is [`.github/workflows/ci.yml`](.github/workflows/ci.yml). It runs on
every pull request and every push to `main`.

| Job | Checks |
|---|---|
| `fmt` | Formatting |
| `clippy` | Lints, with warnings as errors |
| `test` | The workspace test suite, on stable and on Rust 1.88 |
| `docs` | Rustdoc, with broken links as errors |
| `analysis-purity` | The analysis crate has no I/O-capable dependency |
| `deny` | Licences, banned crates and registry sources |

CI does not contact an RPC endpoint.

## Pull requests

1. Branch from `main`. One logical change per pull request.
2. Write commit messages in [conventional commits](https://www.conventionalcommits.org/)
   form: `feat:`, `fix:`, `docs:`, `test:`, `ci:`, `chore:`, `refactor:`.
3. Fill in the [pull request template](.github/pull_request_template.md). It
   includes the milestone the change belongs to, and the checklist.
4. Open the pull request as a draft if it is not ready. Drafts are welcome.

Asking a question on an issue before writing code is always fine, and usually
faster.

## Issues

Labels you will see:

| Label | Meaning |
|---|---|
| `good first issue` | Self-contained and clearly specified. No deep Soroban knowledge needed. |
| `help wanted` | The maintainer would particularly like outside help |
| `rust` / `soroban` / `xdr` / `rpc` | What you will be touching |
| `fixtures` | Involves the test corpus |
| `testing` / `documentation` | The kind of work |
| `blocked` | Waiting on an earlier milestone |

Every issue should state the problem, the expected behaviour, acceptance criteria,
the relevant files and the tests required. If one does not, ask. An
underspecified issue is the maintainer's mistake.

## Security

Do not open a public issue for a security problem. See [SECURITY.md](SECURITY.md).

## Code of conduct

By participating you agree to the [Code of Conduct](CODE_OF_CONDUCT.md).
