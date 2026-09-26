//! Typed derived states for migration stress cases and counterexample search.
//!
//! A derivation starts from an exact world and applies declared mutations. Amounts
//! (with the mint supply that must move with them) and the Clock are rebuilt through
//! typed packers; approvals, freezes, destination creation, CPI Guard and required
//! memos are produced by executing the real token-program instruction the relevant
//! authority would sign, in a scratch VM. Every touched account becomes `Derived`
//! with its parent origin, and the world becomes `Derived` with its base recorded:
//! a derived state is never described as observed chain state.
use super::{
    spec::TokenMigrationV1,
    world::{DerivedFrom, World, WorldAccount, WorldClock, WorldKind, WorldOrigin, SYSTEM_PROGRAM},
};
use crate::{
    standard_programs::token as decode,
    standard_programs::token::{ATA_PROGRAM, CLOCK},
    types::AccountSnapshot,
};
use anyhow::{anyhow, bail, ensure, Context, Result};
use litesvm::LiteSVM;
use serde::{Deserialize, Serialize};
use solana_account::Account;
use solana_address::Address;
use solana_instruction::{account_meta::AccountMeta, Instruction};
use solana_message::Message;
use solana_transaction::Transaction;
use spl_token_2022_interface::{
    extension::{cpi_guard, memo_transfer, ExtensionType},
    instruction as token,
};

pub const DERIVATION_VERSION: &str = "eplyx-migration-derivation/v1";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Mutation {
    /// Packer: the token amount and the source mint supply move together.
    #[serde(rename_all = "camelCase")]
    SourceAmount { account: String, to_raw: String },
    /// Packer: the pinned Clock slot.
    ClockSlot { to: String },
    /// Packer: the pinned Clock Unix timestamp.
    ClockUnixTimestamp { to: String },
    /// Real ApproveChecked, signed by the token-account owner (assumed locally).
    #[serde(rename_all = "camelCase")]
    Approve {
        account: String,
        delegate: String,
        amount_raw: String,
    },
    /// Real FreezeAccount, signed by the mint freeze authority (assumed locally).
    Freeze { account: String },
    /// Real ATA CreateIdempotent of the owner's destination account.
    CreateDestination { owner: String },
    /// Real Reallocate + EnableCpiGuard, signed by the owner.
    EnableCpiGuard { account: String },
    /// Real Reallocate + EnableRequiredTransferMemos, signed by the owner.
    RequireMemo { account: String },
}

impl Mutation {
    pub fn label(&self) -> String {
        crate::canonical::document(self)
            .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
            .unwrap_or_default()
    }
}

/// Whether a mutation is reachable at all: freezing needs a freeze authority,
/// CPI Guard and memos need Token-2022. Unreachable states are never fabricated.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reachability {
    Reachable,
    NotReachable { reason: String },
}

fn token_program_of(world: &World, account: &str) -> Result<String> {
    Ok(world
        .snapshot(account)
        .with_context(|| format!("derivation target {account} is not in the world"))?
        .owner
        .clone())
}

fn base_of(world: &World, account: &str) -> Result<(String, String, u8)> {
    let program = token_program_of(world, account)?;
    let data = &world.snapshot(account).context("account")?.data;
    let mint = decode::account_base(&program, data)?.mint;
    let decimals = world.mint(&mint)?.decimals;
    Ok((program, mint, decimals))
}

pub fn reachability(world: &World, mutation: &Mutation) -> Result<Reachability> {
    Ok(match mutation {
        Mutation::Freeze { account } => {
            let (_, mint, _) = base_of(world, account)?;
            if world.mint(&mint)?.freeze_authority.is_none() {
                Reachability::NotReachable {
                    reason: "the mint has no freeze authority, so the account can never be frozen"
                        .into(),
                }
            } else {
                Reachability::Reachable
            }
        }
        Mutation::EnableCpiGuard { account } | Mutation::RequireMemo { account } => {
            if token_program_of(world, account)? != decode::TOKEN_2022_PROGRAM {
                Reachability::NotReachable {
                    reason: "CPI Guard and required memos exist only on Token-2022 accounts".into(),
                }
            } else {
                Reachability::Reachable
            }
        }
        _ => Reachability::Reachable,
    })
}

fn mark(world: &mut World, address: &str, account: AccountSnapshot, mutation: &Mutation) {
    let parent = world
        .accounts
        .get(address)
        .map(|a| Box::new(a.origin.clone()));
    world.accounts.insert(
        address.to_string(),
        WorldAccount {
            account,
            origin: WorldOrigin::Derived {
                parent,
                mutation: mutation.label(),
            },
        },
    );
}

