//! One Eplyx-protected Solana program, its tokens and its bundle pointer.
//!
//! Deliberately not a user model. There are no organizations, teams, roles,
//! invitations or billing here: a project is one program, one adapter, one
//! active bundle and the tokens that may check it.
//!
//! Types rather than strings wherever the set of values is closed. `chain` is
//! an enum with one variant because Eplyx supports one chain, and a `String`
//! there would invite a second to be written down long before anything could
//! execute it.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The chain a project's program runs on.
///
/// One variant, on purpose. Adding a second is a body of work, not a string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Chain {
    #[default]
    Solana,
}

/// Where a project is in its own setup, never where a run is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectStatus {
    /// Exists, but has no active bundle, so a check has nothing to measure
    /// against.
    Setup,
    /// Has a verified active bundle and accepts checks.
    Ready,
    /// Accepts no checks. Set by an operator, never by the system.
    Disabled,
}

/// Which protocol vocabulary a project's bundles are built under.
///
/// `name@version`, matching what the engine's adapter reports. `none@0` is a
/// real answer rather than an absence: the engine speaks no semantics for that
/// program, every check against it reports `no_semantic_coverage`, and a bundle
/// that claims otherwise is refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AdapterId {
    pub name: String,
    pub version: u32,
}

impl AdapterId {
    pub const NONE: &'static str = "none";

    pub fn none() -> Self {
        Self {
            name: Self::NONE.to_string(),
            version: 0,
        }
    }

    /// What this build speaks for a program, which is not the caller's choice.
    ///
    /// An adapter is selected by program id inside the engine. A project
    /// declares one so the declaration can be checked, not so it can pick.
    pub fn for_program(program_id: &str) -> Self {
        match eplyx_engine::protocol::adapter_for(program_id) {
            Some(adapter) => Self {
                name: adapter.name().to_string(),
                version: adapter.adapter_version(),
            },
            None => Self::none(),
        }
    }

    /// Every adapter a project may declare, derived from the engine.
    pub fn supported() -> Vec<Self> {
        let mut all: Vec<Self> = eplyx_engine::protocol::adapters()
            .iter()
            .map(|adapter| Self {
                name: adapter.name().to_string(),
                version: adapter.adapter_version(),
            })
            .collect();
        all.push(Self::none());
        all
    }

    pub fn speaks_semantics(&self) -> bool {
        self.name != Self::NONE
    }
}

impl std::fmt::Display for AdapterId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}@{}", self.name, self.version)
    }
}

impl From<AdapterId> for String {
    fn from(id: AdapterId) -> Self {
        id.to_string()
    }
}

impl TryFrom<String> for AdapterId {
    type Error = anyhow::Error;
    fn try_from(text: String) -> Result<Self> {
        let (name, version) = text
            .split_once('@')
            .with_context(|| format!("adapter {text:?} is not name@version"))?;
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            bail!("adapter name {name:?} is not a valid adapter name");
        }
        let version: u32 = version
            .parse()
            .with_context(|| format!("adapter version {version:?} is not a number"))?;
        Ok(Self {
            name: name.to_string(),
            version,
        })
    }
}

/// A Solana program address, checked rather than believed.
///
/// A pubkey is 32 bytes in base58. Anything else is a typo at best, and at
/// worst a string chosen to become a path segment somewhere downstream.
pub fn validate_program_id(program_id: &str) -> Result<()> {
    let decoded = bs58::decode(program_id)
        .into_vec()
        .with_context(|| format!("program id {program_id:?} is not base58"))?;
    if decoded.len() != 32 {
        bail!(
            "program id {program_id:?} decodes to {} bytes, not 32",
            decoded.len()
        );
    }
    Ok(())
}

pub fn validate_name(name: &str) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        bail!("name is empty");
    }
    if trimmed.chars().count() > 80 {
        bail!("name is longer than 80 characters");
    }
    if trimmed.chars().any(|c| c.is_control()) {
        bail!("name contains control characters");
    }
    Ok(())
}

/// Which bundle a project currently measures against.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveBundle {
    pub bundle_id: String,
    pub bundle_sha256: String,
    pub activated_at_unix_seconds: u64,
}

