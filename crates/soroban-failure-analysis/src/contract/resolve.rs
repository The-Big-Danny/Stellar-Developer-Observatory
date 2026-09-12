//! Resolving `Error(Contract, #N)` to the name the contract declared.
//!
//! Three questions have to be answered in order, and each can honestly fail:
//!
//! 1. **Which contract raised it?** A number is only meaningful relative to one
//!    contract's spec. Several contracts may appear, and a caller that catches
//!    or propagates a callee's error re-emits the callee's number from its own
//!    frame. See [`ContractIdentification`].
//! 2. **Do we have that contract's spec?** Specs are supplied by the caller (the
//!    engine does no I/O). See [`SpecAvailability`].
//! 3. **Is the spec for the code that actually ran?** Contracts can be upgraded
//!    after a failure. The transaction's footprint names the exact WASM it
//!    loaded, so a spec whose WASM hash is not in the footprint describes some
//!    other version of the contract. See [`SpecProvenance`].
//!
//! Only when all three succeed is a name reported. Every other outcome is a
//! distinct [`ErrorResolution`] variant, so "we could not name this" can never
//! be confused with a name.

use std::collections::BTreeMap;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use stellar_xdr::{ContractId, ScError};

use super::spec::{ContractSpec, ErrorLookup};
use crate::model::TransactionModel;

/// The host's message on the `error` event emitted when a contract raises its
/// own error via `fail_with_error`.
///
/// It marks the *origin* frame: a caller reporting a caught or propagated
/// failure emits the same error value with a different message. This is a
/// host diagnostic string — not consensus, and not a stable API — so it is
/// only used to break a tie between several emitters, never on its own.
pub const ORIGIN_MARKER: &str = "failing with contract error";

/// Whether a contract's spec could be obtained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecAvailability {
    /// The spec was obtained and parsed.
    Available(ContractSpec),
    /// It could not be, for the stated reason.
    Unavailable {
        /// Why — surfaced to the user verbatim.
        reason: String,
    },
}

/// How a contract was singled out as the one that raised an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum IdentificationBasis {
    /// It was the only contract that emitted an `error` event with this value.
    SoleEmitter,
    /// Several contracts emitted it; only this one emitted the origin marker.
    OriginMarker,
}

/// Which contract raised an error.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
pub enum ContractIdentification {
    /// Exactly one contract.
    Unique {
        /// The contract.
        contract: ContractId,
        /// How it was determined.
        basis: IdentificationBasis,
    },
    /// Several contracts, and nothing in the events separates them.
    Ambiguous {
        /// Every candidate, in order of first appearance.
        candidates: Vec<ContractId>,
    },
    /// No `error` event attributes the value to any contract.
    Unidentified,
}

impl ContractIdentification {
    /// The contract, if exactly one was identified.
    pub fn contract(&self) -> Option<&ContractId> {
        match self {
            Self::Unique { contract, .. } => Some(contract),
            _ => None,
        }
    }
}

/// Whether a spec is known to describe the code that ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
pub enum SpecProvenance {
    /// The spec's WASM hash is in the transaction's footprint: it is the code
    /// the transaction loaded.
    MatchesFootprint {
        /// The WASM hash.
        wasm_hash: [u8; 32],
    },
    /// The spec carries no WASM hash, or the transaction has no footprint to
    /// check against. The name may still be right; it is not verified.
    Unverified,
}

