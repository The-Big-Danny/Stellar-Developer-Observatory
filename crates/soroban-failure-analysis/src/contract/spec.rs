//! Contract specifications: the error enums a contract declares.
//!
//! A contract that raises `Error(Contract, #3)` only ever puts the number `3`
//! on chain. The *name* — `Unauthorized`, `InsufficientBalance` — exists only
//! in the contract's spec, which the Soroban SDK embeds in the WASM as
//! `ScSpecEntry::UdtErrorEnumV0` entries in the `contractspecv0` custom
//! section (one per `#[contracterror]` enum).
//!
//! Names are only ever read from that spec. This module never maps a number to
//! a name any other way.

use core::fmt;
use std::io::Cursor;

use stellar_xdr::{Limited, Limits, ReadXdr, ScSpecEntry};

use super::wasm::{custom_sections, WasmError};

/// Name of the custom section carrying the spec.
pub const SPEC_SECTION: &str = "contractspecv0";

/// Maximum XDR nesting depth accepted when decoding a spec.
///
/// Spec type definitions are recursive (`Option<Vec<Map<…>>>`). Real specs nest
/// a handful of levels; the WASM is attacker-supplied, and unbounded depth
/// would let a hostile spec exhaust the stack. Exceeding this is an error, not
/// a crash. See also issue #2, which tracks limits for the rest of decoding.
pub const SPEC_DECODE_DEPTH: u32 = 64;

/// One case of a contract error enum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorCase {
    /// The case name, e.g. `NoHarvestablePails`.
    pub name: String,
    /// The value raised as `Error(Contract, value)`.
    pub value: u32,
    /// The developer's doc comment, possibly empty.
    pub doc: String,
}

/// One `#[contracterror]` enum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorEnum {
    /// The enum name, e.g. `Error`.
    pub name: String,
    /// The library it came from, when not the contract itself. Often empty.
    pub lib: String,
    /// The developer's doc comment, possibly empty.
    pub doc: String,
    /// The cases, in declaration order.
    pub cases: Vec<ErrorCase>,
}

/// The parts of a contract spec that error resolution needs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContractSpec {
    /// Hash of the WASM the spec was read from, when known.
    ///
    /// Lets resolution check the spec against the code the transaction actually
    /// loaded. A spec with no hash cannot be checked and is reported as
    /// unverified.
    pub wasm_hash: Option<[u8; 32]>,
    /// Every error enum the spec declares.
    pub error_enums: Vec<ErrorEnum>,
    /// Total number of spec entries read, of any kind.
    pub entry_count: usize,
}

/// A spec could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecError {
    /// The WASM framing was invalid.
    Wasm(WasmError),
    /// The WASM has no `contractspecv0` section — built without a spec, or
    /// stripped.
    NoSpecSection,
    /// The spec section held invalid XDR.
    Xdr(String),
}

impl fmt::Display for SpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wasm(e) => write!(f, "invalid contract WASM: {e}"),
            Self::NoSpecSection => write!(f, "contract WASM has no `{SPEC_SECTION}` section"),
            Self::Xdr(e) => write!(f, "contract spec is not valid XDR: {e}"),
        }
    }
}

impl std::error::Error for SpecError {}

/// The result of looking an error code up in a spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorLookup<'a> {
    /// Exactly one name is declared for the code.
    Found {
        /// The enum declaring it.
        error_enum: &'a ErrorEnum,
        /// The case.
        case: &'a ErrorCase,
    },
    /// No enum declares the code.
    NotFound,
    /// More than one enum declares the code under **different** names.
    ///
    /// The chain only carries the number, so there is no way to know which enum
    /// the contract meant. Picking one would be a guess.
    Conflicting(Vec<(&'a ErrorEnum, &'a ErrorCase)>),
}

