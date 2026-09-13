# Fixtures

Real, recorded Stellar RPC responses. This corpus is what lets the entire test
suite run **offline** — no endpoint, no API key, no funded account, no node.

## Layout

fixtures/
└── failed/
    └── <name>/
        ├── rpc-response.json   the `result` member of getTransaction, verbatim
        ├── probe.json          what sdo-probe observed at capture time
        ├── metadata.json       validated machine-readable fixture metadata
        └── README.md           provenance and why this fixture is useful

## `metadata.json` schema

Each fixture must contain a `metadata.json` file with the following fields:

| Field | Type | Description |
|---|---|---|
| `transaction_hash` | string | Stellar transaction hash recorded by the fixture |
| `network` | string | Stellar network passphrase |
| `ledger` | integer | Ledger containing the recorded transaction |
| `captured_at` | string | Time the fixture was captured, in RFC 3339 format |
| `rpc_provider` | string | RPC endpoint used to capture the fixture |
| `failure_category` | string | Human-readable category for the failure |
| `fee_bumped` | boolean | Whether the transaction used a fee-bump |
| `diagnostic_event_count` | integer | Number of diagnostic events recorded by the probe |
| `purpose` | string | Human-readable explanation of why the fixture is useful |

The metadata is machine-readable and validated by CI. The fixture `README.md` remains the human-readable explanation of the fixture and its purpose.

## The four rules

1. **Fixtures are real.** Recorded from an actual network. Never hand-written,
   never edited to make a test pass. If you need a failure mode that does not
   exist here, *cause* one on testnet and capture it.
2. **Fixtures are verbatim.** Stored exactly as returned. Reshaping a response
   would turn our tests into a test of our own recorder.
3. **Fixtures are permanent.** Mainnet RPC retains about 7 days (120,960
   ledgers). Once that window passes, the committed file is the only copy in
   existence. Deleting one destroys evidence that cannot be recovered.
4. **Fixtures never contain secrets.** `getTransaction` returns only public keys
   and signatures, so there is no legitimate reason for a secret to appear. The
   `no_fixture_contains_a_secret` test enforces this.

## Current corpus

| Fixture | Category | Diagnostic events | Why it is here |
|---|---|---|---|
| `soroban-trapped-feebump-24ev` | `ContractTrap` | 24 | Baseline; the dominant mainnet failure shape |
| `soroban-trapped-feebump-49ev` | `ContractTrap` | 49 | Same class, different event-list size and ledger region |
| `soroban-trapped-feebump-49ev-alt` | `ContractTrap` | 49 | A third ledger, so a rule cannot pass by memorising one transaction |
| `classic-failed-no-diagnostics` | Classic (non-Soroban) | 0 | **Negative control.** Any rule that fires here is wrong |

### ⚠️ This corpus is not yet good enough

Every Soroban fixture is the same failure category, almost certainly the same
arbitrage bots failing repeatedly. **Five of the six planned rule categories have
no fixture at all**: auth, footprint, archival, resource limit, resource fee.

This is the project's biggest open risk and it blocks milestone M4. See
[the taxonomy](../docs/research/failure-taxonomy.md) and
[ROADMAP.md](../ROADMAP.md).

**Contributing a fixture for a missing category is one of the most valuable
things you can do here right now.**

## Capturing one

```bash
# Find candidates
cargo run -p sdo-probe -- scan --want 5

# Record one
cargo run -p sdo-probe -- capture \
    --tx <TRANSACTION_HASH> \
    --out fixtures/failed/<short-descriptive-name> \
    --failure-category <CATEGORY> \
    --purpose "<WHY THIS FIXTURE IS USEFUL>"
```

Then write the fixture's `README.md`, copying the shape of an existing one:
transaction hash, network, ledger, failure category, RPC provider used, fields
available, and why the fixture is useful.

For a category that does not occur naturally, produce it deliberately on testnet
— deploy a contract, then invoke it with the auth entry omitted, the footprint
truncated, resources under-declared, and so on — and capture each result.

## Using one

```rust
use soroban_failure_rpc::fixture;
use soroban_failure_analysis::analyze;

let decoded = fixture::load("fixtures/failed/soroban-trapped-feebump-49ev")?;
let diagnosis = analyze(&decoded.input);
```

From the CLI:

```bash
cargo run -p sdo -- explain --fixture fixtures/failed/soroban-trapped-feebump-49ev
```

## What guards the corpus

`crates/sdo/tests/fixtures.rs` asserts that every fixture has its required files,
decodes, records a failed transaction, analyses reproducibly, agrees with its own
`probe.json`, and contains no secret. It also fails if the corpus is empty, so
the other tests can never pass vacuously.
