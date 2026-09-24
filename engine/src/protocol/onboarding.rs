//! Small compiled onboarding pieces for bounded protocol interfaces.
//!
//! Recognition and role binding use transaction facts only. Source identity is
//! semantic interface metadata. None of these types selects a runtime, proves
//! an ELF build, or evaluates an economic quantity.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::{
    ingest::transactions::HistoricalTransaction,
    semantic_binding::{RepositoryInterface, SourceBlob},
    types::{AccountMetaSpec, AccountSnapshot, InstructionSpec},
};

#[derive(Clone, Copy, Debug)]
pub struct SourceIdentity {
    pub repository: &'static str,
    pub commit: &'static str,
    pub blobs: &'static [(&'static str, &'static str)],
}

impl SourceIdentity {
    pub fn repository_interface(self) -> RepositoryInterface {
        RepositoryInterface {
            repository: self.repository.into(),
            commit: self.commit.into(),
            source_blobs: self
                .blobs
                .iter()
                .map(|(path, git_blob_sha1)| SourceBlob {
                    path: (*path).into(),
                    git_blob_sha1: (*git_blob_sha1).into(),
                })
                .collect(),
        }
    }
}

/// One immutable semantic interface identity. Several descriptors may be
/// selected explicitly by an adapter for different historical deployments.
#[derive(Clone, Copy, Debug)]
pub struct ProtocolDescriptor {
    pub name: &'static str,
    pub program_id: &'static str,
    pub version: u32,
    pub source: Option<SourceIdentity>,
    pub interactions: &'static [InteractionDescriptor],
}

#[derive(Clone, Copy, Debug)]
pub struct InteractionDescriptor {
    pub id: &'static str,
    pub discriminator: &'static [u8],
    pub outer_index: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OnboardingFailure {
    UnknownProgram,
    UnsupportedInteraction(String),
    AmbiguousInteraction,
    RoleBinding(String),
}

impl fmt::Display for OnboardingFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownProgram => f.write_str("unknown program"),
            Self::UnsupportedInteraction(reason) => write!(f, "unsupported interaction: {reason}"),
            Self::AmbiguousInteraction => f.write_str("ambiguous interaction recognition"),
            Self::RoleBinding(reason) => write!(f, "account role binding failed: {reason}"),
        }
    }
}

impl std::error::Error for OnboardingFailure {}

/// Map onboarding failures into the U15 result without treating an ambiguous
/// compiled recognizer as ordinary lack of coverage.
pub fn evaluation_failure(error: OnboardingFailure) -> anyhow::Result<super::SemanticEvaluation> {
    match error {
        OnboardingFailure::UnknownProgram | OnboardingFailure::UnsupportedInteraction(_) => {
            Ok(super::SemanticEvaluation::Unsupported)
        }
        OnboardingFailure::RoleBinding(reason) => Ok(super::SemanticEvaluation::Unevaluable {
            reason: format!("account role binding failed: {reason}"),
        }),
        error @ OnboardingFailure::AmbiguousInteraction => Err(error.into()),
    }
}

pub fn require_shape(condition: bool, reason: &'static str) -> Result<(), OnboardingFailure> {
    if condition {
        Ok(())
    } else {
        Err(OnboardingFailure::UnsupportedInteraction(reason.into()))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RecognizedInteraction<'a> {
    pub id: &'static str,
    pub outer_index: usize,
    pub matched_discriminator: &'static [u8],
    pub instruction: &'a InstructionSpec,
}

impl ProtocolDescriptor {
    /// Program ID first, then exact discriminator prefix and optional outer
    /// index. More than one match is an error, never a first-match guess.
    pub fn recognize<'a>(
        &self,
        transaction: &'a HistoricalTransaction,
    ) -> Result<RecognizedInteraction<'a>, OnboardingFailure> {
        let mut saw_program = false;
        let mut match_one = None;
        for (outer_index, instruction) in transaction.instructions.iter().enumerate() {
            if instruction.program != self.program_id {
                continue;
            }
            saw_program = true;
            for interaction in self.interactions {
                if interaction
                    .outer_index
                    .is_some_and(|index| index != outer_index)
                    || !instruction.data.starts_with(interaction.discriminator)
                {
                    continue;
                }
                if match_one.is_some() {
                    return Err(OnboardingFailure::AmbiguousInteraction);
                }
                match_one = Some(RecognizedInteraction {
                    id: interaction.id,
                    outer_index,
                    matched_discriminator: interaction.discriminator,
                    instruction,
                });
            }
        }
        match match_one {
            Some(value) => Ok(value),
            None if saw_program => Err(OnboardingFailure::UnsupportedInteraction(
                "no matching instruction discriminator or outer index".into(),
            )),
            None => Err(OnboardingFailure::UnknownProgram),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RoleRule {
    pub index: usize,
    pub address: Option<&'static str>,
    pub signer: Option<bool>,
    pub writable: Option<bool>,
}

impl RoleRule {
    pub const fn address(index: usize, address: &'static str) -> Self {
        Self {
            index,
            address: Some(address),
            signer: None,
            writable: None,
        }
    }
    pub const fn signer(index: usize, value: bool) -> Self {
        Self {
            index,
            address: None,
            signer: Some(value),
            writable: None,
        }
    }
    pub const fn writable(index: usize, value: bool) -> Self {
        Self {
            index,
            address: None,
            signer: None,
            writable: Some(value),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BoundInteraction<'a> {
    pub recognized: RecognizedInteraction<'a>,
    labels: &'static [&'static str],
}

impl<'a> BoundInteraction<'a> {
    pub fn meta(&self, index: usize) -> &'a AccountMetaSpec {
        &self.recognized.instruction.accounts[index]
    }

    pub fn address(&self, index: usize) -> &'a str {
        &self.meta(index).address
    }

    pub fn label_for(&self, address: &str) -> Option<&'static str> {
        self.recognized
            .instruction
            .accounts
            .iter()
            .position(|meta| meta.address == address)
            .map(|index| self.labels[index])
    }

    /// Optional account owner/type constraint at a previously bound role.
    pub fn account<'b>(
        &self,
        snapshots: &'b BTreeMap<String, AccountSnapshot>,
        index: usize,
        owner: &str,
        len: usize,
        discriminator: &[u8],
    ) -> Result<&'b AccountSnapshot, OnboardingFailure> {
        let label = self.labels.get(index).ok_or_else(|| {
            OnboardingFailure::RoleBinding("account role index outside role list".into())
        })?;
        let meta = self
            .recognized
            .instruction
            .accounts
            .get(index)
            .ok_or_else(|| {
                OnboardingFailure::RoleBinding("account role index outside account list".into())
            })?;
        let account = snapshots
            .get(&meta.address)
            .ok_or_else(|| OnboardingFailure::RoleBinding(format!("missing {label} account")))?;
        if account.owner != owner
            || account.data.len() != len
            || !account.data.starts_with(discriminator)
        {
            return Err(OnboardingFailure::RoleBinding(format!(
                "{label} owner or account type differs"
            )));
        }
        Ok(account)
    }
}

