#![no_std]

use soroban_sdk::{contract, contractimpl, Env};

/// A minimal contract whose only purpose is to panic unconditionally.
///
/// Deployed to testnet and invoked once to capture a real, evidence-backed
/// `ContractTrap` fixture for issue #13 — a contract failure that is *not*
/// one of its own declared `#[contracterror]` codes. `unwrap()` on `None` is
/// used deliberately, since it is one of the most common ways a real
/// contract traps during development.
#[contract]
pub struct TrapContract;

#[contractimpl]
impl TrapContract {
    /// Always panics via `unwrap()` on `None`.
    pub fn trigger(_env: Env) {
        let nothing: Option<u32> = None;
        nothing.unwrap();
    }

    /// Succeeds. Used only to obtain a simulated footprint and fee for
    /// `trigger`, which traps before touching any ledger entry, so the same
    /// resource profile applies to both.
    pub fn ok(_env: Env) -> u32 {
        1
    }
}
