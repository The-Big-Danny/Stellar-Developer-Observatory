#![no_std]

use soroban_sdk::{contract, contractimpl, Address, Env};

/// A minimal contract whose only purpose is to require authorization from a
/// caller-supplied address.
///
/// Deployed to testnet and invoked with that address's authorization entry
/// removed, so the host rejects the invocation for missing authorization. The
/// result is a real missing-authorization failure, the category the M4 corpus
/// previously lacked.
#[contract]
pub struct AuthContract;

#[contractimpl]
impl AuthContract {
    /// Requires `who` to have authorized this invocation, then returns.
    pub fn guarded(_env: Env, who: Address) {
        who.require_auth();
    }
}