pub fn bind_roles<'a>(
    recognized: RecognizedInteraction<'a>,
    labels: &'static [&'static str],
    exact_addresses: Option<&[&str]>,
    rules: &[RoleRule],
    distinct: bool,
) -> Result<BoundInteraction<'a>, OnboardingFailure> {
    let accounts = &recognized.instruction.accounts;
    if accounts.len() != labels.len()
        || exact_addresses.is_some_and(|addresses| addresses.len() != labels.len())
        || labels.iter().collect::<BTreeSet<_>>().len() != labels.len()
    {
        return Err(OnboardingFailure::RoleBinding(
            "ordered role count or labels differ".into(),
        ));
    }
    if distinct
        && accounts
            .iter()
            .map(|meta| &meta.address)
            .collect::<BTreeSet<_>>()
            .len()
            != accounts.len()
    {
        return Err(OnboardingFailure::RoleBinding("account roles alias".into()));
    }
    if let Some(addresses) = exact_addresses {
        for (index, (meta, address)) in accounts.iter().zip(addresses).enumerate() {
            if meta.address != *address {
                return Err(OnboardingFailure::RoleBinding(format!(
                    "{} address differs at role {index}",
                    labels[index]
                )));
            }
        }
    }
    for rule in rules {
        let Some(meta) = accounts.get(rule.index) else {
            return Err(OnboardingFailure::RoleBinding(
                "role rule index outside account list".into(),
            ));
        };
        if rule.address.is_some_and(|address| meta.address != address)
            || rule.signer.is_some_and(|signer| meta.is_signer != signer)
            || rule
                .writable
                .is_some_and(|writable| meta.is_writable != writable)
        {
            return Err(OnboardingFailure::RoleBinding(format!(
                "{} access or address differs",
                labels[rule.index]
            )));
        }
    }
    Ok(BoundInteraction { recognized, labels })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: InteractionDescriptor = InteractionDescriptor {
        id: "one",
        discriminator: &[7, 8],
        outer_index: Some(0),
    };
    const SOURCE: SourceIdentity = SourceIdentity {
        repository: "https://example.test/protocol",
        commit: "1111111111111111111111111111111111111111",
        blobs: &[("interface.json", "2222222222222222222222222222222222222222")],
    };
    const DESCRIPTOR: ProtocolDescriptor = ProtocolDescriptor {
        name: "example",
        program_id: "program",
        version: 1,
        source: Some(SOURCE),
        interactions: &[ONE],
    };
    const LABELS: &[&str] = &["user", "vault"];

    fn transaction() -> HistoricalTransaction {
        HistoricalTransaction {
            signature: String::new(),
            slot: 0,
            block_time: None,
            version: "v0".into(),
            recent_blockhash: String::new(),
            payer: String::new(),
            account_keys: Vec::new(),
            loaded_address_count: 0,
            instructions: vec![InstructionSpec {
                program: "program".into(),
                accounts: vec![
                    AccountMetaSpec {
                        address: "user-address".into(),
                        is_signer: true,
                        is_writable: true,
                    },
                    AccountMetaSpec {
                        address: "vault-address".into(),
                        is_signer: false,
                        is_writable: true,
                    },
                ],
                data: vec![7, 8, 9],
            }],
            inner_instructions: Vec::new(),
            inner_instruction_frames: Vec::new(),
            success: true,
            error: None,
            fee: 0,
            compute_units: None,
            pre_balances: None,
            post_balances: None,
            pre_token_balances: None,
            post_token_balances: None,
            native_value_lamports: None,
            logs: Vec::new(),
        }
    }

    #[test]
    fn recognition_distinguishes_program_shape_and_ambiguity() {
        let tx = transaction();
        let recognized = DESCRIPTOR.recognize(&tx).unwrap();
        assert_eq!((recognized.id, recognized.outer_index), ("one", 0));
        assert_eq!(recognized.matched_discriminator, &[7, 8]);
        let mut unknown = tx.clone();
        unknown.instructions[0].program = "other".into();
        assert!(matches!(
            DESCRIPTOR.recognize(&unknown),
            Err(OnboardingFailure::UnknownProgram)
        ));
        let mut unsupported = tx.clone();
        unsupported.instructions[0].data[0] = 6;
        assert!(matches!(
            DESCRIPTOR.recognize(&unsupported),
            Err(OnboardingFailure::UnsupportedInteraction(_))
        ));
        let ambiguous = ProtocolDescriptor {
            interactions: &[ONE, ONE],
            ..DESCRIPTOR
        };
        assert!(matches!(
            ambiguous.recognize(&tx),
            Err(OnboardingFailure::AmbiguousInteraction)
        ));
        assert!(matches!(
            evaluation_failure(OnboardingFailure::UnsupportedInteraction(
                "other opcode".into()
            ))
            .unwrap(),
            crate::protocol::SemanticEvaluation::Unsupported
        ));
        assert!(evaluation_failure(OnboardingFailure::AmbiguousInteraction).is_err());
    }

    #[test]
    fn ordered_roles_and_optional_owner_type_constraints_fail_closed() {
        let tx = transaction();
        let rules = [
            RoleRule::signer(0, true),
            RoleRule::address(1, "vault-address"),
        ];
        let bound = bind_roles(
            DESCRIPTOR.recognize(&tx).unwrap(),
            LABELS,
            None,
            &rules,
            true,
        )
        .unwrap();
        assert_eq!(bound.label_for("vault-address"), Some("vault"));
        let mut snapshots = BTreeMap::new();
        snapshots.insert(
            "vault-address".into(),
            AccountSnapshot {
                lamports: 0,
                owner: "program".into(),
                data: vec![3, 4],
                executable: false,
                rent_epoch: 0,
            },
        );
        assert!(bound.account(&snapshots, 1, "program", 2, &[3]).is_ok());
        assert!(matches!(
            bound.account(&snapshots, 1, "other", 2, &[3]),
            Err(OnboardingFailure::RoleBinding(_))
        ));
        let mut wrong_signer = tx.clone();
        wrong_signer.instructions[0].accounts[0].is_signer = false;
        assert!(matches!(
            bind_roles(
                DESCRIPTOR.recognize(&wrong_signer).unwrap(),
                LABELS,
                None,
                &rules,
                true
            ),
            Err(OnboardingFailure::RoleBinding(_))
        ));
        let mut aliases = tx.clone();
        aliases.instructions[0].accounts[1].address = "user-address".into();
        assert!(matches!(
            bind_roles(
                DESCRIPTOR.recognize(&aliases).unwrap(),
                LABELS,
                None,
                &[],
                true
            ),
            Err(OnboardingFailure::RoleBinding(_))
        ));
    }

    #[test]
    fn interface_source_can_vary_without_an_execution_identity() {
        let first = SOURCE.repository_interface();
        let later = SourceIdentity {
            commit: "3333333333333333333333333333333333333333",
            ..SOURCE
        }
        .repository_interface();
        assert_ne!(first.commit, later.commit);
        assert_eq!(first.repository, later.repository);
        let binding = |source| {
            crate::semantic_binding::SemanticBinding::ExecutionCorroboratedExternalInterface {
                source,
                facts: vec![crate::semantic_binding::CorroboratedFact::InstructionShape],
                historical_elf_sha256: "a".repeat(64),
                execution_evidence_sha256: "b".repeat(64),
            }
        };
        let first_binding = binding(first);
        let later_binding = binding(later);
        let hashes = |binding: &crate::semantic_binding::SemanticBinding| {
            let crate::semantic_binding::SemanticBinding::ExecutionCorroboratedExternalInterface {
                historical_elf_sha256,
                execution_evidence_sha256,
                ..
            } = binding
            else {
                unreachable!()
            };
            (
                historical_elf_sha256.clone(),
                execution_evidence_sha256.clone(),
            )
        };
        assert_eq!(hashes(&first_binding), hashes(&later_binding));
        assert_ne!(first_binding, later_binding);
        let next = ProtocolDescriptor {
            version: DESCRIPTOR.version + 1,
            ..DESCRIPTOR
        };
        assert_ne!(DESCRIPTOR.version, next.version);
    }
}
