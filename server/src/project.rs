//! One Eplyx-protected Solana program, and the token that may check it.
//!
//! Deliberately not a user model. There are no organizations, teams, roles,
//! invitations or billing here: this is pilot infrastructure, and a project is
//! one program with one active bundle and one CI token.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A project's stored record.
///
/// The raw CI token is never in it. What is stored is a salted SHA-256
/// verifier, so a leaked data volume does not hand over the ability to run
/// checks as the project.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub program_id: String,
    /// Content hash of the bundle this project currently checks against.
    /// `None` until an operator activates one — a project cannot check before
    /// then, and says so rather than picking a bundle on its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_bundle_sha256: Option<String>,
    pub token: TokenVerifier,
}

/// Salted hash of a CI token.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenVerifier {
    pub salt: String,
    pub sha256: String,
}

fn digest(salt: &str, token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update(b":");
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

/// Compare without leaking where two values first differ.
///
/// The timing signal from a short-circuiting comparison is small over a
/// network, but the cost of not doing this is zero.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0_u8, |difference, (x, y)| difference | (x ^ y))
        == 0
}

impl TokenVerifier {
    /// Build a verifier for a freshly generated token.
    pub fn new(token: &str) -> Self {
        let salt: [u8; 16] = rand::random();
        let salt = hex::encode(salt);
        let sha256 = digest(&salt, token);
        Self { salt, sha256 }
    }

    pub fn verifies(&self, token: &str) -> bool {
        constant_time_eq(&self.sha256, &digest(&self.salt, token))
    }
}

/// A new CI token. Returned to the operator once, at creation, and never again.
pub fn generate_token() -> String {
    let bytes: [u8; 32] = rand::random();
    format!("eplyx_{}", hex::encode(bytes))
}

impl Project {
    pub fn new(id: &str, name: &str, program_id: &str, token: &str) -> Result<Self> {
        if !crate::storage::valid_id(id) {
            bail!("project id {id:?} is not a valid identifier");
        }
        Ok(Self {
            id: id.to_string(),
            name: name.to_string(),
            program_id: program_id.to_string(),
            active_bundle_sha256: None,
            token: TokenVerifier::new(token),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_verifies_only_itself() {
        let token = generate_token();
        let verifier = TokenVerifier::new(&token);
        assert!(verifier.verifies(&token));
        assert!(!verifier.verifies(&generate_token()));
        assert!(!verifier.verifies(""));
        assert!(!verifier.verifies(&format!("{token}x")));
    }

    /// A leaked data volume must not hand over the ability to run checks.
    #[test]
    fn the_stored_record_never_contains_the_token() {
        let token = generate_token();
        let project = Project::new("stake-pool", "Stake Pool", "SPoo1", &token).unwrap();
        let stored = serde_json::to_string(&project).unwrap();
        assert!(!stored.contains(&token), "raw token was persisted");
        assert!(stored.contains(&project.token.sha256));
    }

    /// Two projects issued the same token string still get different
    /// verifiers, so one stored digest cannot be replayed against another.
    #[test]
    fn verifiers_are_salted_per_project() {
        let token = generate_token();
        let a = TokenVerifier::new(&token);
        let b = TokenVerifier::new(&token);
        assert_ne!(a.salt, b.salt);
        assert_ne!(a.sha256, b.sha256);
        assert!(a.verifies(&token) && b.verifies(&token));
    }

    #[test]
    fn an_invalid_project_id_is_refused() {
        assert!(Project::new("../escape", "x", "y", "t").is_err());
    }
}
