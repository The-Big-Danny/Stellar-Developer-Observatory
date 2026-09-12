//! Record a contract's instance and code as an offline fixture.
//!
//! ```bash
//! cargo run -p soroban-failure-rpc --example capture_contract -- \
//!     <CONTRACT_ID> fixtures/contracts [--rpc <URL>]
//! ```
//!
//! Writes `<out>/<CONTRACT_ID>/instance.json` and `code.json`: the `result`
//! member of each `getLedgerEntries` response, verbatim, exactly as
//! `FixtureContractSource` reads them back. Also prints the parsed error enums,
//! so you can see what the fixture will resolve.
//!
//! This makes network requests. It is a capture tool, not a test.

use std::path::PathBuf;
use std::str::FromStr;

use soroban_failure_analysis::contract::ContractSpec;
use soroban_failure_rpc::contract::decode_instance;
use soroban_failure_rpc::RpcClient;
use stellar_xdr::{ContractExecutable, ContractId};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (contract, out) = match args.as_slice() {
        [c, o, ..] => (ContractId::from_str(c)?, PathBuf::from(o)),
        _ => {
            eprintln!("usage: capture_contract <CONTRACT_ID> <OUT_DIR> [--rpc <URL>]");
            std::process::exit(2);
        }
    };
    let rpc = args
        .windows(2)
        .find(|w| w[0] == "--rpc")
        .map(|w| w[1].clone())
        .unwrap_or_else(|| "https://mainnet.sorobanrpc.com".into());

    let client = RpcClient::new(&rpc);
    let dir = out.join(contract.to_string());
    std::fs::create_dir_all(&dir)?;

    let instance = client.contract_instance_entry(&contract)?;
    std::fs::write(
        dir.join("instance.json"),
        serde_json::to_string_pretty(&instance)?,
    )?;
    println!("wrote {}", dir.join("instance.json").display());

    let ContractExecutable::Wasm(hash) = decode_instance(&instance)? else {
        println!("contract is not WASM-backed; no code to record");
        return Ok(());
    };
    let code = client.contract_code_entry(&hash)?;
    std::fs::write(dir.join("code.json"), serde_json::to_string_pretty(&code)?)?;
    println!("wrote {}", dir.join("code.json").display());

    let hex: String = hash.0.iter().map(|b| format!("{b:02x}")).collect();
    println!("\nwasm hash: {hex}");
    println!("latest ledger at capture: {}", code["latestLedger"]);

    let wasm = soroban_failure_rpc::contract::decode_code(&code)?;
    let spec = ContractSpec::from_wasm(&wasm)?;
    for e in &spec.error_enums {
        println!("\nerror enum {}:", e.name);
        for c in &e.cases {
            println!("  {:>4} = {}", c.value, c.name);
        }
    }
    Ok(())
}