/// The outcome of trying to name one contract error.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
pub enum ErrorResolution {
    /// The contract's spec declares a name for the code.
    Resolved {
        /// The contract whose spec was used.
        contract: ContractId,
        /// The error enum, e.g. `Error`.
        enum_name: String,
        /// The case, e.g. `NoHarvestablePails`.
        case_name: String,
        /// The developer's doc comment for the case, possibly empty.
        doc: String,
        /// Whether the spec is known to match the code that ran.
        provenance: SpecProvenance,
    },
    /// The spec was obtained, but declares no name for this code.
    CodeNotInSpec {
        /// The contract.
        contract: ContractId,
        /// Whether that spec is known to match the code that ran.
        provenance: SpecProvenance,
    },
    /// The spec declares the code under more than one name.
    AmbiguousInSpec {
        /// The contract.
        contract: ContractId,
        /// `(enum, case)` for each candidate.
        candidates: Vec<(String, String)>,
    },
    /// The contract was identified but its spec could not be obtained.
    SpecUnavailable {
        /// The contract.
        contract: ContractId,
        /// Why.
        reason: String,
    },
    /// A spec was obtained, but for WASM the transaction did not load — the
    /// contract has probably been upgraded since. Its names may not apply.
    SpecVersionMismatch {
        /// The contract.
        contract: ContractId,
        /// The hash of the WASM the spec came from.
        spec_wasm_hash: [u8; 32],
    },
    /// No single contract could be identified. See the report's
    /// [`ContractIdentification`] for the candidates.
    ContractNotIdentified,
    /// The error is not a contract-defined error, so there is nothing to name.
    NotApplicable,
}

impl ErrorResolution {
    /// The resolved case name, only when resolution fully succeeded.
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Resolved { case_name, .. } => Some(case_name),
            _ => None,
        }
    }
}

/// Everything known about one contract error code seen in a transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ContractErrorReport {
    /// The code, as in `Error(Contract, code)`.
    pub code: u32,
    /// Whether this is the error the invocation as a whole failed with.
    ///
    /// Non-terminal errors were raised and then caught or superseded; they are
    /// often the more useful clue.
    pub terminal: bool,
    /// Diagnostic events carrying this error value.
    pub event_indexes: Vec<u32>,
    /// Which contract raised it.
    pub identification: ContractIdentification,
    /// Whether a name could be found.
    pub resolution: ErrorResolution,
}

/// Every contract error code in the transaction's diagnostic events, resolved
/// against the supplied specs.
///
/// Ordered terminal first, then by first appearance, so output is stable.
pub fn resolve_contract_errors(
    model: &TransactionModel,
    specs: &BTreeMap<ContractId, SpecAvailability>,
) -> Vec<ContractErrorReport> {
    let terminal = model
        .diagnostics
        .terminal_error
        .as_ref()
        .and_then(|t| match t.error {
            ScError::Contract(code) => Some(code),
            _ => None,
        });

    let mut codes: Vec<u32> = Vec::new();
    for (_, error, _) in model.diagnostics.errors() {
        if let ScError::Contract(code) = error {
            if !codes.contains(code) {
                codes.push(*code);
            }
        }
    }
    if let Some(t) = terminal {
        if !codes.contains(&t) {
            codes.push(t);
        }
    }
    codes.sort_by_key(|c| Some(*c) != terminal);

    let footprint: Option<Vec<[u8; 32]>> = model
        .soroban
        .as_ref()
        .map(|s| s.footprint.contract_code_hashes());

    codes
        .into_iter()
        .map(|code| {
            let identification = identify(model, code);
            let resolution = resolve(
                &ScError::Contract(code),
                &identification,
                specs,
                footprint.as_deref(),
            );
            let mut event_indexes: Vec<u32> = model
                .diagnostics
                .errors()
                .filter(|(_, e, _)| **e == ScError::Contract(code))
                .map(|(ev, _, _)| ev.index)
                .collect();
            if let Some(t) = &model.diagnostics.terminal_error {
                if t.error == ScError::Contract(code) {
                    event_indexes.push(t.event_index);
                }
            }
            ContractErrorReport {
                code,
                terminal: Some(code) == terminal,
                event_indexes,
                identification,
                resolution,
            }
        })
        .collect()
}

