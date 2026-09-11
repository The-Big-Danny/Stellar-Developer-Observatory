//! Plain-text rendering of a model and diagnosis.
//!
//! Presentation only: every line printed here is either read from the model
//! or from the diagnosis. Nothing is inferred at this layer.

use std::fmt::Write;

use soroban_failure_analysis::contract::{
    ContractErrorReport, ContractIdentification, ErrorResolution, IdentificationBasis,
    SpecProvenance,
};
use soroban_failure_analysis::model::{error_label, CallOutcome, OperationKind};
use soroban_failure_analysis::{Diagnosis, TransactionModel};
use stellar_xdr::ContractId;

fn short(id: &ContractId) -> String {
    let s = id.to_string();
    format!("{}…{}", &s[..6], &s[s.len() - 4..])
}

fn hex8(bytes: &[u8; 32]) -> String {
    bytes[..4]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
        + "…"
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The full report.
pub fn report(model: &TransactionModel, diagnosis: &Diagnosis) -> String {
    let mut s = String::new();
    summary(&mut s, model, diagnosis);
    invocation(&mut s, model);
    trace(&mut s, model);
    terminal(&mut s, model);
    contract_errors(&mut s, &diagnosis.contract_errors);
    resources(&mut s, model);
    causes(&mut s, diagnosis);
    s
}

fn summary(s: &mut String, m: &TransactionModel, d: &Diagnosis) {
    let _ = writeln!(
        s,
        "Transaction     {}",
        m.hash.as_deref().unwrap_or("<unknown>")
    );
    match (&m.fee_bump, &m.outcome.fee_bump) {
        (Some(env), Some(res)) => {
            let _ = writeln!(
                s,
                "Fee-bumped      yes — fee source {}, outer result {}",
                env.fee_source, res.outer_code
            );
        }
        (Some(env), None) => {
            let _ = writeln!(s, "Fee-bumped      yes — fee source {}", env.fee_source);
        }
        _ => {
            let _ = writeln!(s, "Fee-bumped      no");
        }
    }
    let _ = writeln!(s, "Source account  {}", m.source_account);

    let mut result = m.outcome.result_code.to_string();
    if let Some(op) = &m.outcome.failed_operation {
        let _ = write!(
            result,
            " — operation {}: {} {}",
            op.index,
            op.operation.unwrap_or("(generic)"),
            op.code
        );
    }
    let _ = writeln!(s, "Result          {result}");

    match d.stage {
        Some(stage) => {
            let _ = writeln!(s, "Failure stage   {} — {}", stage, stage.description());
        }
        None => {
            let _ = writeln!(s, "Failure stage   none — the transaction succeeded");
        }
    }

    let events = &m.diagnostics.events;
    let metrics = events.len() - m.diagnostics.execution_event_count();
    let _ = writeln!(
        s,
        "Diagnostic      {}",
        if events.is_empty() {
            "none returned".to_string()
        } else {
            format!(
                "{} events ({} execution, {} core_metrics)",
                events.len(),
                events.len() - metrics,
                metrics
            )
        }
    );
    for note in &m.notes {
        let _ = writeln!(s, "Note            {note}");
    }
}

fn invocation(s: &mut String, m: &TransactionModel) {
    for op in &m.operations {
        if let OperationKind::InvokeHostFunction {
            host_function,
            invocation,
        } = &op.kind
        {
            let _ = writeln!(s, "\nInvocation");
            match invocation {
                Some(inv) => {
                    let _ = writeln!(
                        s,
                        "  {}::{}({} args)",
                        inv.contract,
                        inv.function,
                        inv.args.len()
                    );
                }
                None => {
                    let _ = writeln!(s, "  {host_function}");
                }
            }
            if let Some(soroban) = &m.soroban {
                let _ = writeln!(
                    s,
                    "  auth entries: {}   footprint: {} read-only, {} read-write",
                    soroban.auth.len(),
                    soroban.footprint.read_only.len(),
                    soroban.footprint.read_write.len()
                );
            }
        }
    }
}

fn trace(s: &mut String, m: &TransactionModel) {
    if m.diagnostics.calls.is_empty() {
        return;
    }
    let _ = writeln!(s, "\nCall trace (from diagnostic events)");
    for frame in &m.diagnostics.calls {
        let outcome = match &frame.outcome {
            CallOutcome::Returned => "returned".to_string(),
            CallOutcome::Failed { error, .. } => format!("failed {}", error_label(error)),
            CallOutcome::Unknown => "outcome not shown".to_string(),
        };
        let call = format!(
            "{}{}::{}",
            "  ".repeat(frame.depth as usize),
            short(&frame.contract),
            frame.function
        );
        let _ = writeln!(s, "  {call:<40} {outcome}");
    }
}

fn terminal(s: &mut String, m: &TransactionModel) {
    let Some(t) = &m.diagnostics.terminal_error else {
        return;
    };
    let _ = writeln!(s, "\nTerminal error");
    let _ = writeln!(s, "  {}", error_label(&t.error));
    // The host's own words from the first error event carrying the same value.
    // Printed as evidence, verbatim; interpreting it is left to rules.
    let message = m
        .diagnostics
        .errors()
        .find(|(_, e, msg)| **e == t.error && msg.is_some())
        .and_then(|(_, _, msg)| msg);
    if let Some(msg) = message {
        let _ = writeln!(s, "  host message: \"{msg}\"");
    }
    if !matches!(t.error, stellar_xdr::ScError::Contract(_)) {
        let _ = writeln!(
            s,
            "  a host error, not a contract-defined one: contract error resolution does not apply"
        );
    }
}

fn contract_errors(s: &mut String, reports: &[ContractErrorReport]) {
    if reports.is_empty() {
        return;
    }
    let _ = writeln!(s, "\nContract errors");
    for r in reports {
        let role = if r.terminal {
            "terminal"
        } else {
            "raised, then caught or superseded"
        };
        let name = r.resolution.name().unwrap_or("unknown");
        let _ = writeln!(s, "  Error(Contract, #{})  →  {name}   [{role}]", r.code);

        match &r.identification {
            ContractIdentification::Unique { contract, basis } => {
                let how = match basis {
                    IdentificationBasis::SoleEmitter => "only contract to raise it",
                    IdentificationBasis::OriginMarker => "origin frame of a re-emitted error",
                };
                let _ = writeln!(s, "      contract    {contract} ({how})");
            }
            ContractIdentification::Ambiguous { candidates } => {
                let list: Vec<String> = candidates.iter().map(short).collect();
                let _ = writeln!(s, "      contract    ambiguous: {}", list.join(", "));
            }
            ContractIdentification::Unidentified => {
                let _ = writeln!(s, "      contract    not identified by any error event");
            }
        }

        let resolution = match &r.resolution {
            ErrorResolution::Resolved {
                enum_name,
                provenance,
                doc,
                ..
            } => {
                let mut line = format!("from the contract spec (enum {enum_name})");
                match provenance {
                    SpecProvenance::MatchesFootprint { wasm_hash } => {
                        let _ = write!(
                            line,
                            "; WASM {} matches the transaction footprint",
                            hex8(wasm_hash)
                        );
                    }
                    SpecProvenance::Unverified => line.push_str("; spec version not verified"),
                }
                if !doc.is_empty() {
                    let _ = write!(line, "\n      doc         {doc}");
                }
                line
            }
            ErrorResolution::CodeNotInSpec { .. } => {
                "unavailable — the contract spec declares no name for this code".into()
            }
            ErrorResolution::AmbiguousInSpec { candidates, .. } => {
                let names: Vec<String> = candidates
                    .iter()
                    .map(|(e, c)| format!("{e}::{c}"))
                    .collect();
                format!("ambiguous — the spec declares {}", names.join(" and "))
            }
            ErrorResolution::SpecUnavailable { reason, .. } => format!("unavailable — {reason}"),
            ErrorResolution::SpecVersionMismatch { spec_wasm_hash, .. } => format!(
                "unavailable — spec WASM {} is not the code the transaction loaded \
                 (contract upgraded since?)",
                hex8(spec_wasm_hash)
            ),
            ErrorResolution::ContractNotIdentified => {
                "unavailable — no single contract could be identified".into()
            }
            ErrorResolution::NotApplicable => "not applicable".into(),
        };
        let _ = writeln!(s, "      resolution  {resolution}");
    }
}

fn resources(s: &mut String, m: &TransactionModel) {
    let Some(soroban) = &m.soroban else {
        return;
    };
    let d = &soroban.declared;
    let o = &m.observed;
    let observed = |v: Option<u64>| v.map_or("not reported".to_string(), thousands);

    let _ = writeln!(s, "\nResources               declared        observed");
    let _ = writeln!(
        s,
        "  CPU instructions      {:<15} {}",
        thousands(u64::from(d.instructions)),
        observed(o.cpu_instructions())
    );
    let _ = writeln!(
        s,
        "  memory bytes          {:<15} {}",
        "—",
        observed(o.memory_bytes())
    );
    let _ = writeln!(
        s,
        "  disk read bytes       {:<15} —",
        thousands(u64::from(d.disk_read_bytes))
    );
    let _ = writeln!(
        s,
        "  write bytes           {:<15} —",
        thousands(u64::from(d.write_bytes))
    );
    let charged = o.fees_charged.map_or("not reported".to_string(), |f| {
        format!(
            "{} charged (non-refundable {}, refundable {}, rent {})",
            thousands(
                f.non_refundable
                    .saturating_add(f.refundable)
                    .saturating_add(f.rent)
                    .max(0) as u64
            ),
            f.non_refundable,
            f.refundable,
            f.rent
        )
    });
    let _ = writeln!(
        s,
        "  resource fee          {:<15} {charged}",
        thousands(d.resource_fee.max(0) as u64)
    );
    let _ = writeln!(
        s,
        "  (observed values are the reporting node's diagnostic measurements)"
    );
}

fn causes(s: &mut String, d: &Diagnosis) {
    let _ = writeln!(s, "\nCandidate causes");
    if d.is_undetermined() {
        let _ = writeln!(
            s,
            "  none — no failure rules are implemented yet (milestone M4)"
        );
    } else {
        for (i, c) in d.candidate_causes.iter().enumerate() {
            let _ = writeln!(
                s,
                "  {}. [{}] {} ({})",
                i + 1,
                c.confidence.id(),
                c.summary,
                c.class
            );
        }
    }
    let _ = writeln!(s, "\nLimitations");
    for l in &d.limitations {
        let _ = writeln!(s, "  - {l}");
    }
}