impl ContractSpec {
    /// Read the spec from contract WASM.
    ///
    /// Every `contractspecv0` section is read and concatenated, since a linker
    /// may split it.
    pub fn from_wasm(wasm: &[u8]) -> Result<Self, SpecError> {
        let sections = custom_sections(wasm, SPEC_SECTION).map_err(SpecError::Wasm)?;
        if sections.is_empty() {
            return Err(SpecError::NoSpecSection);
        }
        let bytes: Vec<u8> = sections.concat();
        let limits = Limits {
            depth: SPEC_DECODE_DEPTH,
            len: bytes.len(),
        };
        let mut reader = Limited::new(Cursor::new(bytes), limits);
        let entries = ScSpecEntry::read_xdr_iter(&mut reader)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| SpecError::Xdr(e.to_string()))?;
        Ok(Self::from_entries(entries))
    }

    /// Build a spec from already-decoded entries.
    pub fn from_entries(entries: impl IntoIterator<Item = ScSpecEntry>) -> Self {
        let mut entry_count = 0;
        let error_enums = entries
            .into_iter()
            .inspect(|_| entry_count += 1)
            .filter_map(|e| match e {
                ScSpecEntry::UdtErrorEnumV0(e) => Some(ErrorEnum {
                    name: e.name.to_utf8_string_lossy(),
                    lib: e.lib.to_utf8_string_lossy(),
                    doc: e.doc.to_utf8_string_lossy(),
                    cases: e
                        .cases
                        .iter()
                        .map(|c| ErrorCase {
                            name: c.name.to_utf8_string_lossy(),
                            value: c.value,
                            doc: c.doc.to_utf8_string_lossy(),
                        })
                        .collect(),
                }),
                _ => None,
            })
            .collect();
        Self {
            wasm_hash: None,
            error_enums,
            entry_count,
        }
    }

    /// Record the hash of the WASM this spec came from.
    pub fn with_wasm_hash(mut self, hash: [u8; 32]) -> Self {
        self.wasm_hash = Some(hash);
        self
    }

    /// Look up the name declared for a contract error code.
    pub fn lookup(&self, code: u32) -> ErrorLookup<'_> {
        let matches: Vec<(&ErrorEnum, &ErrorCase)> = self
            .error_enums
            .iter()
            .flat_map(|en| en.cases.iter().map(move |c| (en, c)))
            .filter(|(_, c)| c.value == code)
            .collect();

        match matches.as_slice() {
            [] => ErrorLookup::NotFound,
            [(error_enum, case)] => ErrorLookup::Found { error_enum, case },
            [(error_enum, case), rest @ ..] if rest.iter().all(|(_, c)| c.name == case.name) => {
                ErrorLookup::Found { error_enum, case }
            }
            _ => ErrorLookup::Conflicting(matches),
        }
    }
}

#[cfg(test)]
mod tests {
    //! Synthetic specs built in code. The real mainnet specs are exercised from
    //! `fixtures/contracts/` by the integration tests.

    use super::*;
    use crate::testutil::error_enum;
    use stellar_xdr::{
        ScSpecFunctionInputV0, ScSpecFunctionV0, ScSpecTypeDef, ScSpecTypeOption, WriteXdr,
    };

    fn wasm_with_spec(entries: &[ScSpecEntry]) -> Vec<u8> {
        let mut content = Vec::new();
        for e in entries {
            content.extend(e.to_xdr(Limits::none()).unwrap());
        }
        let name = SPEC_SECTION.as_bytes();
        let mut payload = vec![name.len() as u8];
        payload.extend_from_slice(name);
        payload.extend(content);
        let mut m = b"\0asm\x01\0\0\0".to_vec();
        m.push(0);
        let mut n = payload.len() as u32;
        loop {
            let b = (n & 0x7f) as u8;
            n >>= 7;
            if n == 0 {
                m.push(b);
                break;
            }
            m.push(b | 0x80);
        }
        m.extend(payload);
        m
    }

