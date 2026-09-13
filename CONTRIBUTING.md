# Contributing

Thank you for considering a contribution. This project is deliberately built so
that you should not need to ask the maintainer how anything works.

If something here is unclear or wrong, **that is a bug** — please open an issue
saying so. Documentation gaps are real issues.

## Quick start

Requires **Rust 1.88 or newer** (`rustup update stable`).

```bash
git clone https://github.com/The-Big-Danny/Stellar-Developer-Observatory
cd Stellar-Developer-Observatory
cargo test --workspace
```

That is the whole setup. **No RPC endpoint, no API key, no funded account, no
node.** If a test ever requires network access, that is a bug.

Before opening a pull request, run what CI runs:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Repository structure

| Crate | Responsibility | May it do I/O? |
|---|---|---|
| `crates/soroban-failure-analysis` | The analysis engine: taxonomy, rules, ranking | **Never** |
| `crates/soroban-failure-rpc` | Fetching, XDR decoding, fixture loading | Only in `client.rs` |
| `crates/sdo` | The CLI, and the integration tests | Presentation only |
| `tools/rpc-probe` | `sdo-probe`, the M0 research instrument | Yes — it exists to measure endpoints |

### The one rule that matters

**`soroban-failure-analysis` performs no I/O.** No network, no filesystem, no
clock, no environment variables, no randomness, no global state.

This is not stylistic. It is what lets every rule be tested from a committed
fixture with no network, which is what lets *you* contribute without running a
node or spending a rate limit. A pull request that adds an I/O-capable dependency
to that crate will be asked to move the code to `soroban-failure-rpc`.

## Fixture philosophy

A **fixture** is a verbatim recording of a `getTransaction` RPC response, stored
under `fixtures/failed/<name>/`.

1. **Fixtures are real.** Recorded from an actual network. Never hand-written,
   never edited to make a test pass. If you need a failure that does not exist
   yet, *cause* one on testnet and capture it.
2. **Fixtures are verbatim.** The response is stored exactly as returned.
   Reshaping it would make our tests a test of our own recorder.
3. **Fixtures are permanent.** Mainnet RPC retains about 7 days. Once the window
   passes, the committed file is the only copy. Deleting one destroys evidence.
4. **Fixtures never contain secrets.** `getTransaction` returns only public keys
   and signatures. A test (`no_fixture_contains_a_secret`) enforces this.

### Capturing a fixture

Run:

```bash
cargo run -p sdo-probe -- capture \
    --tx <TRANSACTION_HASH> \
    --out fixtures/failed/<short-descriptive-name> \
    --failure-category <CATEGORY> \
    --purpose "<WHY THIS FIXTURE IS USEFUL>"
```

Then write `fixtures/failed/<name>/README.md` documenting the transaction hash,
network, ledger, failure category, which RPC provider you used, which fields were
available, and — most importantly — **why this fixture is useful**. Copy the
shape of an existing one.

A fixture that duplicates the failure mode of an existing fixture adds little.
A fixture covering a category we have never seen is extremely valuable — see
[the taxonomy](docs/research/failure-taxonomy.md) for what is currently missing.

## Adding a failure rule

Rules are the unit of contribution here. Adding one should touch three things.
Six rules already exist in `crates/soroban-failure-analysis/src/rules/`; read
one, and [rules.md](docs/architecture/rules.md), before writing your own.

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

Work from `ctx.model` (the canonical `TransactionModel`), not raw XDR. Export
the rule from `rules/mod.rs`, register it in `RuleRegistry::builtin()`, and
update the tripwire test that lists rule ids.

### 2. A fixture exhibiting it

A rule that interprets diagnostic events needs a **real** fixture — its
evidence shape cannot be known otherwise, and it will not be merged without one.
The one exception is a rule whose only evidence is a protocol result code
defined as that exact cause; it may ship with synthetic tests if its docs say
plainly that it is unvalidated.

`cargo run -p soroban-failure-rpc --example survey_failures` samples mainnet
and buckets failed Soroban transactions by how they failed, which is how the
authorization fixtures were found.

### 3. Tests: it fires, and it stays quiet

- Synthetic unit tests in the rule file for **every** confidence level and every
  `NotApplicable` / `NoEvidence` path.
- A real-fixture test in `crates/sdo/tests/classification.rs`.
- Assert it does not fire on other rules' fixtures or on
  `fixtures/failed/classic-failed-no-diagnostics`, the negative control.

### What makes a rule good

- **Return `NoEvidence` rather than guessing.** An honest "unknown" is a correct
  answer. A confident wrong answer is the worst thing this project could ship.
- **Reserve `Confirmed` for corroborated claims.** The host stating a cause is
  `Likely`; the host stating it *and* the transaction's own data corroborating
  it is `Confirmed`.
- **Cite evidence.** Point at the specific diagnostic event or auth entry index.
  Anything above `Confidence::Possible` must carry evidence.
- **State observations, not conclusions, in evidence.** Good: "auth entry list is
  empty". Bad: "the developer forgot to sign".
- **Never depend on another rule.** Rules are independent by design.
- **Write remediation a developer can act on.** "Check the transaction's
  authorization entries for account G..." beats "authorization failed".

## Coding standards

- `cargo fmt` and `clippy -D warnings` are enforced by CI, not by review.
- Public items need doc comments — the analysis and RPC crates warn on
  `missing_docs`.
- `unsafe` is forbidden workspace-wide.
- Prefer small modules over large ones. If a file is getting long, it is probably
  two things.
- Explain *why* in comments, not *what*. The code says what.
- Tests should assert behaviour worth protecting. We would rather have 10
  meaningful tests than 100 that assert getters return what was set.

## Pull requests

- Branch from `main`. One logical change per PR.
- Write a commit message that explains the change, using
  [conventional commits](https://www.conventionalcommits.org/)
  (`feat:`, `fix:`, `docs:`, `test:`, `ci:`, `chore:`, `refactor:`).
- Fill in the PR template.
- If you change behaviour, update the docs in the same PR.
- If your change makes a documented limitation obsolete, remove it — stale
  disclaimers are as bad as overclaiming.

Draft PRs are welcome. Asking a question on an issue before writing code is
always fine and usually faster.

## Issues

Labels you will see:

| Label | Meaning |
|---|---|
| `good first issue` | Self-contained, clearly specified, no deep Soroban knowledge needed |
| `help wanted` | We would particularly like outside help |
| `rust` / `soroban` / `xdr` / `rpc` | What you will be touching |
| `fixtures` | Involves the test corpus |
| `testing` / `documentation` | Kind of work |
| `blocked` | Waiting on an earlier milestone |

Every issue should state the problem, the expected behaviour, acceptance
criteria, the relevant files, and what tests are required. If one does not, ask —
an underspecified issue is our mistake.

## Code of conduct

By participating you agree to the [Code of Conduct](CODE_OF_CONDUCT.md).

## Security

Do not open a public issue for a security problem. See [SECURITY.md](SECURITY.md).