/// Contracts whose specs are needed to resolve this transaction's errors.
///
/// Only uniquely identified contracts are listed: fetching a spec for an
/// ambiguous one could not produce a trustworthy name anyway. This is what an
/// I/O layer should fetch before calling [`resolve_contract_errors`].
pub fn contracts_needing_specs(model: &TransactionModel) -> Vec<ContractId> {
    let mut out: Vec<ContractId> = Vec::new();
    for report in resolve_contract_errors(model, &BTreeMap::new()) {
        if let Some(c) = report.identification.contract() {
            if !out.contains(c) {
                out.push(c.clone());
            }
        }
    }
    out
}

/// Identify the contract that raised `Error(Contract, code)`.
pub fn identify(model: &TransactionModel, code: u32) -> ContractIdentification {
    let target = ScError::Contract(code);
    let mut emitters: Vec<ContractId> = Vec::new();
    let mut origins: Vec<ContractId> = Vec::new();

    for (event, error, message) in model.diagnostics.errors() {
        let Some(contract) = &event.contract else {
            continue;
        };
        if *error != target {
            continue;
        }
        if !emitters.contains(contract) {
            emitters.push(contract.clone());
        }
        if message.is_some_and(|m| m.starts_with(ORIGIN_MARKER)) && !origins.contains(contract) {
            origins.push(contract.clone());
        }
    }

    match (emitters.as_slice(), origins.as_slice()) {
        ([], _) => ContractIdentification::Unidentified,
        ([only], _) => ContractIdentification::Unique {
            contract: only.clone(),
            basis: IdentificationBasis::SoleEmitter,
        },
        (_, [origin]) => ContractIdentification::Unique {
            contract: origin.clone(),
            basis: IdentificationBasis::OriginMarker,
        },
        _ => ContractIdentification::Ambiguous {
            candidates: emitters,
        },
    }
}

/// Resolve one error against the supplied specs.
///
/// `footprint_code_hashes` is the set of WASM hashes the transaction loaded;
/// `None` when there is no footprint to check against.
pub fn resolve(
    error: &ScError,
    identification: &ContractIdentification,
    specs: &BTreeMap<ContractId, SpecAvailability>,
    footprint_code_hashes: Option<&[[u8; 32]]>,
) -> ErrorResolution {
    let ScError::Contract(code) = error else {
        return ErrorResolution::NotApplicable;
    };
    let Some(contract) = identification.contract() else {
        return ErrorResolution::ContractNotIdentified;
    };

    let spec = match specs.get(contract) {
        Some(SpecAvailability::Available(spec)) => spec,
        Some(SpecAvailability::Unavailable { reason }) => {
            return ErrorResolution::SpecUnavailable {
                contract: contract.clone(),
                reason: reason.clone(),
            }
        }
        None => {
            return ErrorResolution::SpecUnavailable {
                contract: contract.clone(),
                reason: "no contract spec was supplied for this contract".into(),
            }
        }
    };

    let provenance = match (spec.wasm_hash, footprint_code_hashes) {
        (Some(hash), Some(loaded)) if !loaded.is_empty() => {
            if !loaded.contains(&hash) {
                return ErrorResolution::SpecVersionMismatch {
                    contract: contract.clone(),
                    spec_wasm_hash: hash,
                };
            }
            SpecProvenance::MatchesFootprint { wasm_hash: hash }
        }
        _ => SpecProvenance::Unverified,
    };

    match spec.lookup(*code) {
        ErrorLookup::Found { error_enum, case } => ErrorResolution::Resolved {
            contract: contract.clone(),
            enum_name: error_enum.name.clone(),
            case_name: case.name.clone(),
            doc: case.doc.clone(),
            provenance,
        },
        ErrorLookup::NotFound => ErrorResolution::CodeNotInSpec {
            contract: contract.clone(),
            provenance,
        },
        ErrorLookup::Conflicting(matches) => ErrorResolution::AmbiguousInSpec {
            contract: contract.clone(),
            candidates: matches
                .into_iter()
                .map(|(e, c)| (e.name.clone(), c.name.clone()))
                .collect(),
        },
    }
}

