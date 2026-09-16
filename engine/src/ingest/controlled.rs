//! Local-validator demonstration. Empty accounts are created at genesis; every
//! protocol state transition, including position creation, is executed by SBF.
use super::{fetch_accounts, read_json, rpc::RpcProvider, transactions::normalize, write_json};
use crate::{
    replay::{
        hash_bytes, outcome_hash, state_hash, OriginalExecution, PostAccountDigest, ReplayClock,
        ReplayRecord, ReplayStateSource,
    },
    types::{
        AccountMetaSpec, AccountSnapshot, Category, Fixture, InstructionSpec, KeypairSpec,
        NamedAccount,
    },
};
use anyhow::{Context, Result};
use base64::Engine;
use borsh::BorshDeserialize;
use fixture_lending_interface::{
    LendingInstruction as Ix, Position, MARKET_LEN, POSITION_LEN, VAULT_SEED,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use solana_address::Address;
use solana_keypair::Keypair;
use solana_signer::Signer;
use std::{path::Path, thread::sleep, time::Duration};

#[derive(Serialize, Deserialize)]
pub struct Prepared {
    pub fixtures: Vec<Fixture>,
}
/// Secrets are written only to caller-provided temporary storage (mode 0600).
pub fn prepare(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir.join("genesis"))?;
    let program = crate::fixture_program_id();
    let mut fixtures = Vec::new();
    for n in 0..3 {
        let mut keypairs = Vec::new();
        let mut accounts = Vec::new();
        for label in ["payer", "owner", "authority"] {
            let key = Keypair::new();
            let bytes = key.to_bytes();
            keypairs.push(KeypairSpec {
                label: label.into(),
                seed: bytes[..32].to_vec(),
                address: key.pubkey().to_string(),
            });
            accounts.push(NamedAccount {
                label: label.into(),
                address: key.pubkey().to_string(),
                account: AccountSnapshot {
                    lamports: 5_000_000_000_000,
                    owner: "11111111111111111111111111111111".into(),
                    data: vec![],
                    executable: false,
                    rent_epoch: u64::MAX,
                },
            });
        }
        let market = Keypair::new().pubkey();
        let position = Keypair::new().pubkey();
        let vault = Address::find_program_address(&[VAULT_SEED, market.as_ref()], &program).0;
        for (label, address, size) in [
            ("market", market, MARKET_LEN),
            ("position", position, POSITION_LEN),
            ("vault", vault, 0),
        ] {
            accounts.push(NamedAccount {
                label: label.into(),
                address: address.to_string(),
                account: AccountSnapshot {
                    lamports: solana_rent::Rent::default().minimum_balance(size),
                    owner: program.to_string(),
                    data: vec![0; size],
                    executable: false,
                    rent_epoch: u64::MAX,
                },
            });
        }
        for a in &accounts {
            write_json(
                &dir.join("genesis").join(format!("{}.json", a.address)),
                &json!({"pubkey":a.address,"account":{
                "lamports":a.account.lamports,"owner":a.account.owner,"data":[base64::prelude::BASE64_STANDARD.encode(&a.account.data),"base64"],"executable":false,"rentEpoch":a.account.rent_epoch}}),
            )?;
        }
        fixtures.push(Fixture {
            id: format!("controlled-{n:03}"),
            category: Category::Boundary,
            scenario: "controlled chain activity".into(),
            notes: String::new(),
            keypairs,
            accounts,
            fee_payer: "payer".into(),
            signers: vec![],
            instruction: InstructionSpec {
                program: program.to_string(),
                accounts: vec![],
                data: vec![],
            },
            watch: vec![],
        });
    }
    let path = dir.join("private-setup.json");
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
    }
    std::fs::write(path, serde_json::to_vec(&Prepared { fixtures })?)?;
    Ok(())
}
fn instruction(base: &Fixture, ix: Ix, roles: &[(&str, bool, bool)]) -> Result<Fixture> {
    let mut fixture = base.clone();
    fixture.signers.clear();
    fixture.instruction.accounts = roles
        .iter()
        .map(|(label, signer, writable)| {
            if *signer {
                fixture.signers.push((*label).into());
            }
            let address = if *label == "system" {
                "11111111111111111111111111111111".into()
            } else {
                base.account(label)
                    .context("unknown controlled role")?
                    .address
                    .clone()
            };
            Ok(AccountMetaSpec {
                address,
                is_signer: *signer,
                is_writable: *writable,
            })
        })
        .collect::<Result<_>>()?;
    fixture.instruction.data = borsh::to_vec(&ix)?;
    Ok(fixture)
}
fn send(
    rpc: &dyn RpcProvider,
    fixture: &Fixture,
) -> Result<super::transactions::HistoricalTransaction> {
    let latest = rpc.call("getLatestBlockhash", json!([{"commitment":"confirmed"}]))?;
    let hash = latest["value"]["blockhash"]
        .as_str()
        .context("missing blockhash")?
        .parse()?;
    let tx = crate::executor::fixture_transaction(fixture, hash)?;
    let bytes = wincode::serialize(&tx)?;
    let signature=rpc.call("sendTransaction",json!([base64::prelude::BASE64_STANDARD.encode(bytes),{"encoding":"base64","preflightCommitment":"confirmed"}]))?.as_str().context("missing sent signature")?.to_string();
    for _ in 0..120 {
        let raw=rpc.call("getTransaction",json!([signature,{"encoding":"json","commitment":"confirmed","maxSupportedTransactionVersion":0}]))?;
        if !raw.is_null() {
            let tx = normalize(&raw)?;
            anyhow::ensure!(tx.success, "controlled transaction failed: {:?}", tx.error);
            return Ok(tx);
        }
        sleep(Duration::from_millis(250));
    }
    anyhow::bail!("timed out waiting for controlled transaction")
}
/// Only use with the isolated local validator launched by demo-real-replay.sh.
/// No other writer operates these freshly generated accounts between snapshots.
pub fn capture(
    rpc: &dyn RpcProvider,
    dir: &Path,
    snapshots: &Path,
    binary: &Path,
) -> Result<(u64, u64)> {
    let genesis = rpc
        .call("getGenesisHash", json!([]))?
        .as_str()
        .context("genesis hash")?
        .to_string();
    let prepared: Prepared = read_json(&dir.join("private-setup.json"))?;
    let binary_bytes = std::fs::read(binary)?;
    let program_hash = hash_bytes(&binary_bytes);
    let program_id = crate::fixture_program_id().to_string();
    let (_, deployed) = fetch_accounts(rpc, std::slice::from_ref(&program_id))?;
    let deployed = &deployed[0].account;
    anyhow::ensure!(deployed.executable, "controlled program is not executable");
    let code = if deployed.owner == "BPFLoaderUpgradeab1e11111111111111111111111" {
        anyhow::ensure!(
            deployed.data.len() == 36 && deployed.data[..4] == 2u32.to_le_bytes(),
            "invalid upgradeable program account"
        );
        let data_address = Address::new_from_array(deployed.data[4..36].try_into()?).to_string();
        let (_, data) = fetch_accounts(rpc, &[data_address])?;
        anyhow::ensure!(
            data[0].account.owner == deployed.owner
                && data[0].account.data.len() >= 45
                && data[0].account.data[..4] == 3u32.to_le_bytes(),
            "invalid ProgramData account"
        );
        data[0].account.data[45..].to_vec()
    } else {
        anyhow::ensure!(
            deployed.owner == "BPFLoader2111111111111111111111111111111111",
            "unsupported controlled loader"
        );
        deployed.data.clone()
    };
    anyhow::ensure!(
        code == binary_bytes,
        "deployed program bytes differ from --current artifact"
    );
    let mut first = u64::MAX;
    let mut last = 0;
    for (n, base) in prepared.fixtures.iter().enumerate() {
        // Borrow at $110/SOL to respect the 75% LTV guard, then move price to
        // $100/SOL to create the familiar liquidation-boundary position.
        for f in [
            instruction(
                base,
                Ix::InitializeMarket {
                    collateral_price: 110_000_000,
                    liquidation_threshold_bps: 8000,
                    max_ltv_bps: 7500,
                },
                &[("market", false, true), ("authority", true, false)],
            )?,
            instruction(
                base,
                Ix::CreatePosition,
                &[
                    ("position", false, true),
                    ("market", false, true),
                    ("owner", true, false),
                ],
            )?,
            instruction(
                base,
                Ix::DepositCollateral {
                    amount: if n == 0 {
                        100_000_000_000
                    } else {
                        99_500_000_000
                    },
                },
                &[
                    ("position", false, true),
                    ("market", false, false),
                    ("vault", false, true),
                    ("owner", true, true),
                    ("system", false, false),
                ],
            )?,
            instruction(
                base,
                Ix::Borrow {
                    amount: if n == 2 { 7_950_000_000 } else { 7_930_000_000 },
                },
                &[
                    ("position", false, true),
                    ("market", false, false),
                    ("owner", true, false),
                ],
            )?,
            instruction(
                base,
                Ix::SetPrice { price: 100_000_000 },
                &[("market", false, true), ("authority", true, false)],
            )?,
        ] {
            let tx = send(rpc, &f).with_context(|| {
                format!(
                    "controlled setup instruction {}",
                    crate::interpret::instruction_name(&f.instruction.data)
                )
            })?;
            first = first.min(tx.slot);
        }
        let refresh = instruction(
            base,
            Ix::RefreshPosition,
            &[("position", false, true), ("market", false, false)],
        )?;
        // Warm the cached price at $100 before recording the independent
        // refresh. A fresh blockhash makes its signature distinct from warmup.
        let warmup = send(rpc, &refresh)?;
        let mut advanced = false;
        for _ in 0..120 {
            let latest = rpc.call("getLatestBlockhash", json!([{"commitment":"confirmed"}]))?;
            if latest["value"]["blockhash"].as_str() != Some(warmup.recent_blockhash.as_str()) {
                advanced = true;
                break;
            }
            sleep(Duration::from_millis(250));
        }
        anyhow::ensure!(
            advanced,
            "timed out waiting for a distinct capture blockhash"
        );
        let addresses: Vec<_> = ["payer", "position", "market"]
            .iter()
            .map(|label| base.account(label).unwrap().address.clone())
            .collect();
        let (pre_slot, mut pre) = fetch_accounts(rpc, &addresses)?;
        for account in &mut pre {
            account.label = base
                .label_for(&account.address)
                .context("unlabelled capture")?
                .into();
        }
        let transaction = send(rpc, &refresh)?;
        let (post_slot, mut post) = fetch_accounts(rpc, &addresses)?;
        for account in &mut post {
            account.label = base
                .label_for(&account.address)
                .context("unlabelled capture")?
                .into();
        }
        anyhow::ensure!(
            pre_slot <= transaction.slot && post_slot >= transaction.slot,
            "snapshot slots do not bracket transaction"
        );
        let position = post
            .iter()
            .find(|a| a.label == "position")
            .context("missing position")?;
        let decoded = Position::try_from_slice(&position.account.data[..POSITION_LEN])?;
        anyhow::ensure!(
            decoded.last_update_slot == transaction.slot,
            "captured post-state was changed by another slot"
        );
        // This protocol reads Clock.slot only. Other Clock fields are explicit
        // assumptions; exact means complete account/outcome/fee equality under
        // this supported runtime contract, not identical validator internals.
        let epoch = rpc.call("getEpochInfo", json!([{"commitment":"confirmed"}]))?;
        let clock = ReplayClock {
            slot: transaction.slot,
            unix_timestamp: transaction
                .block_time
                .context("controlled block time unavailable")?,
            epoch: epoch["epoch"].as_u64().context("missing epoch")?,
            epoch_start_timestamp: 0,
            leader_schedule_epoch: epoch["epoch"].as_u64().context("epoch")? + 1,
        };
        let record=ReplayRecord {dependencies:Default::default(),acquisitions:Vec::new(),slot_screening:None,schema_version:1,id:base.id.clone(),program_id:crate::fixture_program_id().to_string(),genesis_hash:genesis.clone(),
            current_program_sha256:program_hash.clone(),clock,state_source:ReplayStateSource::ControlledSnapshot,
            pre_state_hash:state_hash(&pre)?,original:Some(OriginalExecution{cpi_invocations:Vec::new(),success:transaction.success,fee:transaction.fee,post_state_hash:outcome_hash(&post)?,post_accounts:post.iter().map(PostAccountDigest::of).collect()}),
            accounts:pre,transaction,assumptions:vec!["isolated local validator; no concurrent account writers".into(),"fixture protocol reads only Clock.slot; remaining sysvars use pinned LiteSVM defaults".into(),"fidelity checks all captured account fields, transaction outcome and fee; CU/log text not expected to match across runtimes".into()]};
        record.validate()?;
        last = last.max(record.transaction.slot);
        write_json(
            &snapshots.join(format!("{}.json", record.transaction.signature)),
            &record,
        )?;
    }
    Ok((first, last))
}
