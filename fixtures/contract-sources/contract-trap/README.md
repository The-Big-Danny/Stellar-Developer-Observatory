# contract-trap

A minimal contract whose only purpose is to panic. `trigger()` calls
`unwrap()` on `None`. `ok()` succeeds and is used only to obtain a valid
footprint and fee (see [`../README.md`](../README.md) for the method).

- Build: `cargo build --target wasm32v1-none --release`
- Built wasm sha256 (captured fixture): `550e13906441ef91aadf8abff5819d3a296e717c0cde49de07b6233e446b230b`
- Testnet contract: `CC5JULCIO5LSKKGECT5TK2TXNZHWH5ZNMKIAXARABVJUA7BV22XLOTK7`
- Failing transaction: `1f020cba1c06bc09422c9223546e5da5c2327b6bf380eac50fb43e7f57eaa8ad`
- Fixture: [`fixtures/failed/soroban-trapped-testnet-22ev`](../../failed/soroban-trapped-testnet-22ev/README.md)