#[cfg(test)]
mod tests {
    //! Synthetic resolver tests. These are **not** real transactions; they pin
    //! down each resolution outcome, including ones the real fixture corpus
    //! does not yet contain. Real mainnet resolution is tested from `fixtures/`
    //! in `crates/sdo/tests/contract_resolution.rs`.

    use super::*;
    use crate::testutil::{call, cid, err, error_enum, host_fn_failed, input_with_events};
    use stellar_xdr::ScErrorCode;

    fn model(events: Vec<stellar_xdr::DiagnosticEvent>) -> TransactionModel {
        TransactionModel::from_input(&input_with_events(events))
    }

    #[test]
    fn a_sole_emitter_is_identified() {
        let m = model(vec![
            call(None, cid(1), "f"),
            err(cid(1), ScError::Contract(4), "anything"),
        ]);
        assert_eq!(
            identify(&m, 4),
            ContractIdentification::Unique {
                contract: cid(1),
                basis: IdentificationBasis::SoleEmitter
            }
        );
    }

    #[test]
    fn a_propagated_error_is_attributed_to_its_origin_not_the_caller() {
        // B raises #9; A reports the caught failure with the same value. A's
        // spec may define #9 as something else entirely — naming it from A
        // would be confidently wrong.
        let m = model(vec![
            call(None, cid(1), "outer"),
            call(Some(cid(1)), cid(2), "inner"),
            err(cid(2), ScError::Contract(9), ORIGIN_MARKER),
            err(cid(1), ScError::Contract(9), "contract try_call failed"),
        ]);
        assert_eq!(
            identify(&m, 9),
            ContractIdentification::Unique {
                contract: cid(2),
                basis: IdentificationBasis::OriginMarker
            }
        );
    }

    #[test]
    fn several_emitters_with_no_origin_marker_are_ambiguous() {
        let m = model(vec![
            err(cid(1), ScError::Contract(5), "x"),
            err(cid(2), ScError::Contract(5), "y"),
        ]);
        assert_eq!(
            identify(&m, 5),
            ContractIdentification::Ambiguous {
                candidates: vec![cid(1), cid(2)]
            }
        );
    }

    #[test]
    fn a_code_no_contract_emitted_is_unidentified() {
        let m = model(vec![host_fn_failed(ScError::Contract(7))]);
        assert_eq!(identify(&m, 7), ContractIdentification::Unidentified);
        let reports = resolve_contract_errors(&m, &BTreeMap::new());
        assert_eq!(reports.len(), 1);
        assert!(reports[0].terminal);
        assert_eq!(
            reports[0].resolution,
            ErrorResolution::ContractNotIdentified
        );
    }

    #[test]
    fn reports_list_the_terminal_error_first_then_in_order_of_appearance() {
        let m = model(vec![
            call(None, cid(1), "f"),
            call(Some(cid(1)), cid(2), "g"),
            err(cid(2), ScError::Contract(9), ORIGIN_MARKER),
            err(cid(1), ScError::Contract(2), ORIGIN_MARKER),
            host_fn_failed(ScError::Contract(2)),
        ]);
        let reports = resolve_contract_errors(&m, &BTreeMap::new());
        assert_eq!(
            reports
                .iter()
                .map(|r| (r.code, r.terminal))
                .collect::<Vec<_>>(),
            vec![(2, true), (9, false)]
        );
    }

    #[test]
    fn only_uniquely_identified_contracts_are_requested() {
        let m = model(vec![
            err(cid(1), ScError::Contract(1), "a"),
            err(cid(2), ScError::Contract(5), "x"),
            err(cid(3), ScError::Contract(5), "y"),
        ]);
        assert_eq!(contracts_needing_specs(&m), vec![cid(1)]);
    }

    fn unique(n: u8) -> ContractIdentification {
        ContractIdentification::Unique {
            contract: cid(n),
            basis: IdentificationBasis::SoleEmitter,
        }
    }

    fn specs(n: u8, spec: ContractSpec) -> BTreeMap<ContractId, SpecAvailability> {
        BTreeMap::from([(cid(n), SpecAvailability::Available(spec))])
    }

