//! The rehearsal world: every account a migration rehearsal executes against, with
//! its provenance.
//!
//! An account is **Observed** (exact bytes at a recorded RPC record and JSON
//! pointer), **SyntheticFixture** (built by a declared recipe step executing real
//! token-program instructions), **CapturedExecutable** (exact deployed program bytes
//! from a pinned capture, loaded into a synthetic world) or, only inside a derived
//! case, **Derived** (a typed mutation of an identified parent). The world kind is
//! explicit: a synthetic fixture can never be described as observed chain state.
use crate::{
    replay::hash_bytes as sha256,
    standard_programs::token as decode,
    standard_programs::token::{CLOCK, UPGRADEABLE_LOADER},
    types::AccountSnapshot,
};
use anyhow::{bail, ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use solana_address::Address;
use std::collections::BTreeMap;

pub const BPF_LOADER_2: &str = "BPFLoader2111111111111111111111111111111111";
pub const BPF_LOADER_1: &str = "BPFLoader1111111111111111111111111111111111";
pub const SYSVAR_OWNER: &str = "Sysvar1111111111111111111111111111111111111";
pub const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";
pub const MAINNET_GENESIS: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";
pub const SYNTHETIC_CLUSTER: &str = "eplyx-synthetic-fixture";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorldKind {
    /// Finalized read-only RPC observations. Accounts may come from different
    /// finalized slots; the world is a composite, never a historical validator bank.
    ObservedCapture,
    /// Deterministic local fixture built from a declared recipe. Never chain state.
    SyntheticFixture,
    /// A base world with typed local mutations for stress or search. Never chain
    /// state, even when the base is observed; the base kind stays recorded.
    Derived,
}

/// The parent of a derived world and the mutations applied to it, in order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DerivedFrom {
    pub base_kind: WorldKind,
    pub base_world_sha256: String,
    pub mutations: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum WorldOrigin {
    Observed {
        artifact: String,
        record: usize,
        pointer: String,
        #[serde(with = "crate::numfmt::u64_string")]
        slot: u64,
    },
    SyntheticFixture {
        recipe_sha256: String,
        step: String,
    },
    CapturedExecutable {
        artifact: String,
        record: usize,
        pointer: String,
        #[serde(with = "crate::numfmt::u64_string")]
        slot: u64,
    },
    /// A typed local mutation. `parent` is the account's origin before the
    /// mutation, or absent when a real instruction created the account locally.
    Derived {
        parent: Option<Box<WorldOrigin>>,
        mutation: String,
    },
}

impl WorldOrigin {
    pub fn is_observed(&self) -> bool {
        matches!(self, Self::Observed { .. })
    }
    pub fn is_derived(&self) -> bool {
        matches!(self, Self::Derived { .. })
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Observed { .. } => "Observed",
            Self::SyntheticFixture { .. } => "SyntheticFixture",
            Self::CapturedExecutable { .. } => "CapturedExecutable",
            Self::Derived { .. } => "Derived",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldAccount {
    pub account: AccountSnapshot,
    pub origin: WorldOrigin,
}

/// The source-mint holder population as enumerated for this world.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PopulationIndex {
    pub source_mint: String,
    /// Every decoded token account of the source mint, sorted, including zero balances.
    pub token_accounts: Vec<String>,
    /// `CompleteForQuery`, `Partial`, `Unavailable` or `Unsupported` (observed), or
    /// `CompleteSyntheticFixture` for a recipe-defined population.
    pub enumeration_completeness: String,
    /// Whether every positive-balance owner account was inspected.
    pub authority_resolution_completeness: String,
    /// Rows the decoder could not turn into a supported token account. Never zero balances.
    pub undecoded_accounts: Vec<String>,
}

pub use crate::standard_programs::clock::CapturedClock as WorldClock;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct World {
    pub kind: WorldKind,
    pub cluster: String,
    pub genesis_hash: String,
    /// The pinned Clock the rehearsal uses (the captured Clock sysvar account).
    pub clock: WorldClock,
    /// Inclusive range of finalized context slots the accounts were observed at.
    #[serde(with = "crate::numfmt::optional_slot_range")]
    pub observed_slots: Option<(u64, u64)>,
    pub accounts: BTreeMap<String, WorldAccount>,
    /// Addresses an observed capture requested and found absent, sorted. A
    /// synthetic fixture is a closed world: anything not present is absent.
    pub inspected_absent: Vec<String>,
    pub population: PopulationIndex,
    pub limitations: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived_from: Option<DerivedFrom>,
}

/// A compact identity row; the world digest covers bytes through their digest.
#[derive(Serialize)]
struct IdentityRow<'a> {
    address: &'a str,
    #[serde(with = "crate::numfmt::u64_string")]
    lamports: u64,
    owner: &'a str,
    executable: bool,
    #[serde(with = "crate::numfmt::u64_string")]
    rent_epoch: u64,
    data_len: usize,
    data_sha256: String,
    origin: &'a WorldOrigin,
}

impl World {
    pub fn get(&self, address: &str) -> Option<&WorldAccount> {
        self.accounts.get(address)
    }

    /// Whether this world knows the address holds no account (as opposed to not
    /// having looked).
    pub fn absence_known(&self, address: &str) -> bool {
        !self.accounts.contains_key(address)
            && (self.base_kind() == WorldKind::SyntheticFixture
                || self
                    .inspected_absent
                    .binary_search(&address.to_string())
                    .is_ok())
    }

    /// The kind of the underlying capture or fixture, through any derivation.
    pub fn base_kind(&self) -> WorldKind {
        self.derived_from
            .as_ref()
            .map_or(self.kind, |from| from.base_kind)
    }

    pub fn snapshot(&self, address: &str) -> Option<&AccountSnapshot> {
        self.accounts.get(address).map(|a| &a.account)
    }

    /// The account in RPC JSON shape, so the existing decoders apply unchanged.
    /// Absent accounts are JSON null, exactly as an RPC reports them.
    pub fn rpc_value(&self, address: &str) -> Value {
        match self.snapshot(address) {
            None => Value::Null,
            Some(account) => rpc_value(account),
        }
    }

    /// Content digest. A derived world is identified by its base world digest,
    /// its ordered mutations, its Clock and every derived account row; the rest of
    /// its accounts are byte-identical to the base by construction.
    pub fn sha256(&self) -> Result<String> {
        let derived = self.kind == WorldKind::Derived;
        let rows: Vec<IdentityRow> = self
            .accounts
            .iter()
            .filter(|(_, entry)| !derived || entry.origin.is_derived())
            .map(|(address, entry)| IdentityRow {
                address,
                lamports: entry.account.lamports,
                owner: &entry.account.owner,
                executable: entry.account.executable,
                rent_epoch: entry.account.rent_epoch,
                data_len: entry.account.data.len(),
                data_sha256: sha256(&entry.account.data),
                origin: &entry.origin,
            })
            .collect();
        if derived {
            return crate::canonical::digest(&json!({
                "kind": self.kind,
                "derived_from": self.derived_from,
                "clock": self.clock,
                "derived_accounts": rows,
                "inspected_absent": self.inspected_absent,
            }));
        }
        crate::canonical::digest(&json!({
            "kind": self.kind,
            "cluster": self.cluster,
            "genesis_hash": self.genesis_hash,
            "clock": self.clock,
            "observed_slots": self.observed_slots,
            "accounts": rows,
            "inspected_absent": self.inspected_absent,
            "population": self.population,
            "derived_from": self.derived_from,
        }))
    }

    /// Structural provenance checks: the world kind and every account origin agree,
    /// the pinned Clock is the Clock account, and observed worlds are on mainnet.
    pub fn validate(&self) -> Result<()> {
        let base = match (&self.kind, &self.derived_from) {
            (WorldKind::Derived, Some(from)) => {
                ensure!(
                    from.base_kind != WorldKind::Derived && !from.mutations.is_empty(),
                    "a derived world needs a non-derived base and at least one mutation"
                );
                from.base_kind
            }
            (WorldKind::Derived, None) => anyhow::bail!("a derived world must name its base"),
            (kind, None) => *kind,
            (_, Some(_)) => anyhow::bail!("only a derived world names a base"),
        };
        let undifferentiated = |origin: &WorldOrigin| -> bool {
            match origin {
                WorldOrigin::Derived { .. } => self.kind == WorldKind::Derived,
                _ => true,
            }
        };
        ensure!(
            self.accounts.values().all(|a| undifferentiated(&a.origin)),
            "only a derived world may contain derived accounts"
        );
        match base {
            WorldKind::ObservedCapture => {
                super::error::compatible(
                    self.cluster == "solana-mainnet" && self.genesis_hash == MAINNET_GENESIS,
                    "an observed world must come from Solana mainnet",
                )?;
                ensure!(
                    self.accounts
                        .values()
                        .all(|a| a.origin.is_observed() || a.origin.is_derived()),
                    "an observed world contains a non-observed account"
                );
            }
            WorldKind::Derived => unreachable!("a derived base was refused above"),
            WorldKind::SyntheticFixture => {
                ensure!(
                    self.cluster == SYNTHETIC_CLUSTER && self.genesis_hash != MAINNET_GENESIS,
                    "a synthetic fixture cannot claim a mainnet identity"
                );
                ensure!(
                    self.accounts.values().all(|a| !a.origin.is_observed()),
                    "a synthetic fixture cannot contain observed accounts"
                );
                ensure!(
                    self.observed_slots.is_none(),
                    "a synthetic fixture has no observed slots"
                );
            }
        }
        let clock = self
            .snapshot(CLOCK)
            .context("the world has no Clock sysvar account")?;
        ensure!(
            clock.owner == SYSVAR_OWNER && WorldClock::from_bytes(&clock.data)? == self.clock,
            "the pinned Clock differs from the Clock sysvar account"
        );
        let mut absent = self.inspected_absent.clone();
        absent.sort();
        absent.dedup();
        ensure!(
            absent == self.inspected_absent
                && absent.iter().all(|a| !self.accounts.contains_key(a)),
            "inspected-absent addresses must be sorted, unique and absent"
        );
        let mut sorted = self.population.token_accounts.clone();
        sorted.sort();
        sorted.dedup();
        ensure!(
            sorted == self.population.token_accounts,
            "population index must be sorted and unique"
        );
        for account in &self.population.token_accounts {
            ensure!(
                self.accounts.contains_key(account),
                "population account {account} is not in the world"
            );
        }
        Ok(())
    }

    /// Executable bytes of a deployed program in this world, with its loader.
    pub fn program(&self, program_id: &str) -> Result<crate::executor::LoadedProgram> {
        let header = self
            .snapshot(program_id)
            .with_context(|| format!("program {program_id} is not in the world"))?;
        ensure!(header.executable, "program {program_id} is not executable");
        let bytes = if header.owner == UPGRADEABLE_LOADER {
            let programdata =
                crate::standard_programs::upgradeable_loader::decode_program(&header.data)?
                    .to_string();
            let expected = crate::standard_programs::upgradeable_loader::programdata_address(
                &program_id.parse()?,
            )
            .to_string();
            ensure!(programdata == expected, "noncanonical ProgramData");
            let state = self
                .snapshot(&programdata)
                .with_context(|| format!("ProgramData {programdata} is not in the world"))?;
            ensure!(
                state.owner == UPGRADEABLE_LOADER && !state.executable,
                "invalid ProgramData owner or executable flag"
            );
            let decoded =
                crate::standard_programs::upgradeable_loader::decode_programdata(&state.data)?;
            ensure!(
                self.base_kind() != WorldKind::ObservedCapture
                    || decoded.deploy_slot < self.clock.slot,
                "ProgramData was deployed after the pinned Clock"
            );
            decoded.bytes
        } else {
            ensure!(
                [BPF_LOADER_2, BPF_LOADER_1].contains(&header.owner.as_str()),
                "unsupported loader {} for {program_id}",
                header.owner
            );
            header.data.clone()
        };
        ensure!(
            bytes.starts_with(b"\x7fELF"),
            "program {program_id} has no ELF"
        );
        Ok(crate::executor::LoadedProgram {
            program_id: program_id.parse()?,
            loader: header.owner.parse()?,
            bytes,
        })
    }

    pub fn mint(&self, address: &str) -> Result<decode::MintConfig> {
        let value = self.rpc_value(address);
        ensure!(!value.is_null(), "mint {address} is not in the world");
        decode::decode_mint(&value)
    }

    pub fn token_account(
        &self,
        address: &str,
        program: &str,
        mint: &str,
        decimals: u8,
    ) -> Result<Option<decode::TokenAccountState>> {
        let value = self.rpc_value(address);
        if value.is_null() {
            return Ok(None);
        }
        decode::decode_token_account(&value, program, mint, decimals).map(Some)
    }
}

pub fn rpc_value(account: &AccountSnapshot) -> Value {
    json!({
        "lamports": account.lamports,
        "owner": account.owner,
        "executable": account.executable,
        "rentEpoch": account.rent_epoch,
        "data": [STANDARD.encode(&account.data), "base64"],
        "space": account.data.len(),
    })
}

pub fn snapshot_from_rpc(raw: &Value) -> Result<AccountSnapshot> {
    if raw.is_null() {
        bail!("absent account has no snapshot");
    }
    Ok(AccountSnapshot {
        lamports: raw["lamports"].as_u64().context("missing lamports")?,
        owner: raw["owner"]
            .as_str()
            .context("missing runtime owner")?
            .into(),
        data: decode::raw_account_bytes(raw)?,
        executable: raw["executable"]
            .as_bool()
            .context("missing executable flag")?,
        rent_epoch: raw["rentEpoch"].as_u64().unwrap_or(0),
    })
}

/// The canonical associated token account for an owner, token program and mint.
/// The token program is part of the derivation: an ATA derived for one token
/// program is never valid for the other.
pub fn associated_token_address(owner: &str, token_program: &str, mint: &str) -> Result<String> {
    Ok(Address::find_program_address(
        &[
            owner.parse::<Address>()?.as_ref(),
            token_program.parse::<Address>()?.as_ref(),
            mint.parse::<Address>()?.as_ref(),
        ],
        &crate::standard_programs::token::ATA_PROGRAM.parse()?,
    )
    .0
    .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::standard_programs::token::{LEGACY_PROGRAM, TOKEN_2022_PROGRAM};

    #[test]
    fn ata_derivation_depends_on_the_token_program() {
        let owner = "2wCvQzHiDHAHTvzwPeof9H3uEzq8Bzvg38DFbvZMGkuj";
        let mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        let legacy = associated_token_address(owner, LEGACY_PROGRAM, mint).unwrap();
        let token_2022 = associated_token_address(owner, TOKEN_2022_PROGRAM, mint).unwrap();
        assert_ne!(legacy, token_2022);
    }

    #[test]
    fn clock_bytes_round_trip() {
        let clock = WorldClock {
            slot: 7,
            epoch_start_timestamp: -3,
            epoch: 2,
            leader_schedule_epoch: 3,
            unix_timestamp: 1_760_000_000,
        };
        assert_eq!(WorldClock::from_bytes(&clock.bytes()).unwrap(), clock);
    }
}