/// A project's stored record.
///
/// No token material is in it. Tokens are their own records, so issuing and
/// revoking one never rewrites the project, and two of them can exist at once.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub project_id: String,
    pub name: String,
    pub chain: Chain,
    pub program_id: String,
    pub adapter_id: AdapterId,
    pub status: ProjectStatus,
    /// `None` until an operator activates a bundle. A project cannot check
    /// before then, and says so rather than picking one on its own.
    #[serde(default)]
    pub active_bundle: Option<ActiveBundle>,
    pub created_at_unix_seconds: u64,
    pub updated_at_unix_seconds: u64,
}

impl Project {
    pub fn new(
        project_id: &str,
        name: &str,
        program_id: &str,
        adapter_id: AdapterId,
    ) -> Result<Self> {
        if !crate::storage::valid_id(project_id) {
            bail!("project id {project_id:?} is not a valid identifier");
        }
        validate_name(name)?;
        validate_program_id(program_id)?;

        // The engine chooses the adapter from the program. A declaration that
        // disagrees is a misunderstanding worth refusing at the door rather
        // than at the first check.
        let expected = AdapterId::for_program(program_id);
        if adapter_id != expected {
            bail!("program {program_id} is spoken for by adapter {expected}, not {adapter_id}");
        }

        let now = now_unix_seconds();
        Ok(Self {
            project_id: project_id.to_string(),
            name: name.trim().to_string(),
            chain: Chain::Solana,
            program_id: program_id.to_string(),
            adapter_id,
            status: ProjectStatus::Setup,
            active_bundle: None,
            created_at_unix_seconds: now,
            updated_at_unix_seconds: now,
        })
    }

    /// Ready means a bundle is active. Disabled is an operator's decision and
    /// survives activation; it is never inferred.
    pub fn refresh_status(&mut self) {
        if self.status == ProjectStatus::Disabled {
            return;
        }
        self.status = match self.active_bundle {
            Some(_) => ProjectStatus::Ready,
            None => ProjectStatus::Setup,
        };
    }

    pub fn touch(&mut self) {
        self.updated_at_unix_seconds = now_unix_seconds();
    }
}

pub fn now_unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

// ------------------------------------------------------------------ tokens

/// A project API token, as stored.
///
/// The secret is not here. What is stored is a salted SHA-256 verifier, so a
/// leaked data volume does not hand over the ability to run checks.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectToken {
    pub token_id: String,
    pub project_id: String,
    pub label: String,
    pub verifier: TokenVerifier,
    pub created_at_unix_seconds: u64,
    #[serde(default)]
    pub last_used_at_unix_seconds: Option<u64>,
    #[serde(default)]
    pub revoked_at_unix_seconds: Option<u64>,
}

impl ProjectToken {
    pub fn new(token_id: &str, project_id: &str, label: &str, secret: &str) -> Result<Self> {
        validate_name(label)?;
        Ok(Self {
            token_id: token_id.to_string(),
            project_id: project_id.to_string(),
            label: label.trim().to_string(),
            verifier: TokenVerifier::new(secret),
            created_at_unix_seconds: now_unix_seconds(),
            last_used_at_unix_seconds: None,
            revoked_at_unix_seconds: None,
        })
    }

    pub fn is_revoked(&self) -> bool {
        self.revoked_at_unix_seconds.is_some()
    }
}

/// Salted hash of a token secret.
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

