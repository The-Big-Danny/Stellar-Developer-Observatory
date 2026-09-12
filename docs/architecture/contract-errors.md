# Contract error resolution (M3)

A contract that raises an error only puts a number on chain:
`Error(Contract, #2)`. The name exists only in the contract's spec, which the
Soroban SDK embeds in the WASM. M3 turns the number into that name — and says
plainly when it cannot.

On the real 49-event mainnet fixture:

```
Error(Contract, #2)  →  NoHarvestablePails   [terminal]
    contract    CBGSBKYMYO6OMGHQXXNOBRGVUDFUDVC2XLC3SXON5R2SNXILR7XCKKY3 (only contract to raise it)
    resolution  from the contract spec (enum Error); WASM 70fe4469… matches the transaction footprint
    doc         Harvesting all pails results in 0 reward
Error(Contract, #9)  →  PailMissing   [raised, then caught or superseded]
    contract    CDL74RF5BLYR2YBLCCI7F5FB6TPSCLKEJUBSD2RSVWZ4YHF3VMFAIGWA (origin frame of a re-emitted error)
    resolution  from the contract spec (enum Errors); WASM db2c1429… matches the transaction footprint
```

## Pipeline, and where each part lives

```
                            PURE (soroban-failure-analysis)          I/O (soroban-failure-rpc)
                            ───────────────────────────────          ─────────────────────────
TransactionModel ─► contracts_needing_specs() ───────────────────────► fetch_specs(source, ids)
                                                                        │  getLedgerEntries(instance)
                                                                        │    → ContractExecutable::Wasm(hash)
                                                                        │  getLedgerEntries(code)
                                                                        │    → WASM bytes
                    ContractSpec::from_wasm() ◄─────────────────────────┘
                              │
            AnalysisInput.contract_specs  (data, not a connection)
                              │
                    resolve_contract_errors()  ─►  Diagnosis.contract_errors
```

The engine never fetches anything. It says which contracts it needs, the caller
fetches them, and the specs come back in as data. This is the purity rule from
[purity.md](purity.md) applied exactly as that document anticipated.

`ContractSource` is the one abstraction, and it exists because it protects a
real boundary: the same orchestration runs against the network (`RpcClient`) or
against recorded responses (`FixtureContractSource`). Both decode through the
same pure functions, so offline tests exercise real decoding.

## Three questions, each of which can fail

### 1. Which contract raised it?

A number means nothing without the contract whose spec defines it, and a
transaction can involve several contracts. Worse, a caller that catches or
propagates a callee's error **re-emits the callee's number from its own frame**.
In the real fixture, the harvester emits `#9` five times while reporting failed
`try_call`s — but `#9` is the *farm* contract's code. Naming it from the
harvester's spec would be confidently wrong.

`identify()` therefore:

1. collects every contract that emitted an `error` event with that exact value;
2. if there is exactly one, that is the contract (`SoleEmitter`);
3. otherwise, if exactly one of them emitted the host's origin message
   `"failing with contract error"` — emitted when a contract raises its *own*
   error — that is the contract (`OriginMarker`);
4. otherwise the result is `Ambiguous`, or `Unidentified` if nobody emitted it.

The origin marker is a host diagnostic string. It is not consensus and not a
stable API, so it only ever breaks a tie; it never identifies a contract alone.

### 2. Do we have the contract's spec?

`SpecAvailability` is `Available(ContractSpec)` or `Unavailable { reason }`. A
contract with no entry at all is reported as "no contract spec was supplied".
Reasons are passed through to the user unchanged.

Specs are read from every `contractspecv0` custom section in the WASM, which
holds a stream of XDR `ScSpecEntry` values. Each `#[contracterror]` enum is a
`UdtErrorEnumV0` entry.

### 3. Is the spec for the code that actually ran?

Contracts can be upgraded. A spec fetched today may describe code deployed
after the failure — and such an upgrade can renumber errors.

The transaction's footprint contains a `ContractCode` key for every WASM it
loaded. If the fetched spec's WASM hash is in that set, provenance is
`MatchesFootprint`. If the footprint names code but not this hash, the result is
`SpecVersionMismatch`, and **no name is reported**. If there is nothing to
check against, the name is reported with provenance `Unverified`.

The real fixtures matter here: both contract specs were captured *after* their
transactions, and the farm contract's instance was modified after the failure.
The footprint check proves the recorded specs still match what ran, and a test
asserts it.

## Outcomes

Every outcome is its own `ErrorResolution` variant, so "could not name it" can
never be mistaken for a name. `resolution.name()` is `Some` only for `Resolved`.

| Variant | Meaning |
|---|---|
| `Resolved` | Named from the spec, with provenance |
| `CodeNotInSpec` | Spec obtained; it declares no name for this code |
| `AmbiguousInSpec` | The spec declares the code under **different** names in two enums |
| `SpecUnavailable` | Contract identified; spec not obtained (reason attached) |
| `SpecVersionMismatch` | Spec is for WASM the transaction did not load |
| `ContractNotIdentified` | No single contract could be identified |
| `NotApplicable` | Not a contract-defined error — e.g. `Error(Storage, ExceededLimit)` |

## Deliberate limitations

- **Stellar Asset Contract errors are not named.** The SAC is built into the
  host and has no on-chain spec. Its codes are well defined in
  `soroban-env-host`, but this project only takes names from a contract's spec,
  so they are reported as `SpecUnavailable` with that reason.
- **No persistent cache.** `fetch_specs` fetches each distinct WASM once per
  call; nothing is stored between runs.
- **Archived contracts.** A contract whose instance or code has been archived is
  reported as not found.
- **Spec decoding is depth-limited** (`SPEC_DECODE_DEPTH = 64`). Contract WASM
  is attacker-deployable, and spec types are recursive; a hostile spec produces
  an error rather than exhausting the stack. Decode limits for the rest of the
  pipeline are tracked in issue #2.
- **The WASM section reader is hand-rolled.** Only the top-level WASM framing is
  needed, so it is about sixty lines instead of a new dependency. It is
  bounds-checked throughout and has malformed-input tests.

## Using it

```bash
# Online: fetches the transaction, then only the specs it needs
sdo explain <TX_HASH>

# Offline, from recorded fixtures
sdo explain --fixture fixtures/failed/soroban-trapped-feebump-49ev \
            --contracts fixtures/contracts
```

```rust
let model = TransactionModel::from_input(&input);
let specs = fetch_specs(&client, &contracts_needing_specs(&model));
let diagnosis = analyze(&input.with_contract_specs(specs));

for report in &diagnosis.contract_errors {
    match report.resolution.name() {
        Some(name) => println!("#{} = {name}", report.code),
        None => println!("#{} could not be named: {:?}", report.code, report.resolution),
    }
}
```
