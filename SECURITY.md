# Security policy

## Reporting a vulnerability

**Please do not open a public issue for a security problem.**

Report it privately through
[GitHub Security Advisories](https://github.com/The-Big-Danny/Stellar-Developer-Observatory/security/advisories/new).

Please include what the issue is, how to reproduce it, what an attacker could
achieve, and which version or commit you tested.

You can expect an acknowledgement within 7 days and an assessment within 14. If
a fix is needed we will agree a disclosure timeline with you and credit you in
the advisory unless you prefer otherwise.

This is currently a single-maintainer project in early development. Responses may
be slower than a funded project's — that is a reason to report privately rather
than publicly, not a reason to skip reporting.

## Supported versions

Pre-1.0 and under active development. Only the latest `main` receives fixes.
There are no supported released versions yet.

## Threat model

Understanding what this software actually does bounds what can go wrong.

### It parses untrusted input

Every transaction this project reads is **attacker-controlled data from a public
blockchain**. Anyone can submit a transaction crafted to be hostile to a parser.
This is the primary attack surface.

Mitigations in place:

- All XDR decoding goes through [`stellar-xdr`](https://crates.io/crates/stellar-xdr),
  the canonical implementation, rather than hand-rolled parsing.
- `unsafe` is **forbidden** workspace-wide (`unsafe_code = "forbid"`), so memory
  corruption is not reachable in our own code.
- Malformed input must produce an error, never a panic. There are tests for
  malformed base64, valid-base64-invalid-XDR, and missing fields.

Report as a vulnerability: any input that causes a **panic**, an unbounded
allocation, or a non-terminating loop.

> **Note on decode limits.** RPC-supplied XDR is decoded under explicit depth
> and length limits, defined in `crates/soroban-failure-rpc/src/xdr.rs`. The
> depth bound is an engineering safety limit chosen by this project. It is not a
> Stellar protocol maximum, and it has not been measured against the deepest
> legitimate data. The remaining work (configurable limits, and closing the
> acceptance criteria) is tracked in
> [issue #2](https://github.com/The-Big-Danny/Stellar-Developer-Observatory/issues/2).
> Treat a demonstrated resource exhaustion through any decode path as a valid
> report.

### It makes outbound network requests

`soroban-failure-rpc::client` contacts the RPC endpoint you point it at, and
`sdo-probe` does the same. Both default to a public mainnet endpoint.

- Endpoint URLs come from you. Point it somewhere you trust.
- A response is untrusted input, even from a trusted endpoint.
- Diagnostic events are **not part of consensus**. Two honest nodes may return
  different diagnostic output for the same transaction, and a malicious endpoint
  can return whatever it likes. Never treat a `Diagnosis` as proof of anything
  on-chain.

### It handles no secrets

This project **never** needs a secret key, a seed, a signing operation, or a
funded account. It only reads public ledger data.

**If any part of this software ever asks you for a secret key, that is an
attack.** Report it.

### No secrets in fixtures

Fixtures are recorded RPC responses. `getTransaction` returns public keys and
signatures only, so a fixture has no legitimate reason to contain a secret.

This is enforced, not merely requested: the `no_fixture_contains_a_secret`
integration test rejects Stellar secret seeds (`S…`, 56 characters) and
credential-shaped JSON fields. The check is deliberately strict so that it does
not produce false positives against base64 blobs and train people to ignore it.

If you believe a committed fixture contains sensitive data, report it privately
rather than opening a public issue — a public issue would point everyone at it.

## Out of scope

- Vulnerabilities in Stellar itself — report those to the
  [Stellar Development Foundation](https://stellar.org/security).
- Vulnerabilities in third-party RPC providers — report those to the provider.
- The fact that a `Diagnosis` may be wrong. This tool produces **ranked
  candidate explanations with confidence levels**, not verdicts. An incorrect
  diagnosis is a correctness bug — please file it publicly, it is not a security
  issue.