    fn spec() -> ContractSpec {
        ContractSpec::from_entries([error_enum("Error", &[("Unauthorized", 3)])])
    }

    #[test]
    fn known_code_resolves_and_identifies_its_contract() {
        let r = resolve(&ScError::Contract(3), &unique(1), &specs(1, spec()), None);
        match r {
            ErrorResolution::Resolved {
                contract,
                case_name,
                enum_name,
                provenance,
                ..
            } => {
                assert_eq!(contract, cid(1));
                assert_eq!(case_name, "Unauthorized");
                assert_eq!(enum_name, "Error");
                assert_eq!(provenance, SpecProvenance::Unverified);
            }
            other => panic!("expected Resolved, got {other:?}"),
        }
    }

    #[test]
    fn unknown_code_is_reported_as_not_in_spec() {
        let r = resolve(&ScError::Contract(99), &unique(1), &specs(1, spec()), None);
        assert!(matches!(r, ErrorResolution::CodeNotInSpec { .. }));
        assert_eq!(r.name(), None);
    }

    #[test]
    fn missing_spec_is_unavailable_not_a_guess() {
        let r = resolve(&ScError::Contract(3), &unique(1), &BTreeMap::new(), None);
        assert!(matches!(r, ErrorResolution::SpecUnavailable { .. }));
        assert_eq!(r.name(), None);
    }

    #[test]
    fn an_unavailable_spec_carries_its_reason() {
        let s = BTreeMap::from([(
            cid(1),
            SpecAvailability::Unavailable {
                reason: "archived".into(),
            },
        )]);
        let r = resolve(&ScError::Contract(3), &unique(1), &s, None);
        assert!(
            matches!(r, ErrorResolution::SpecUnavailable { reason, .. } if reason == "archived")
        );
    }

    #[test]
    fn ambiguous_contract_is_not_resolved_even_if_a_spec_exists() {
        let id = ContractIdentification::Ambiguous {
            candidates: vec![cid(1), cid(2)],
        };
        let r = resolve(&ScError::Contract(3), &id, &specs(1, spec()), None);
        assert_eq!(r, ErrorResolution::ContractNotIdentified);
    }

    #[test]
    fn non_contract_error_is_not_applicable() {
        let r = resolve(
            &ScError::Storage(ScErrorCode::ExceededLimit),
            &unique(1),
            &specs(1, spec()),
            None,
        );
        assert_eq!(r, ErrorResolution::NotApplicable);
    }

    #[test]
    fn spec_for_code_the_transaction_did_not_load_is_a_version_mismatch() {
        let upgraded = spec().with_wasm_hash([0xaa; 32]);
        let r = resolve(
            &ScError::Contract(3),
            &unique(1),
            &specs(1, upgraded),
            Some(&[[0xbb; 32]]),
        );
        assert!(matches!(r, ErrorResolution::SpecVersionMismatch { .. }));
        assert_eq!(r.name(), None, "a mismatched spec must not produce a name");
    }

    #[test]
    fn spec_matching_the_footprint_is_verified() {
        let s = spec().with_wasm_hash([0xaa; 32]);
        let r = resolve(
            &ScError::Contract(3),
            &unique(1),
            &specs(1, s),
            Some(&[[0xaa; 32]]),
        );
        assert!(matches!(
            r,
            ErrorResolution::Resolved {
                provenance: SpecProvenance::MatchesFootprint { .. },
                ..
            }
        ));
    }

    #[test]
    fn conflicting_spec_names_are_ambiguous_in_spec() {
        let s = ContractSpec::from_entries([
            error_enum("Error", &[("Unauthorized", 3)]),
            error_enum("Lib", &[("Paused", 3)]),
        ]);
        let r = resolve(&ScError::Contract(3), &unique(1), &specs(1, s), None);
        assert!(
            matches!(r, ErrorResolution::AmbiguousInSpec { candidates, .. } if candidates.len() == 2)
        );
    }
}
