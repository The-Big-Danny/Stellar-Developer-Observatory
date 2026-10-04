# contract-auth

A minimal contract whose `guarded(who)` calls `who.require_auth()`. The
fixture is captured by invoking it for a second identity, then removing that
identity's authorization entry from the envelope before submission (see
[`../README.md`](../README.md) for the method).

- Build: `cargo build --target wasm32v1-none --release`
- Built wasm sha256 (captured fixture): `0560ddba80bb8991125b901dfbcc6cc25a2933b8ab39e1ed2e400bbe9fa92537`
- Testnet contract: `CCLS3NCTSKVP3S3IPKE55KOHFC4IK757GJDFEDB46O2BPTA7WZWW4GGN`
- Authorized (successful) call: `7539470334cc5967341fb6b71d4fde436c29a359c4c60c25a2a368113eb61c34`
- Failing transaction (auth entry removed): `6c3a1a7e05a3b82de8b0082082ebe50dba742d5829428f8bebb86795b81b62e9`
- Fixture: [`fixtures/failed/soroban-auth-missing-testnet`](../../failed/soroban-auth-missing-testnet/README.md)
