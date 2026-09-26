//! Deterministic synthetic migration fixtures built with real token programs.
//!
//! A recipe declares mints, wallets, SPL multisigs, program-owned owners and token
//! accounts. The builder executes the real SPL Token / Token-2022 / Associated Token
//! Account instructions (initialize mint and extensions, create accounts, mint,
//! approve, freeze, reallocate, enable memo/CPI guard, set authority, pause) in a
//! fresh LiteSVM loaded with pinned executable bytes, then exports the resulting
//! accounts as a [`World`] of kind `SyntheticFixture`. Keys are derived from public
//! labels; nothing here is a real key, and nothing here is chain state.
use super::world::{
    PopulationIndex, World, WorldAccount, WorldClock, WorldKind, WorldOrigin, SYNTHETIC_CLUSTER,
    SYSTEM_PROGRAM, SYSVAR_OWNER,
};
use crate::{
    executor::LoadedProgram,
    replay::hash_bytes as sha256,
    standard_programs::token::{self as decode, LEGACY_PROGRAM, TOKEN_2022_PROGRAM},
    standard_programs::token::{ATA_PROGRAM, CLOCK, UPGRADEABLE_LOADER},
    types::AccountSnapshot,
};
use anyhow::{anyhow, bail, ensure, Context, Result};
use litesvm::LiteSVM;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use solana_account::Account;
use solana_address::Address;
use solana_instruction::{account_meta::AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_program_pack::Pack;
use solana_signer::Signer;
use solana_transaction::Transaction;
use spl_token_2022_interface::{
    extension::{
        cpi_guard, default_account_state, interest_bearing_mint, memo_transfer, metadata_pointer,
        pausable, transfer_fee, transfer_hook, ExtensionType,
    },
    instruction as token,
    state::{Account as TokenAccount, AccountState, Mint},
};
use std::collections::{BTreeMap, BTreeSet};

pub const RECIPE_SCHEMA: u32 = 1;
pub const BUILDER_VERSION: &str = "eplyx-migration-fixture/v1";
pub const MAX_RECIPE_BYTES: usize = 256 * 1024;
const MAX_ITEMS: usize = 512;

/// Pinned mainnet executables used by synthetic fixtures. Exact bytes, from a
/// finalized read-only capture already in this repository.
pub const PROGRAM_CAPTURE: &str = "fixtures/migration/pinned-programs/live-market.capture.json";
pub const PROGRAM_CAPTURE_SHA256: &str =
    "3d7ba0f24d424bcc0042495f5cdf39555c0b09569d5dfcecc8017097a8604b52";
pub const PROGRAM_CAPTURE_RECORD: usize = 6;
/// (program id, executable ELF sha-256) of the captured deployed programs.
pub const PINNED_PROGRAMS: [(&str, &str); 3] = [
    (
        LEGACY_PROGRAM,
        "8190d3f7ceb6cb7a7a8d8924bff89f9f611e15ce1f806f2b6237f3311a98f697",
    ),
    (
        TOKEN_2022_PROGRAM,
        "0999dbf708971e723b08d1caafc988826a59c6001ed6dc02260da07defbe1469",
    ),
    (
        ATA_PROGRAM,
        "6804554e69fd3a58caa191dc4a58f4c67223d30ca28ab8987f39fc18d2f7374d",
    ),
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RecipeClock {
    pub slot: String,
    pub unix_timestamp: String,
    pub epoch: String,
}

/// A key reference: a public label (deterministic synthetic key), an explicit
/// address, or the migration authority the adapter derives for this package.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum KeyRef {
    Label {
        label: String,
    },
    Address {
        address: String,
    },
    MigrationAuthority {
        #[serde(rename = "migrationAuthority")]
        migration_authority: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WalletSpec {
    pub label: String,
    pub lamports: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProgramOwnedSpec {
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MultisigSpec {
    pub label: String,
    pub token_program: String,
    pub threshold: u8,
    pub signers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum MintExtension {
    #[serde(rename_all = "camelCase")]
    TransferFee {
        bps: u16,
        maximum_fee: String,
    },
    PermanentDelegate {
        delegate: KeyRef,
    },
    NonTransferable,
    DefaultAccountState {
        state: DefaultState,
    },
    TransferHook {
        program: KeyRef,
    },
    Pausable {
        authority: KeyRef,
    },
    #[serde(rename_all = "camelCase")]
    InterestBearing {
        rate_bps: i16,
    },
    MintCloseAuthority {
        authority: KeyRef,
    },
    MetadataPointer {
        authority: KeyRef,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DefaultState {
    Initialized,
    Frozen,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MintSpec {
    pub label: String,
    pub token_program: String,
    pub decimals: u8,
    pub mint_authority: KeyRef,
    #[serde(default)]
    pub freeze_authority: Option<KeyRef>,
    #[serde(default)]
    pub extensions: Vec<MintExtension>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountLayout {
    /// Created through the Associated Token Account program.
    Associated,
    /// Allocated at a label-derived address and initialized with InitializeAccount3.
    Explicit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountExtension {
    MemoTransfer,
    CpiGuard,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DelegateSpec {
    pub delegate: KeyRef,
    pub amount: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TokenAccountSpec {
    pub label: String,
    pub mint: String,
    pub owner: KeyRef,
    pub layout: AccountLayout,
    pub amount: String,
    #[serde(default)]
    pub extensions: Vec<AccountExtension>,
    #[serde(default)]
    pub delegate: Option<DelegateSpec>,
    #[serde(default)]
    pub frozen: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum PostStep {
    /// SetAuthority(MintTokens) on a mint, executed by its current mint authority.
    SetMintAuthority { mint: String, to: KeyRef },
    /// Pause a Pausable mint.
    Pause { mint: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProgramSource {
    /// Exact deployed mainnet bytes from the pinned repository capture.
    PinnedMainnetCapture,
    /// The official SPL releases bundled with LiteSVM 0.16 (no repository needed).
    LiteSvmBundled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Recipe {
    pub schema_version: u32,
    pub id: String,
    pub description: String,
    pub programs: ProgramSource,
    pub clock: RecipeClock,
    /// Label of the source mint whose holders form the population.
    pub population_mint: String,
    #[serde(default)]
    pub wallets: Vec<WalletSpec>,
    #[serde(default)]
    pub program_owned: Vec<ProgramOwnedSpec>,
    #[serde(default)]
    pub multisigs: Vec<MultisigSpec>,
    pub mints: Vec<MintSpec>,
    pub token_accounts: Vec<TokenAccountSpec>,
    #[serde(default)]
    pub post_steps: Vec<PostStep>,
}

/// Values the recipe may reference that come from the package, not the recipe.
#[derive(Clone, Debug, Default)]
pub struct FixtureContext {
    pub migration_authority: Option<String>,
}

fn label_ok(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 48
        && label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl Recipe {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() <= MAX_RECIPE_BYTES,
            "fixture recipe exceeds 256 KiB"
        );
        let recipe: Recipe = serde_json::from_slice(bytes).context("invalid fixture recipe")?;
        recipe.validate()?;
        Ok(recipe)
    }

    pub fn sha256(&self) -> Result<String> {
        crate::canonical::digest(self)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == RECIPE_SCHEMA,
            "unsupported fixture recipe schema"
        );
        ensure!(label_ok(&self.id), "fixture id must be [a-z0-9-]{{1,48}}");
        ensure!(
            self.description.len() <= 400,
            "fixture description too long"
        );
        let total = self.wallets.len()
            + self.program_owned.len()
            + self.multisigs.len()
            + self.mints.len()
            + self.token_accounts.len();
        ensure!(total <= MAX_ITEMS, "fixture recipe has too many items");
        let mut labels = BTreeSet::new();
        for label in self
            .wallets
            .iter()
            .map(|w| &w.label)
            .chain(self.program_owned.iter().map(|p| &p.label))
            .chain(self.multisigs.iter().map(|m| &m.label))
            .chain(self.mints.iter().map(|m| &m.label))
            .chain(self.token_accounts.iter().map(|t| &t.label))
        {
            ensure!(label_ok(label), "invalid fixture label {label:?}");
            ensure!(
                labels.insert(label.clone()),
                "duplicate fixture label {label}"
            );
        }
        ensure!(
            self.mints.iter().any(|m| m.label == self.population_mint),
            "populationMint must name a declared mint"
        );
        for value in [
            &self.clock.slot,
            &self.clock.unix_timestamp,
            &self.clock.epoch,
        ] {
            crate::migration::spec::canonical_u64(value)?;
        }
        for wallet in &self.wallets {
            crate::migration::spec::canonical_u64(&wallet.lamports)?;
        }
        for mint in &self.mints {
            TokenProgramKind::of(&mint.token_program)?;
            if mint.token_program == LEGACY_PROGRAM {
                ensure!(
                    mint.extensions.is_empty(),
                    "legacy SPL Token mints have no extensions"
                );
            }
        }
        for account in &self.token_accounts {
            crate::migration::spec::canonical_u64(&account.amount)?;
            ensure!(
                self.mints.iter().any(|m| m.label == account.mint),
                "token account {} names an unknown mint",
                account.label
            );
        }
        for multisig in &self.multisigs {
            TokenProgramKind::of(&multisig.token_program)?;
            ensure!(
                !multisig.signers.is_empty()
                    && multisig.signers.len() <= 11
                    && multisig.threshold >= 1
                    && usize::from(multisig.threshold) <= multisig.signers.len(),
                "invalid multisig threshold"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TokenProgramKind {
    Legacy,
    Token2022,
}
impl TokenProgramKind {
    fn of(address: &str) -> Result<Self> {
        match address {
            LEGACY_PROGRAM => Ok(Self::Legacy),
            TOKEN_2022_PROGRAM => Ok(Self::Token2022),
            other => bail!("unsupported fixture token program {other}"),
        }
    }
}

/// Deterministic synthetic key for a public label. Its seed is public, so it is
/// never a real key; it only makes fixture addresses reproducible.
pub fn label_key(recipe_id: &str, label: &str) -> Keypair {
    let digest = sha256(format!("eplyx-fixture/v1/{recipe_id}/{label}").as_bytes());
    let mut seed = [0u8; 32];
    for (index, byte) in seed.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&digest[index * 2..index * 2 + 2], 16).unwrap_or(0);
    }
    Keypair::new_from_array(seed)
}

pub fn label_address(recipe_id: &str, label: &str) -> Address {
    label_key(recipe_id, label).pubkey()
}

/// The owning program of synthetic program-owned authorities.
pub fn fixture_protocol_program(recipe_id: &str) -> Address {
    label_address(recipe_id, "fixture-protocol-program")
}

pub fn program_owned_address(recipe_id: &str, label: &str) -> Address {
    Address::find_program_address(
        &[b"eplyx-fixture-program-owned", label.as_bytes()],
        &fixture_protocol_program(recipe_id),
    )
    .0
}

/// The pinned deployed executables, verified byte for byte.
pub struct PinnedPrograms {
    pub accounts: Vec<(String, AccountSnapshot, WorldOrigin)>,
}

impl PinnedPrograms {
    pub fn load(source: ProgramSource) -> Result<Self> {
        match source {
            ProgramSource::PinnedMainnetCapture => Self::captured(),
            ProgramSource::LiteSvmBundled => Self::bundled(),
        }
    }

    fn captured() -> Result<Self> {
        let path = crate::repo_root().join(PROGRAM_CAPTURE);
        let bytes = std::fs::read(&path).with_context(|| {
            format!("pinned program capture {PROGRAM_CAPTURE} is unavailable; run scripts/import-migration-fixtures.py --sta <pinned-archive>")
        })?;
        ensure!(
            sha256(&bytes) == PROGRAM_CAPTURE_SHA256,
            "pinned program capture digest mismatch"
        );
        let capture: Value = serde_json::from_slice(&bytes)?;
        let record = &capture["observations"][PROGRAM_CAPTURE_RECORD];
        ensure!(
            record["method"] == "getMultipleAccounts",
            "unexpected pinned program record"
        );
        let slot = record["result"]["context"]["slot"]
            .as_u64()
            .context("missing capture slot")?;
        let addresses = record["params"][0]
            .as_array()
            .context("missing capture addresses")?;
        let values = record["result"]["value"]
            .as_array()
            .context("missing capture values")?;
        let mut wanted: BTreeSet<String> = PINNED_PROGRAMS
            .iter()
            .map(|(id, _)| id.to_string())
            .collect();
        // ProgramData accounts of the upgradeable token programs.
        for (address, value) in addresses.iter().zip(values) {
            let address = address.as_str().unwrap_or_default();
            if (address == LEGACY_PROGRAM || address == TOKEN_2022_PROGRAM)
                && value["owner"] == UPGRADEABLE_LOADER
            {
                let header = decode::raw_account_bytes(value)?;
                wanted.insert(
                    crate::standard_programs::upgradeable_loader::decode_program(&header)?
                        .to_string(),
                );
            }
        }
        let mut accounts = Vec::new();
        for (index, (address, value)) in addresses.iter().zip(values).enumerate() {
            let address = address.as_str().unwrap_or_default().to_string();
            if !wanted.contains(&address) {
                continue;
            }
            accounts.push((
                address,
                super::world::snapshot_from_rpc(value)?,
                WorldOrigin::CapturedExecutable {
                    artifact: PROGRAM_CAPTURE.into(),
                    record: PROGRAM_CAPTURE_RECORD,
                    pointer: format!("/result/value/{index}"),
                    slot,
                },
            ));
        }
        ensure!(
            accounts.len() == wanted.len(),
            "pinned program capture is incomplete"
        );
        let pinned = Self { accounts };
        pinned.verify()?;
        Ok(pinned)
    }

    fn bundled() -> Result<Self> {
        let svm = LiteSVM::new();
        let mut accounts = Vec::new();
        for (id, _) in PINNED_PROGRAMS {
            let address: Address = id.parse()?;
            let account = svm
                .get_account(&address)
                .with_context(|| format!("LiteSVM does not bundle {id}"))?;
            let origin = WorldOrigin::SyntheticFixture {
                recipe_sha256: String::new(),
                step: format!("litesvm-0.16-bundled:{id}"),
            };
            if account.owner.to_string() == UPGRADEABLE_LOADER {
                let programdata =
                    crate::standard_programs::upgradeable_loader::decode_program(&account.data)?;
                let state = svm
                    .get_account(&programdata)
                    .context("missing bundled ProgramData")?;
                accounts.push((programdata.to_string(), snapshot(&state), origin.clone()));
            }
            accounts.push((id.to_string(), snapshot(&account), origin));
        }
        Ok(Self { accounts })
    }

    /// Pinned mainnet bytes must match their recorded executable digests.
    fn verify(&self) -> Result<()> {
        let world: BTreeMap<String, WorldAccount> = self
            .accounts
            .iter()
            .map(|(address, account, origin)| {
                (
                    address.clone(),
                    WorldAccount {
                        account: account.clone(),
                        origin: origin.clone(),
                    },
                )
            })
            .collect();
        let probe = World {
            kind: WorldKind::SyntheticFixture,
            cluster: SYNTHETIC_CLUSTER.into(),
            genesis_hash: "program-verification".into(),
            clock: WorldClock {
                slot: u64::MAX,
                epoch_start_timestamp: 0,
                epoch: 0,
                leader_schedule_epoch: 0,
                unix_timestamp: 0,
            },
            observed_slots: None,
            accounts: world,
            inspected_absent: vec![],
            population: PopulationIndex {
                source_mint: String::new(),
                token_accounts: vec![],
                enumeration_completeness: String::new(),
                authority_resolution_completeness: String::new(),
                undecoded_accounts: vec![],
            },
            limitations: vec![],
            derived_from: None,
        };
        for (id, digest) in PINNED_PROGRAMS {
            let program = probe.program(id)?;
            ensure!(
                sha256(&program.bytes) == digest,
                "pinned executable digest mismatch for {id}"
            );
        }
        Ok(())
    }

    pub fn loaded(&self, clock_slot: u64) -> Result<Vec<LoadedProgram>> {
        let world = World {
            kind: WorldKind::SyntheticFixture,
            cluster: SYNTHETIC_CLUSTER.into(),
            genesis_hash: "program-loading".into(),
            clock: WorldClock {
                slot: clock_slot,
                epoch_start_timestamp: 0,
                epoch: 0,
                leader_schedule_epoch: 0,
                unix_timestamp: 0,
            },
            observed_slots: None,
            inspected_absent: vec![],
            accounts: self
                .accounts
                .iter()
                .map(|(address, account, origin)| {
                    (
                        address.clone(),
                        WorldAccount {
                            account: account.clone(),
                            origin: origin.clone(),
                        },
                    )
                })
                .collect(),
            population: PopulationIndex {
                source_mint: String::new(),
                token_accounts: vec![],
                enumeration_completeness: String::new(),
                authority_resolution_completeness: String::new(),
                undecoded_accounts: vec![],
            },
            limitations: vec![],
            derived_from: None,
        };
        PINNED_PROGRAMS
            .iter()
            .map(|(id, _)| world.program(id))
            .collect()
    }
}

fn snapshot(account: &Account) -> AccountSnapshot {
    AccountSnapshot {
        lamports: account.lamports,
        owner: account.owner.to_string(),
        data: account.data.clone(),
        executable: account.executable,
        rent_epoch: account.rent_epoch,
    }
}

struct Builder<'a> {
    recipe: &'a Recipe,
    context: &'a FixtureContext,
    svm: LiteSVM,
    payer: Address,
    /// Every exported address with the step that produced it.
    exported: BTreeMap<String, String>,
}

impl Builder<'_> {
    fn key(&self, reference: &KeyRef) -> Result<Address> {
        match reference {
            KeyRef::Label { label } => {
                if let Some(owned) = self.recipe.program_owned.iter().find(|p| &p.label == label) {
                    return Ok(program_owned_address(&self.recipe.id, &owned.label));
                }
                Ok(label_address(&self.recipe.id, label))
            }
            KeyRef::Address { address } => Ok(address.parse()?),
            KeyRef::MigrationAuthority {
                migration_authority: true,
            } => self
                .context
                .migration_authority
                .as_deref()
                .context(
                    "this fixture references the migration authority, which the package derives",
                )?
                .parse()
                .map_err(Into::into),
            KeyRef::MigrationAuthority { .. } => bail!("migrationAuthority must be true"),
        }
    }

    fn mint(&self, label: &str) -> Result<&MintSpec> {
        self.recipe
            .mints
            .iter()
            .find(|m| m.label == label)
            .with_context(|| format!("unknown mint {label}"))
    }

    fn send(&mut self, step: &str, instructions: Vec<Instruction>) -> Result<()> {
        let message = Message::new(&instructions, Some(&self.payer));
        let result = self
            .svm
            .send_transaction(Transaction::new_unsigned(message));
        if let Err(failure) = result {
            bail!(
                "fixture step {step} failed in the real token program: {:?}\n{}",
                failure.err,
                failure.meta.logs.join("\n")
            );
        }
        Ok(())
    }

    fn allocate(&mut self, address: &Address, owner: &str, len: usize) -> Result<()> {
        let lamports = self.svm.minimum_balance_for_rent_exemption(len);
        self.svm
            .set_account(
                *address,
                Account {
                    lamports,
                    data: vec![0; len],
                    owner: owner.parse()?,
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .map_err(|e| anyhow!("cannot allocate fixture account: {e:?}"))
    }

    fn export(&mut self, address: Address, step: String) {
        self.exported.entry(address.to_string()).or_insert(step);
    }
}

fn mint_extension_type(extension: &MintExtension) -> ExtensionType {
    match extension {
        MintExtension::TransferFee { .. } => ExtensionType::TransferFeeConfig,
        MintExtension::PermanentDelegate { .. } => ExtensionType::PermanentDelegate,
        MintExtension::NonTransferable => ExtensionType::NonTransferable,
        MintExtension::DefaultAccountState { .. } => ExtensionType::DefaultAccountState,
        MintExtension::TransferHook { .. } => ExtensionType::TransferHook,
        MintExtension::Pausable { .. } => ExtensionType::Pausable,
        MintExtension::InterestBearing { .. } => ExtensionType::InterestBearingConfig,
        MintExtension::MintCloseAuthority { .. } => ExtensionType::MintCloseAuthority,
        MintExtension::MetadataPointer { .. } => ExtensionType::MetadataPointer,
    }
}

/// Build the synthetic world. Deterministic for a recipe and context.
pub fn build(recipe: &Recipe, context: &FixtureContext) -> Result<World> {
    recipe.validate()?;
    let recipe_sha256 = recipe.sha256()?;
    let programs = PinnedPrograms::load(recipe.programs)?;
    let clock = WorldClock {
        slot: crate::migration::spec::canonical_u64(&recipe.clock.slot)?,
        epoch_start_timestamp: i64::try_from(crate::migration::spec::canonical_u64(
            &recipe.clock.unix_timestamp,
        )?)?,
        epoch: crate::migration::spec::canonical_u64(&recipe.clock.epoch)?,
        leader_schedule_epoch: crate::migration::spec::canonical_u64(&recipe.clock.epoch)? + 1,
        unix_timestamp: i64::try_from(crate::migration::spec::canonical_u64(
            &recipe.clock.unix_timestamp,
        )?)?,
    };
    let mut svm = LiteSVM::new()
        .with_sigverify(false)
        .with_blockhash_check(false)
        .with_transaction_history(0);
    svm.set_sysvar(&clock.clock());
    for program in programs.loaded(clock.slot)? {
        svm.add_program_with_loader(program.program_id, &program.bytes, program.loader)
            .map_err(|e| anyhow!("cannot load fixture program {}: {e:?}", program.program_id))?;
    }
    let payer = label_address(&recipe.id, "fixture-builder-payer");
    svm.set_account(
        payer,
        Account {
            lamports: 1_000_000_000_000,
            data: vec![],
            owner: SYSTEM_PROGRAM.parse()?,
            executable: false,
            rent_epoch: 0,
        },
    )
    .map_err(|e| anyhow!("cannot fund fixture payer: {e:?}"))?;
    let mut b = Builder {
        recipe,
        context,
        svm,
        payer,
        exported: BTreeMap::new(),
    };

    for wallet in &recipe.wallets {
        let address = label_address(&recipe.id, &wallet.label);
        b.svm
            .set_account(
                address,
                Account {
                    lamports: crate::migration::spec::canonical_u64(&wallet.lamports)?,
                    data: vec![],
                    owner: SYSTEM_PROGRAM.parse()?,
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .map_err(|e| anyhow!("cannot create fixture wallet: {e:?}"))?;
        b.export(address, format!("wallet:{}", wallet.label));
    }
    let protocol = fixture_protocol_program(&recipe.id);
    for owned in &recipe.program_owned {
        let address = program_owned_address(&recipe.id, &owned.label);
        b.allocate(&address, &protocol.to_string(), 8)?;
        b.export(address, format!("program-owned:{}", owned.label));
    }
    for multisig in &recipe.multisigs {
        let address = label_address(&recipe.id, &multisig.label);
        b.allocate(
            &address,
            &multisig.token_program,
            spl_token_2022_interface::state::Multisig::LEN,
        )?;
        let signers: Vec<Address> = multisig
            .signers
            .iter()
            .map(|s| label_address(&recipe.id, s))
            .collect();
        let refs: Vec<&Address> = signers.iter().collect();
        let ix = token::initialize_multisig2(
            &multisig.token_program.parse()?,
            &address,
            &refs,
            multisig.threshold,
        )?;
        b.send(&format!("multisig:{}", multisig.label), vec![ix])?;
        b.export(address, format!("multisig:{}", multisig.label));
    }

    for mint in &recipe.mints {
        let program: Address = mint.token_program.parse()?;
        let address = label_address(&recipe.id, &mint.label);
        let types: Vec<ExtensionType> = mint.extensions.iter().map(mint_extension_type).collect();
        let len = if mint.token_program == LEGACY_PROGRAM {
            Mint::LEN
        } else {
            ExtensionType::try_calculate_account_len::<Mint>(&types)?
        };
        b.allocate(&address, &mint.token_program, len)?;
        let mut instructions = Vec::new();
        for extension in &mint.extensions {
            instructions.push(match extension {
                MintExtension::TransferFee { bps, maximum_fee } => {
                    let authority = b.key(&mint.mint_authority)?;
                    transfer_fee::instruction::initialize_transfer_fee_config(
                        &program,
                        &address,
                        Some(&authority),
                        Some(&authority),
                        *bps,
                        crate::migration::spec::canonical_u64(maximum_fee)?,
                    )?
                }
                MintExtension::PermanentDelegate { delegate } => {
                    token::initialize_permanent_delegate(&program, &address, &b.key(delegate)?)?
                }
                MintExtension::NonTransferable => {
                    token::initialize_non_transferable_mint(&program, &address)?
                }
                MintExtension::DefaultAccountState { state } => {
                    default_account_state::instruction::initialize_default_account_state(
                        &program,
                        &address,
                        &match state {
                            DefaultState::Initialized => AccountState::Initialized,
                            DefaultState::Frozen => AccountState::Frozen,
                        },
                    )?
                }
                MintExtension::TransferHook { program: hook } => {
                    transfer_hook::instruction::initialize(
                        &program,
                        &address,
                        Some(b.key(&mint.mint_authority)?),
                        Some(b.key(hook)?),
                    )?
                }
                MintExtension::Pausable { authority } => {
                    pausable::instruction::initialize(&program, &address, &b.key(authority)?)?
                }
                MintExtension::InterestBearing { rate_bps } => {
                    interest_bearing_mint::instruction::initialize(
                        &program,
                        &address,
                        Some(b.key(&mint.mint_authority)?),
                        *rate_bps,
                    )?
                }
                MintExtension::MintCloseAuthority { authority } => {
                    token::initialize_mint_close_authority(
                        &program,
                        &address,
                        Some(&b.key(authority)?),
                    )?
                }
                MintExtension::MetadataPointer { authority } => {
                    metadata_pointer::instruction::initialize(
                        &program,
                        &address,
                        Some(b.key(authority)?),
                        Some(address),
                    )?
                }
            });
        }
        let freeze = mint
            .freeze_authority
            .as_ref()
            .map(|k| b.key(k))
            .transpose()?;
        instructions.push(token::initialize_mint2(
            &program,
            &address,
            &b.key(&mint.mint_authority)?,
            freeze.as_ref(),
            mint.decimals,
        )?);
        b.send(&format!("mint:{}", mint.label), instructions)?;
        b.export(address, format!("mint:{}", mint.label));
    }

    for account in &recipe.token_accounts {
        let mint = b.mint(&account.mint)?.clone();
        let program: Address = mint.token_program.parse()?;
        let mint_address = label_address(&recipe.id, &mint.label);
        let owner = b.key(&account.owner)?;
        let step = format!("token-account:{}", account.label);
        let address = match account.layout {
            AccountLayout::Associated => {
                let ata: Address = super::world::associated_token_address(
                    &owner.to_string(),
                    &mint.token_program,
                    &mint_address.to_string(),
                )?
                .parse()?;
                let ix = Instruction {
                    program_id: ATA_PROGRAM.parse()?,
                    accounts: vec![
                        AccountMeta::new(b.payer, true),
                        AccountMeta::new(ata, false),
                        AccountMeta::new_readonly(owner, false),
                        AccountMeta::new_readonly(mint_address, false),
                        AccountMeta::new_readonly(SYSTEM_PROGRAM.parse()?, false),
                        AccountMeta::new_readonly(program, false),
                    ],
                    data: vec![1],
                };
                b.send(&step, vec![ix])?;
                ata
            }
            AccountLayout::Explicit => {
                let address = label_address(&recipe.id, &account.label);
                let len = if mint.token_program == LEGACY_PROGRAM {
                    TokenAccount::LEN
                } else {
                    let mint_bytes = b
                        .svm
                        .get_account(&mint_address)
                        .context("fixture mint missing")?
                        .data;
                    spl_token_2022_interface::extension::account_len::try_calculate_account_len_from_mint_data(
                        &mint_bytes,
                        &[],
                    )
                    .map_err(|e| anyhow!("cannot size fixture token account: {e:?}"))?
                };
                b.allocate(&address, &mint.token_program, len)?;
                b.send(
                    &step,
                    vec![token::initialize_account3(
                        &program,
                        &address,
                        &mint_address,
                        &owner,
                    )?],
                )?;
                address
            }
        };
        // Default-frozen mints create frozen accounts. Thaw only when funding,
        // approval or extensions need it, then settle on the recipe's final state.
        let amount = crate::migration::spec::canonical_u64(&account.amount)?;
        let freeze_authority = mint
            .freeze_authority
            .as_ref()
            .map(|k| b.key(k))
            .transpose()?;
        let mut frozen_now = account_default_frozen(&mint);
        let needs_activity =
            amount > 0 || account.delegate.is_some() || !account.extensions.is_empty();
        let freeze_ix = |freeze: bool| -> Result<Instruction> {
            let authority =
                freeze_authority.context("changing freeze state needs a mint freeze authority")?;
            Ok(if freeze {
                token::freeze_account(&program, &address, &mint_address, &authority, &[])?
            } else {
                token::thaw_account(&program, &address, &mint_address, &authority, &[])?
            })
        };
        if frozen_now && needs_activity {
            b.send(&format!("{step}:thaw"), vec![freeze_ix(false)?])?;
            frozen_now = false;
        }
        if !account.extensions.is_empty() {
            let types: Vec<ExtensionType> = account
                .extensions
                .iter()
                .map(|e| match e {
                    AccountExtension::MemoTransfer => ExtensionType::MemoTransfer,
                    AccountExtension::CpiGuard => ExtensionType::CpiGuard,
                })
                .collect();
            let mut instructions = vec![token::reallocate(
                &program,
                &address,
                &b.payer,
                &owner,
                &[],
                &types,
            )?];
            for extension in &account.extensions {
                instructions.push(match extension {
                    AccountExtension::MemoTransfer => {
                        memo_transfer::instruction::enable_required_transfer_memos(
                            &program,
                            &address,
                            &owner,
                            &[],
                        )?
                    }
                    AccountExtension::CpiGuard => {
                        cpi_guard::instruction::enable_cpi_guard(&program, &address, &owner, &[])?
                    }
                });
            }
            b.send(&format!("{step}:extensions"), instructions)?;
        }
        if amount > 0 {
            let ix = token::mint_to_checked(
                &program,
                &mint_address,
                &address,
                &b.key(&mint.mint_authority)?,
                &[],
                amount,
                mint.decimals,
            )?;
            b.send(&format!("{step}:mint"), vec![ix])?;
        }
        if let Some(delegate) = &account.delegate {
            let ix = token::approve_checked(
                &program,
                &address,
                &mint_address,
                &b.key(&delegate.delegate)?,
                &owner,
                &[],
                crate::migration::spec::canonical_u64(&delegate.amount)?,
                mint.decimals,
            )?;
            b.send(&format!("{step}:approve"), vec![ix])?;
        }
        if account.frozen != frozen_now {
            b.send(
                &format!("{step}:{}", if account.frozen { "freeze" } else { "thaw" }),
                vec![freeze_ix(account.frozen)?],
            )?;
        }
        b.export(address, step);
    }

    for (index, step) in recipe.post_steps.iter().enumerate() {
        match step {
            PostStep::SetMintAuthority { mint, to } => {
                let spec = b.mint(mint)?.clone();
                let ix = token::set_authority(
                    &spec.token_program.parse()?,
                    &label_address(&recipe.id, mint),
                    Some(&b.key(to)?),
                    token::AuthorityType::MintTokens,
                    &b.key(&spec.mint_authority)?,
                    &[],
                )?;
                b.send(&format!("post:{index}:set-mint-authority"), vec![ix])?;
            }
            PostStep::Pause { mint } => {
                let spec = b.mint(mint)?.clone();
                let authority = spec
                    .extensions
                    .iter()
                    .find_map(|e| match e {
                        MintExtension::Pausable { authority } => Some(authority.clone()),
                        _ => None,
                    })
                    .context("pause requires a Pausable mint")?;
                let ix = pausable::instruction::pause(
                    &spec.token_program.parse()?,
                    &label_address(&recipe.id, mint),
                    &b.key(&authority)?,
                    &[],
                )?;
                b.send(&format!("post:{index}:pause"), vec![ix])?;
            }
        }
    }

    // Export the resulting accounts, the pinned executables and the Clock.
    let mut accounts = BTreeMap::new();
    for (address, step) in &b.exported {
        let account = b
            .svm
            .get_account(&address.parse()?)
            .with_context(|| format!("fixture account {address} vanished"))?;
        accounts.insert(
            address.clone(),
            WorldAccount {
                account: snapshot(&account),
                origin: WorldOrigin::SyntheticFixture {
                    recipe_sha256: recipe_sha256.clone(),
                    step: step.clone(),
                },
            },
        );
    }
    for (address, account, origin) in programs.accounts {
        let origin = match origin {
            WorldOrigin::SyntheticFixture { step, .. } => WorldOrigin::SyntheticFixture {
                recipe_sha256: recipe_sha256.clone(),
                step,
            },
            other => other,
        };
        accounts.insert(address, WorldAccount { account, origin });
    }
    accounts.insert(
        CLOCK.into(),
        WorldAccount {
            account: AccountSnapshot {
                lamports: 1_169_280,
                owner: SYSVAR_OWNER.into(),
                data: clock.bytes(),
                executable: false,
                rent_epoch: 0,
            },
            origin: WorldOrigin::SyntheticFixture {
                recipe_sha256: recipe_sha256.clone(),
                step: "clock".into(),
            },
        },
    );
    let source = b.mint(&recipe.population_mint)?.clone();
    let source_mint = label_address(&recipe.id, &source.label).to_string();
    let mut population: Vec<String> = accounts
        .iter()
        .filter(|(_, entry)| {
            entry.account.owner == source.token_program
                && decode::decode_token_account(
                    &super::world::rpc_value(&entry.account),
                    &source.token_program,
                    &source_mint,
                    source.decimals,
                )
                .is_ok()
        })
        .map(|(address, _)| address.clone())
        .collect();
    population.sort();
    let world = World {
        kind: WorldKind::SyntheticFixture,
        cluster: SYNTHETIC_CLUSTER.into(),
        genesis_hash: format!("synthetic:{recipe_sha256}"),
        clock,
        observed_slots: None,
        accounts,
        inspected_absent: vec![],
        population: PopulationIndex {
            source_mint,
            token_accounts: population,
            enumeration_completeness: "CompleteSyntheticFixture".into(),
            authority_resolution_completeness: "Complete".into(),
            undecoded_accounts: vec![],
        },
        limitations: vec![
            format!("Synthetic fixture {} ({}): every account was built locally by executing real token-program instructions; none of it is chain state.", recipe.id, BUILDER_VERSION),
            match recipe.programs {
                ProgramSource::PinnedMainnetCapture => "Token, Token-2022 and ATA executables are exact mainnet bytes from a pinned finalized capture.".into(),
                ProgramSource::LiteSvmBundled => "Token, Token-2022 and ATA executables are the official releases bundled with LiteSVM 0.16, not the currently deployed mainnet bytes.".into(),
            },
        ],
        derived_from: None,
    };
    world.validate()?;
    Ok(world)
}

fn account_default_frozen(mint: &MintSpec) -> bool {
    mint.extensions.iter().any(|e| {
        matches!(
            e,
            MintExtension::DefaultAccountState {
                state: DefaultState::Frozen
            }
        )
    })
}

/// Resolve a recipe label to its address, for tests and reports.
pub fn address_of(recipe: &Recipe, label: &str) -> String {
    if recipe.program_owned.iter().any(|p| p.label == label) {
        return program_owned_address(&recipe.id, label).to_string();
    }
    if let Some(account) = recipe.token_accounts.iter().find(|t| t.label == label) {
        if account.layout == AccountLayout::Associated {
            if let (Some(mint), Ok(owner)) = (
                recipe.mints.iter().find(|m| m.label == account.mint),
                owner_address(recipe, &account.owner),
            ) {
                if let Ok(ata) = super::world::associated_token_address(
                    &owner,
                    &mint.token_program,
                    &label_address(&recipe.id, &mint.label).to_string(),
                ) {
                    return ata;
                }
            }
        }
    }
    label_address(&recipe.id, label).to_string()
}

fn owner_address(recipe: &Recipe, owner: &KeyRef) -> Result<String> {
    match owner {
        KeyRef::Label { label } => Ok(address_of(recipe, label)),
        KeyRef::Address { address } => Ok(address.clone()),
        KeyRef::MigrationAuthority { .. } => bail!("migration authority is package-derived"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn recipe(value: Value) -> Recipe {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn builds_legacy_and_token_2022_state_with_real_programs() {
        let recipe = recipe(json!({
            "schemaVersion": 1, "id": "unit-mixed", "description": "unit test",
            "programs": "pinnedMainnetCapture",
            "clock": {"slot": "500", "unixTimestamp": "1760000000", "epoch": "300"},
            "populationMint": "source",
            "wallets": [{"label": "alice", "lamports": "1000000000"}, {"label": "issuer", "lamports": "1000000000"}, {"label": "bob", "lamports": "1000000000"}],
            "programOwned": [{"label": "pool"}],
            "multisigs": [{"label": "treasury", "tokenProgram": LEGACY_PROGRAM, "threshold": 2, "signers": ["alice", "bob", "issuer"]}],
            "mints": [
                {"label": "source", "tokenProgram": LEGACY_PROGRAM, "decimals": 6, "mintAuthority": {"label": "issuer"}, "freezeAuthority": {"label": "issuer"}},
                {"label": "destination", "tokenProgram": TOKEN_2022_PROGRAM, "decimals": 9, "mintAuthority": {"label": "issuer"}, "freezeAuthority": {"label": "issuer"},
                 "extensions": [{"kind": "transferFee", "bps": 50, "maximumFee": "1000000"}, {"kind": "permanentDelegate", "delegate": {"label": "issuer"}}]}
            ],
            "tokenAccounts": [
                {"label": "alice-source", "mint": "source", "owner": {"label": "alice"}, "layout": "associated", "amount": "1000", "delegate": {"delegate": {"label": "bob"}, "amount": "400"}},
                {"label": "treasury-source", "mint": "source", "owner": {"label": "treasury"}, "layout": "associated", "amount": "50"},
                {"label": "pool-source", "mint": "source", "owner": {"label": "pool"}, "layout": "associated", "amount": "70"},
                {"label": "frozen-source", "mint": "source", "owner": {"label": "bob"}, "layout": "explicit", "amount": "5", "frozen": true},
                {"label": "alice-destination", "mint": "destination", "owner": {"label": "alice"}, "layout": "associated", "amount": "0", "extensions": ["memoTransfer", "cpiGuard"]}
            ]
        }));
        let world = build(&recipe, &FixtureContext::default()).unwrap();
        assert_eq!(world.kind, WorldKind::SyntheticFixture);
        assert_eq!(world.population.token_accounts.len(), 4);
        let source_mint = address_of(&recipe, "source");
        let alice = address_of(&recipe, "alice-source");
        let state = world
            .token_account(&alice, LEGACY_PROGRAM, &source_mint, 6)
            .unwrap()
            .unwrap();
        assert_eq!(state.raw_balance, "1000");
        assert_eq!(state.delegated_amount, "400");
        let frozen = world
            .token_account(
                &address_of(&recipe, "frozen-source"),
                LEGACY_PROGRAM,
                &source_mint,
                6,
            )
            .unwrap()
            .unwrap();
        assert!(frozen.is_frozen);
        let destination_mint = world.mint(&address_of(&recipe, "destination")).unwrap();
        let names: Vec<_> = destination_mint
            .extensions
            .iter()
            .map(|e| e.extension_type.as_str())
            .collect();
        assert_eq!(names, ["TransferFeeConfig", "PermanentDelegate"]);
        let destination = world
            .token_account(
                &address_of(&recipe, "alice-destination"),
                TOKEN_2022_PROGRAM,
                &address_of(&recipe, "destination"),
                9,
            )
            .unwrap()
            .unwrap();
        let names: Vec<_> = destination
            .extensions
            .iter()
            .map(|e| e.extension_type.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "TransferFeeAmount",
                "ImmutableOwner",
                "MemoTransfer",
                "CpiGuard"
            ]
        );
        assert!(world.program(TOKEN_2022_PROGRAM).is_ok());
        // Deterministic: the same recipe builds byte-identical worlds.
        let again = build(&recipe, &FixtureContext::default()).unwrap();
        assert_eq!(world.sha256().unwrap(), again.sha256().unwrap());
        assert!(world.accounts.values().all(|a| !a.origin.is_observed()));
    }
}
