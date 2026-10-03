# Contract sources for deliberately caused failures

Each subdirectory is a minimal Soroban contract, built and deployed to Stellar
**testnet** to cause one failure mode that does not occur naturally in the
mainnet corpus. The captured transaction is the fixture. These sources are its
provenance, not evidence on their own.

| Directory | Purpose | Fixture |
|---|---|---|
| [`contract-trap/`](contract-trap/) | A panic with no declared error (`unwrap()` on `None`) | `soroban-trapped-testnet-22ev` |
| [`contract-auth/`](contract-auth/) | A `require_auth()` whose authorization entry is removed | `soroban-auth-missing-testnet` |

Each directory is its own Cargo workspace, detached from the repository's, so
`soroban-sdk` never enters the analysis engine's dependency tree. Build output
(`target/`) and `Cargo.lock` are ignored by a local `.gitignore`.

## The method, and why it is not simulation

`stellar contract invoke` simulates a transaction before sending it, so a call
that always fails is refused before it reaches the ledger. To get a failure
**on-chain**, the envelope has to skip that check. The method used for both
fixtures:

1. Simulate a successful call to the same contract. This gives a valid footprint
   and fee, and the failing call takes the same resource profile because it stops
   before touching any ledger entry (or, for auth, before the authorization check).
2. Send that successful call, then fetch its envelope with `stellar tx fetch`.
3. Change only what the failing call needs: the function name, the sequence
   number (+1), and the authorization list if one must be removed. Clear the
   signatures, then sign once with the source account.
4. Submit with `stellar tx send`, with no simulation. The network executes it and
   records the result.

Step 4 is a deliberate departure from normal use. It is acceptable here only
because the goal is to record what the network does, not to submit a transaction
that does something useful.

The CLI used was `stellar` 28.1.0. Testnet RPC is `https://soroban-testnet.stellar.org`.
Capture the result with the project's own tool, `sdo-probe capture`, as described
in each fixture's README.