/// A scratch VM holding only `keys` (the message's accounts): a transaction can
/// read or write nothing else, so seeding the rest of the world adds nothing.
fn scratch(world: &World, extra_programs: &[&str], keys: &[Address]) -> Result<LiteSVM> {
    let mut svm = LiteSVM::new()
        .with_sigverify(false)
        .with_blockhash_check(false)
        .with_transaction_history(0);
    let mut ids = vec![
        decode::LEGACY_PROGRAM,
        decode::TOKEN_2022_PROGRAM,
        ATA_PROGRAM,
    ];
    ids.extend_from_slice(extra_programs);
    for id in ids {
        if let Ok(program) = world.program(id) {
            svm.add_program_with_loader(program.program_id, &program.bytes, program.loader)
                .map_err(|e| anyhow!("cannot load {id} for derivation: {e:?}"))?;
        }
    }
    let wanted: std::collections::BTreeSet<String> = keys.iter().map(|k| k.to_string()).collect();
    for (address, entry) in &world.accounts {
        if entry.account.executable || address == CLOCK || !wanted.contains(address) {
            continue;
        }
        svm.set_account(
            address.parse()?,
            Account {
                lamports: entry.account.lamports,
                data: entry.account.data.clone(),
                owner: entry.account.owner.parse()?,
                executable: false,
                rent_epoch: entry.account.rent_epoch,
            },
        )
        .map_err(|e| anyhow!("cannot seed derivation account: {e:?}"))?;
    }
    svm.set_sysvar(&world.clock.clock());
    Ok(svm)
}

fn payer() -> Address {
    super::fixture::label_address("eplyx-migration-derivation", "derivation-payer")
}

fn run(world: &mut World, mutation: &Mutation, instructions: Vec<Instruction>) -> Result<()> {
    let payer = payer();
    let keys: Vec<Address> = Message::new(&instructions, Some(&payer)).account_keys;
    let mut svm = scratch(world, &[], &keys)?;
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
    .map_err(|e| anyhow!("cannot fund derivation payer: {e:?}"))?;
    let message = Message::new(&instructions, Some(&payer));
    let keys: Vec<Address> = message.account_keys.clone();
    if let Err(failure) = svm.send_transaction(Transaction::new_unsigned(message)) {
        bail!(
            "derivation {} failed in the real token program: {:?}",
            mutation.label(),
            failure.err
        );
    }
    for key in keys {
        if key == payer {
            continue;
        }
        let address = key.to_string();
        let Some(account) = svm.get_account(&key) else {
            continue;
        };
        if account.executable {
            continue;
        }
        let snapshot = AccountSnapshot {
            lamports: account.lamports,
            owner: account.owner.to_string(),
            data: account.data.clone(),
            executable: false,
            rent_epoch: account.rent_epoch,
        };
        if world.snapshot(&address) != Some(&snapshot) {
            mark(world, &address, snapshot, mutation);
            world.inspected_absent.retain(|a| a != &address);
        }
    }
    Ok(())
}

/// Apply mutations to a copy of `world`. The result is a `Derived` world.
pub fn derive(world: &World, spec: &TokenMigrationV1, mutations: &[Mutation]) -> Result<World> {
    ensure!(!mutations.is_empty(), "a derivation needs a mutation");
    let base_sha256 = match &world.derived_from {
        Some(from) => from.base_world_sha256.clone(),
        None => world.sha256()?,
    };
    let mut derived = world.clone();
    let mut labels = derived
        .derived_from
        .as_ref()
        .map(|f| f.mutations.clone())
        .unwrap_or_default();
    for mutation in mutations {
        if let Reachability::NotReachable { reason } = reachability(&derived, mutation)? {
            bail!("unreachable derivation {}: {reason}", mutation.label());
        }
        apply(&mut derived, spec, mutation)?;
        labels.push(mutation.label());
    }
    derived.derived_from = Some(DerivedFrom {
        base_kind: world.base_kind(),
        base_world_sha256: base_sha256,
        mutations: labels,
    });
    derived.kind = WorldKind::Derived;
    derived.limitations.push(format!(
        "Derived world ({DERIVATION_VERSION}): {} typed local mutation(s) of the base; derived accounts are not chain state.",
        mutations.len()
    ));
    derived.validate()?;
    Ok(derived)
}

