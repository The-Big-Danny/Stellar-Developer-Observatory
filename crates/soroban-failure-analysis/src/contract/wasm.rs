//! Minimal WebAssembly custom-section reader.
//!
//! A Soroban contract's interface — including its error enums — is embedded in
//! its WASM as a custom section named `contractspecv0`, holding a stream of
//! XDR-encoded `ScSpecEntry` values. Finding that section needs only the
//! top-level WASM framing, which is small and fully specified:
//!
//! ```text
//! magic "\0asm" | version 1 (u32 LE) | section*
//! section = id (u8) | size (LEB128 u32) | payload[size]
//! custom  = id 0, payload = name_len (LEB128) | name | content
//! ```
//!
//! This is hand-rolled rather than pulled from a WASM parsing crate because
//! only that framing is needed. Contract WASM is **attacker-controlled** —
//! anyone can deploy a contract — so every read is bounds-checked, nothing
//! indexes without checking, and malformed input returns an error.
//! The XDR inside the section is decoded by `stellar-xdr`, not here.

use core::fmt;

/// The WASM could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WasmError {
    /// Missing the `\0asm` magic number.
    NotWasm,
    /// A WASM binary version other than 1.
    UnsupportedVersion(u32),
    /// The input ended inside a section header or payload.
    Truncated,
    /// A LEB128 integer was longer than 5 bytes or overflowed a `u32`.
    BadLeb128,
}

impl fmt::Display for WasmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotWasm => f.write_str("not a WebAssembly module (missing \\0asm magic)"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported WebAssembly version {v}"),
            Self::Truncated => f.write_str("WebAssembly module is truncated"),
            Self::BadLeb128 => f.write_str("malformed LEB128 integer in WebAssembly module"),
        }
    }
}

impl std::error::Error for WasmError {}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], WasmError> {
        let end = self.pos.checked_add(n).ok_or(WasmError::Truncated)?;
        let slice = self.bytes.get(self.pos..end).ok_or(WasmError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    fn byte(&mut self) -> Result<u8, WasmError> {
        Ok(self.take(1)?[0])
    }

    /// Unsigned LEB128, capped at the 5 bytes a `u32` can need.
    fn leb_u32(&mut self) -> Result<u32, WasmError> {
        let mut result: u32 = 0;
        for i in 0..5 {
            let b = self.byte()?;
            let low = u32::from(b & 0x7f);
            // The fifth byte may only contribute the top 4 bits.
            if i == 4 && low > 0x0f {
                return Err(WasmError::BadLeb128);
            }
            result |= low << (7 * i);
            if b & 0x80 == 0 {
                return Ok(result);
            }
        }
        Err(WasmError::BadLeb128)
    }

    fn done(&self) -> bool {
        self.pos >= self.bytes.len()
    }
}

/// Every custom section with the given name, in module order.
///
/// A linker may emit a section name more than once; callers that want the
/// complete content should concatenate the results.
pub fn custom_sections<'a>(wasm: &'a [u8], name: &str) -> Result<Vec<&'a [u8]>, WasmError> {
    let mut r = Reader {
        bytes: wasm,
        pos: 0,
    };

    if r.take(4).map_err(|_| WasmError::NotWasm)? != b"\0asm" {
        return Err(WasmError::NotWasm);
    }
    let v = r.take(4)?;
    let version = u32::from_le_bytes([v[0], v[1], v[2], v[3]]);
    if version != 1 {
        return Err(WasmError::UnsupportedVersion(version));
    }

    let mut found = Vec::new();
    while !r.done() {
        let id = r.byte()?;
        let size = r.leb_u32()? as usize;
        let payload = r.take(size)?;
        if id != 0 {
            continue;
        }
        let mut s = Reader {
            bytes: payload,
            pos: 0,
        };
        let name_len = s.leb_u32()? as usize;
        if s.take(name_len)? == name.as_bytes() {
            found.push(&payload[s.pos..]);
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leb(mut n: u32) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let b = (n & 0x7f) as u8;
            n >>= 7;
            if n == 0 {
                out.push(b);
                return out;
            }
            out.push(b | 0x80);
        }
    }

    fn custom(name: &str, content: &[u8]) -> Vec<u8> {
        let mut payload = leb(name.len() as u32);
        payload.extend_from_slice(name.as_bytes());
        payload.extend_from_slice(content);
        let mut out = vec![0];
        out.extend(leb(payload.len() as u32));
        out.extend(payload);
        out
    }

    fn module(sections: &[Vec<u8>]) -> Vec<u8> {
        let mut m = b"\0asm\x01\0\0\0".to_vec();
        for s in sections {
            m.extend_from_slice(s);
        }
        m
    }

    #[test]
    fn finds_a_named_custom_section() {
        let m = module(&[custom("other", b"x"), custom("contractspecv0", b"spec")]);
        assert_eq!(
            custom_sections(&m, "contractspecv0").unwrap(),
            vec![&b"spec"[..]]
        );
    }

    #[test]
    fn returns_every_section_with_the_name() {
        let m = module(&[custom("s", b"a"), custom("s", b"b")]);
        assert_eq!(
            custom_sections(&m, "s").unwrap(),
            vec![&b"a"[..], &b"b"[..]]
        );
    }

    #[test]
    fn skips_non_custom_sections() {
        let mut m = module(&[]);
        m.extend_from_slice(&[1, 3, 0xaa, 0xbb, 0xcc]); // type section, 3 bytes
        m.extend(custom("s", b"ok"));
        assert_eq!(custom_sections(&m, "s").unwrap(), vec![&b"ok"[..]]);
    }

    #[test]
    fn absent_section_is_an_empty_list_not_an_error() {
        assert!(custom_sections(&module(&[]), "s").unwrap().is_empty());
    }

    #[test]
    fn rejects_non_wasm() {
        assert_eq!(custom_sections(b"not wasm", "s"), Err(WasmError::NotWasm));
        assert_eq!(custom_sections(b"", "s"), Err(WasmError::NotWasm));
    }

    #[test]
    fn rejects_other_versions() {
        assert_eq!(
            custom_sections(b"\0asm\x02\0\0\0", "s"),
            Err(WasmError::UnsupportedVersion(2))
        );
    }

    #[test]
    fn a_section_claiming_more_bytes_than_exist_is_truncated_not_a_panic() {
        let mut m = module(&[]);
        m.extend_from_slice(&[0, 0xff, 0xff, 0xff, 0xff, 0x0f]); // size u32::MAX
        assert_eq!(custom_sections(&m, "s"), Err(WasmError::Truncated));
    }

    #[test]
    fn overlong_leb128_is_rejected() {
        let mut m = module(&[]);
        m.extend_from_slice(&[0, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01]);
        assert_eq!(custom_sections(&m, "s"), Err(WasmError::BadLeb128));
    }

    #[test]
    fn name_length_overrunning_the_payload_is_truncated() {
        let mut m = module(&[]);
        m.extend_from_slice(&[0, 2, 50, b'x']); // payload says name is 50 bytes
        assert_eq!(custom_sections(&m, "s"), Err(WasmError::Truncated));
    }
}