    #[test]
    fn known_code_resolves_to_its_declared_name() {
        let spec = ContractSpec::from_entries([error_enum("Error", &[("Unauthorized", 3)])]);
        match spec.lookup(3) {
            ErrorLookup::Found { error_enum, case } => {
                assert_eq!(error_enum.name, "Error");
                assert_eq!(case.name, "Unauthorized");
            }
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[test]
    fn unknown_code_is_not_found() {
        let spec = ContractSpec::from_entries([error_enum("Error", &[("A", 1)])]);
        assert_eq!(spec.lookup(99), ErrorLookup::NotFound);
    }

    #[test]
    fn conflicting_names_for_one_code_are_not_resolved_by_picking_one() {
        let spec = ContractSpec::from_entries([
            error_enum("Error", &[("Unauthorized", 1)]),
            error_enum("LibError", &[("Paused", 1)]),
        ]);
        assert!(matches!(spec.lookup(1), ErrorLookup::Conflicting(m) if m.len() == 2));
    }

    #[test]
    fn identical_names_across_enums_are_not_a_conflict() {
        let spec = ContractSpec::from_entries([
            error_enum("A", &[("Paused", 1)]),
            error_enum("B", &[("Paused", 1)]),
        ]);
        assert!(matches!(spec.lookup(1), ErrorLookup::Found { .. }));
    }

    #[test]
    fn spec_round_trips_through_wasm() {
        let wasm = wasm_with_spec(&[error_enum("Error", &[("A", 1), ("B", 2)])]);
        let spec = ContractSpec::from_wasm(&wasm).unwrap();
        assert_eq!(spec.entry_count, 1);
        assert_eq!(spec.error_enums[0].cases.len(), 2);
        assert_eq!(
            spec.wasm_hash, None,
            "the hash is supplied by the caller, never invented"
        );
    }

    #[test]
    fn wasm_without_a_spec_section_says_so() {
        assert_eq!(
            ContractSpec::from_wasm(b"\0asm\x01\0\0\0"),
            Err(SpecError::NoSpecSection)
        );
    }

    #[test]
    fn malformed_spec_xdr_fails_gracefully() {
        let mut wasm = wasm_with_spec(&[]);
        // Rewrite the section to hold garbage instead of XDR.
        let garbage = [0xde, 0xad, 0xbe, 0xef, 0x01];
        let name = SPEC_SECTION.as_bytes();
        let mut payload = vec![name.len() as u8];
        payload.extend_from_slice(name);
        payload.extend_from_slice(&garbage);
        wasm.truncate(8);
        wasm.push(0);
        wasm.push(payload.len() as u8);
        wasm.extend(payload);
        assert!(matches!(
            ContractSpec::from_wasm(&wasm),
            Err(SpecError::Xdr(_))
        ));
    }

    #[test]
    fn hostile_deeply_nested_spec_is_an_error_not_a_stack_overflow() {
        // A function whose input type is Option<Option<…>> nested far beyond
        // SPEC_DECODE_DEPTH. Anyone can deploy WASM carrying this.
        let mut ty = ScSpecTypeDef::U32;
        for _ in 0..(SPEC_DECODE_DEPTH * 4) {
            ty = ScSpecTypeDef::Option(Box::new(ScSpecTypeOption {
                value_type: Box::new(ty),
            }));
        }
        let hostile = ScSpecEntry::FunctionV0(ScSpecFunctionV0 {
            doc: "".try_into().unwrap(),
            name: "f".try_into().unwrap(),
            inputs: vec![ScSpecFunctionInputV0 {
                doc: "".try_into().unwrap(),
                name: "x".try_into().unwrap(),
                type_: ty,
            }]
            .try_into()
            .unwrap(),
            outputs: Vec::new().try_into().unwrap(),
        });
        let wasm = wasm_with_spec(&[hostile]);
        assert!(matches!(
            ContractSpec::from_wasm(&wasm),
            Err(SpecError::Xdr(_))
        ));
    }

    #[test]
    fn non_wasm_fails_gracefully() {
        assert!(matches!(
            ContractSpec::from_wasm(b"hello"),
            Err(SpecError::Wasm(WasmError::NotWasm))
        ));
    }
}
