//! Bounded XDR decoding for data fetched from an RPC endpoint.
//!
//! RPC responses are third-party input. Decoding them with `Limits::none()`
//! lets a hostile or buggy endpoint make the decoder recurse without bound or
//! read an arbitrary amount of memory. Every decode of RPC-supplied XDR goes
//! through [`limits_for_base64`] instead.
//!
//! The depth bound is an engineering safety limit chosen by this project. It is
//! not a Stellar protocol maximum, and it has not been measured against the
//! deepest legitimate transaction or ledger entry. The length bound cannot
//! reject valid input, since it is never smaller than the decoded bytes.

use stellar_xdr::Limits;

/// Maximum XDR nesting depth accepted for transactions, results, metadata and
/// ledger entries.
///
/// This is an engineering safety bound, **not a Stellar protocol maximum**. It
/// stops a hostile or buggy endpoint from driving the decoder into unbounded
/// recursion. It has not been derived from the protocol, and the deepest
/// legitimate value has not been measured. Lowering it could reject real data.
///
/// Stack use: decoding at this depth was exercised on a 2 MB test thread in a
/// debug build. It has not been measured on a 1 MB main thread (the Windows
/// default), so that remains an open follow-up rather than a verified property.
/// The spec decoder uses a tighter bound, `SPEC_DECODE_DEPTH` (64, in
/// soroban-failure-analysis), because its input is WASM rather than RPC data.
pub const XDR_DECODE_DEPTH: u32 = 500;

/// Decode limits for raw XDR bytes from an RPC response or a history archive.
///
/// The length bound is the byte count itself.
pub fn limits_for_bytes(bytes: &[u8]) -> Limits {
    Limits {
        depth: XDR_DECODE_DEPTH,
        len: bytes.len(),
    }
}

/// Decode limits for a base64 XDR string from an RPC response.
///
/// The length bound is the base64 text's own length. Base64 expands data by
/// 4/3, so the decoded byte count can never exceed the text length, and no
/// legitimate input is rejected by it.
pub fn limits_for_base64(encoded: &str) -> Limits {
    Limits {
        depth: XDR_DECODE_DEPTH,
        len: encoded.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_xdr::{LedgerKey, ReadXdr, WriteXdr};

    #[test]
    fn limits_admit_every_legitimate_encoding_of_their_own_length() {
        // Any value's base64 encoding, decoded with limits derived from that
        // encoding, must round-trip: the bound never rejects real data.
        let key = LedgerKey::Account(stellar_xdr::LedgerKeyAccount {
            account_id: stellar_xdr::AccountId(stellar_xdr::PublicKey::PublicKeyTypeEd25519(
                stellar_xdr::Uint256([7; 32]),
            )),
        });
        let encoded = key.to_xdr_base64(Limits::none()).unwrap();
        assert_eq!(
            LedgerKey::from_xdr_base64(&encoded, limits_for_base64(&encoded)).unwrap(),
            key
        );
    }

    #[test]
    fn a_length_bound_rejects_a_value_larger_than_the_input_claims() {
        let key = LedgerKey::Account(stellar_xdr::LedgerKeyAccount {
            account_id: stellar_xdr::AccountId(stellar_xdr::PublicKey::PublicKeyTypeEd25519(
                stellar_xdr::Uint256([7; 32]),
            )),
        });
        let encoded = key.to_xdr_base64(Limits::none()).unwrap();
        let too_small = Limits {
            depth: XDR_DECODE_DEPTH,
            len: 4,
        };
        assert!(LedgerKey::from_xdr_base64(&encoded, too_small).is_err());
    }

    /// A value nested `levels` deep: each level wraps the previous one in a vec.
    fn nested_vec(levels: usize) -> stellar_xdr::ScVal {
        let mut v = stellar_xdr::ScVal::U32(1);
        for _ in 0..levels {
            v = stellar_xdr::ScVal::Vec(Some(vec![v].try_into().unwrap()));
        }
        v
    }

    #[test]
    fn a_value_nested_beyond_the_bound_is_rejected_not_recursed_into() {
        // Hostile nesting must fail as an error, never exhaust the stack. The
        // payload is built on a large-stack thread, because the recursive
        // encoder is not what this test measures. Decoding runs on the test
        // thread's default stack, which is the condition production sees.
        let deep = std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                nested_vec(XDR_DECODE_DEPTH as usize + 100)
                    .to_xdr_base64(Limits::none())
                    .unwrap()
            })
            .unwrap()
            .join()
            .unwrap();
        assert!(stellar_xdr::ScVal::from_xdr_base64(&deep, limits_for_base64(&deep)).is_err());
    }

    #[test]
    fn realistic_nesting_is_still_decoded() {
        let shallow = nested_vec(100).to_xdr_base64(Limits::none()).unwrap();
        assert_eq!(
            stellar_xdr::ScVal::from_xdr_base64(&shallow, limits_for_base64(&shallow)).unwrap(),
            nested_vec(100)
        );
    }

    #[test]
    fn malformed_base64_is_an_error_not_a_panic() {
        for garbage in ["", "!!!not base64!!!", "AAAA"] {
            assert!(
                stellar_xdr::ScVal::from_xdr_base64(garbage, limits_for_base64(garbage)).is_err(),
                "{garbage:?} must be rejected"
            );
        }
    }
}