fn apply(world: &mut World, spec: &TokenMigrationV1, mutation: &Mutation) -> Result<()> {
    match mutation {
        Mutation::SourceAmount { account, to_raw } => {
            let to: u64 = crate::migration::spec::canonical_u64(to_raw)?;
            let mut data = world
                .snapshot(account)
                .context("source account")?
                .data
                .clone();
            let base = decode::account_base(&spec.source.token_program, &data)?;
            let (mint, from) = (base.mint, base.amount);
            ensure!(
                mint == spec.source.mint,
                "SourceAmount targets a non-source account"
            );
            decode::replace_amount(&spec.source.token_program, &mut data, to)?;
            let mut mint_data = world.snapshot(&mint).context("source mint")?.data.clone();
            let supply = decode::mint_base(&spec.source.token_program, &mint_data)?
                .supply
                .checked_sub(from)
                .and_then(|s| s.checked_add(to))
                .context("derived source amount overflows the captured mint supply")?;
            decode::replace_supply(&spec.source.token_program, &mut mint_data, supply)?;
            let mut account_snapshot = world.snapshot(account).context("source")?.clone();
            account_snapshot.data = data;
            let mut mint_snapshot = world.snapshot(&mint).context("mint")?.clone();
            mint_snapshot.data = mint_data;
            mark(world, account, account_snapshot, mutation);
            mark(world, &mint, mint_snapshot, mutation);
        }
        Mutation::ClockSlot { to } | Mutation::ClockUnixTimestamp { to } => {
            let value = crate::migration::spec::canonical_u64(to)?;
            let mut clock: WorldClock = world.clock;
            if matches!(mutation, Mutation::ClockSlot { .. }) {
                clock.slot = value;
            } else {
                clock.unix_timestamp = i64::try_from(value)?;
            }
            world.clock = clock;
            let mut snapshot = world.snapshot(CLOCK).context("Clock")?.clone();
            snapshot.data = clock.bytes();
            mark(world, CLOCK, snapshot, mutation);
        }
        Mutation::Approve {
            account,
            delegate,
            amount_raw,
        } => {
            let (program, mint, decimals) = base_of(world, account)?;
            let owner = owner_of(world, account)?;
            let ix = token::approve_checked(
                &program.parse()?,
                &account.parse()?,
                &mint.parse()?,
                &delegate.parse()?,
                &owner.parse()?,
                &[],
                crate::migration::spec::canonical_u64(amount_raw)?,
                decimals,
            )?;
            run(world, mutation, vec![ix])?;
        }
        Mutation::Freeze { account } => {
            let (program, mint, _) = base_of(world, account)?;
            let authority = world
                .mint(&mint)?
                .freeze_authority
                .context("no freeze authority")?;
            let ix = token::freeze_account(
                &program.parse()?,
                &account.parse()?,
                &mint.parse()?,
                &authority.parse()?,
                &[],
            )?;
            run(world, mutation, vec![ix])?;
        }
        Mutation::CreateDestination { owner } => {
            let destination = super::world::associated_token_address(
                owner,
                &spec.destination.token_program,
                &spec.destination.mint,
            )?;
            let ix = Instruction {
                program_id: ATA_PROGRAM.parse()?,
                accounts: vec![
                    AccountMeta::new(payer(), true),
                    AccountMeta::new(destination.parse()?, false),
                    AccountMeta::new_readonly(owner.parse()?, false),
                    AccountMeta::new_readonly(spec.destination.mint.parse()?, false),
                    AccountMeta::new_readonly(SYSTEM_PROGRAM.parse()?, false),
                    AccountMeta::new_readonly(spec.destination.token_program.parse()?, false),
                ],
                data: vec![1],
            };
            run(world, mutation, vec![ix])?;
        }
        Mutation::EnableCpiGuard { account } | Mutation::RequireMemo { account } => {
            let (program, _, _) = base_of(world, account)?;
            let owner: Address = owner_of(world, account)?.parse()?;
            let program: Address = program.parse()?;
            let target: Address = account.parse()?;
            let (extension, enable) = match mutation {
                Mutation::EnableCpiGuard { .. } => (
                    ExtensionType::CpiGuard,
                    cpi_guard::instruction::enable_cpi_guard(&program, &target, &owner, &[])?,
                ),
                _ => (
                    ExtensionType::MemoTransfer,
                    memo_transfer::instruction::enable_required_transfer_memos(
                        &program,
                        &target,
                        &owner,
                        &[],
                    )?,
                ),
            };
            let reallocate =
                token::reallocate(&program, &target, &payer(), &owner, &[], &[extension])?;
            run(world, mutation, vec![reallocate, enable])?;
        }
    }
    Ok(())
}

fn owner_of(world: &World, account: &str) -> Result<String> {
    let snapshot = world.snapshot(account).context("token account")?;
    Ok(decode::account_base(&snapshot.owner, &snapshot.data)?.owner)
}

/// Repack a proposed reserve vault amount for a derived reserve case.
pub fn reserve_amount(account: &AccountSnapshot, to: u64) -> Result<AccountSnapshot> {
    let mut data = account.data.clone();
    decode::replace_amount(&account.owner, &mut data, to)?;
    Ok(AccountSnapshot {
        data,
        ..account.clone()
    })
}