/// A new project token. Returned once, at creation, and never again.
pub fn generate_token() -> String {
    let bytes: [u8; 32] = rand::random();
    format!("eplyx_proj_{}", hex::encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE_PROGRAM: &str = "HopcampquEa7pvG4d6xkVNE2fkMiT9oZmY8T77XkcMBq";
    const STAKE_POOL: &str = "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy";

    fn project(program: &str, adapter: AdapterId) -> Result<Project> {
        Project::new("proj_01ABC", "Example", program, adapter)
    }

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
        let secret = generate_token();
        let token = ProjectToken::new("tok_1", "proj_1", "CI", &secret).unwrap();
        let stored = serde_json::to_string(&token).unwrap();
        assert!(!stored.contains(&secret), "raw token was persisted");
        assert!(stored.contains(&token.verifier.sha256));
    }

    #[test]
    fn verifiers_are_salted_per_token() {
        let secret = generate_token();
        let a = TokenVerifier::new(&secret);
        let b = TokenVerifier::new(&secret);
        assert_ne!(a.salt, b.salt);
        assert_ne!(a.sha256, b.sha256);
        assert!(a.verifies(&secret) && b.verifies(&secret));
    }

    #[test]
    fn a_token_is_recognisable_and_long_enough_to_be_a_secret() {
        let secret = generate_token();
        assert!(secret.starts_with("eplyx_proj_"), "{secret}");
        assert_eq!(secret.len(), "eplyx_proj_".len() + 64);
    }

    #[test]
    fn an_invalid_project_id_is_refused() {
        assert!(Project::new("../escape", "x", FIXTURE_PROGRAM, AdapterId::none()).is_err());
    }

    #[test]
    fn a_program_id_must_be_a_real_pubkey() {
        for bad in ["", "not-base58-0OIl", "SPoo1", &"1".repeat(50)] {
            assert!(
                project(bad, AdapterId::none()).is_err(),
                "accepted {bad:?} as a program id"
            );
        }
        assert!(project(FIXTURE_PROGRAM, AdapterId::none()).is_ok());
    }

    #[test]
    fn a_name_must_be_present_and_bounded() {
        assert!(validate_name("").is_err());
        assert!(validate_name("   ").is_err());
        assert!(validate_name(&"x".repeat(81)).is_err());
        assert!(validate_name("line\nbreak").is_err());
        assert!(validate_name("  Example Lending  ").is_ok());
    }

    /// The adapter is the engine's answer, not the caller's preference.
    #[test]
    fn a_declared_adapter_must_match_what_the_engine_speaks() {
        let stake_pool = AdapterId::for_program(STAKE_POOL);
        assert!(stake_pool.speaks_semantics(), "{stake_pool}");
        assert!(project(STAKE_POOL, stake_pool.clone()).is_ok());
        assert!(
            project(STAKE_POOL, AdapterId::none()).is_err(),
            "a stake-pool project was allowed to declare no adapter"
        );
        assert!(
            project(FIXTURE_PROGRAM, stake_pool).is_err(),
            "a program with no adapter was allowed to claim one"
        );
        assert!(project(FIXTURE_PROGRAM, AdapterId::none()).is_ok());
    }

    #[test]
    fn an_adapter_id_round_trips_and_rejects_nonsense() {
        let parsed = AdapterId::try_from("spl-stake-pool@3".to_string()).unwrap();
        assert_eq!(parsed.to_string(), "spl-stake-pool@3");
        for bad in ["", "noversion", "bad@x", "UPPER@1", "with space@1", "a@-1"] {
            assert!(
                AdapterId::try_from(bad.to_string()).is_err(),
                "accepted {bad:?}"
            );
        }
    }

    #[test]
    fn the_supported_list_comes_from_the_engine() {
        let supported = AdapterId::supported();
        assert!(supported.contains(&AdapterId::for_program(STAKE_POOL)));
        assert!(supported.contains(&AdapterId::none()));
        assert_eq!(
            supported.len(),
            eplyx_engine::protocol::adapters().len() + 1,
            "the list drifted from the engine's registry"
        );
    }

    #[test]
    fn status_follows_the_bundle_pointer_but_never_overrides_disabled() {
        let mut p = project(FIXTURE_PROGRAM, AdapterId::none()).unwrap();
        assert_eq!(p.status, ProjectStatus::Setup);
        p.active_bundle = Some(ActiveBundle {
            bundle_id: "bndl_1".into(),
            bundle_sha256: "abc".into(),
            activated_at_unix_seconds: 1,
        });
        p.refresh_status();
        assert_eq!(p.status, ProjectStatus::Ready);

        p.status = ProjectStatus::Disabled;
        p.refresh_status();
        assert_eq!(
            p.status,
            ProjectStatus::Disabled,
            "disabled was inferred away"
        );
    }
}
