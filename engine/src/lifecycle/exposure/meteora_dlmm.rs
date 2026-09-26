//! One bounded adapter: Meteora DLMM PermissionlessV2, 904-byte LbPair, state version 1.
//! Layout and PDA rules are pinned to the official SDK revision below.
use anyhow::{ensure, Context, Result};
use serde_json::Value;

use super::{
    AdapterIdentity, AdapterRun, ExposureAdapter, LiquidityAsset, LiquidityExposure,
    ProtocolEvidence, SnapshotVaultLink,
};
use crate::lifecycle::{decode, LifecycleSnapshot};

pub use crate::protocol::meteora_dlmm::{
    decode_pool, DecodedPool, ACCOUNT_LEN, ADAPTER_VERSION, DISCRIMINATOR, PROGRAM_ID, SDK_REVISION,
};
pub struct MeteoraDlmmAdapter;

impl ExposureAdapter for MeteoraDlmmAdapter {
    fn identity(&self) -> AdapterIdentity {
        AdapterIdentity {
            name: "meteora-dlmm".into(),
            version: ADAPTER_VERSION.into(),
            decoder_revision: SDK_REVISION.into(),
            program_id: PROGRAM_ID.into(),
        }
    }
    fn required_accounts(&self, pool: &str, raw: &Value, target_mint: &str) -> Result<Vec<String>> {
        let p = decode_pool(pool, raw, target_mint)?;
        Ok(vec![
            pool.into(),
            p.vaults[0].to_string(),
            p.vaults[1].to_string(),
            p.mints[0].to_string(),
            p.mints[1].to_string(),
            PROGRAM_ID.into(),
        ])
    }
    fn verify(
        &self,
        snapshot: &LifecycleSnapshot,
        run: &AdapterRun,
        run_index: usize,
    ) -> Result<LiquidityExposure> {
        let values = run.evidence[2].result["value"]
            .as_array()
            .context("missing DLMM verification accounts")?;
        ensure!(
            values.len() == 6 && values.iter().all(|v| !v.is_null()),
            "DLMM verification account absent"
        );
        let p = decode_pool(&run.candidate_pool, &values[0], &snapshot.asset.mint)?;
        ensure!(
            values[5]["executable"].as_bool() == Some(true),
            "DLMM program account is not executable"
        );
        ensure!(
            matches!(
                values[5]["owner"].as_str(),
                Some(
                    "BPFLoaderUpgradeab1e11111111111111111111111"
                        | "BPFLoader2111111111111111111111111111111111"
                        | "LoaderV411111111111111111111111111111111111"
                )
            ),
            "DLMM executable has unsupported runtime loader"
        );
        let decoder = format!("meteora-dlmm/{ADAPTER_VERSION}:LbPair@{SDK_REVISION}");
        let pool_evidence = ProtocolEvidence::adapter(run, run_index, 0, &decoder)?;
        let program_evidence =
            ProtocolEvidence::adapter(run, run_index, 5, "Solana runtime executable account")?;
        let mut assets = Vec::new();
        for i in 0..2 {
            let mint_config = decode::decode_mint(&values[3 + i])?;
            ensure!(
                mint_config.token_program == p.token_programs[i],
                "DLMM mint owner disagrees with pool token-program flag"
            );
            let state = decode::decode_token_account(
                &values[1 + i],
                &p.token_programs[i],
                &p.mints[i].to_string(),
                mint_config.decimals,
            )?;
            ensure!(
                state.is_initialized && state.owner == run.candidate_pool,
                "DLMM vault is uninitialized or SPL authority is not the pool PDA"
            );
            let mint_evidence = ProtocolEvidence::adapter(
                run,
                run_index,
                3 + i,
                "SPL mint / Token-2022 extensions 3.1.1",
            )?;
            let vault_evidence = ProtocolEvidence::adapter(
                run,
                run_index,
                1 + i,
                "SPL token account / Token-2022 extensions 3.1.1",
            )?;
            let phase2_link = if p.mints[i].to_string() == snapshot.asset.mint {
                let entity = snapshot.entities.iter().find(|e| e.token_account == p.vaults[i].to_string()).context("verified target vault is absent from Phase 2 snapshot; recapture production state instead of inventing a link")?;
                ensure!(
                    entity.state.owner == run.candidate_pool
                        && entity.state.mint == snapshot.asset.mint,
                    "target vault ownership/mint changed relative to Phase 2 snapshot"
                );
                let historic_pool = snapshot.evidence[entity.authority_evidence.rpc_id]
                    .result
                    .pointer(&entity.authority_evidence.pointer)
                    .context("Phase 2 pool authority evidence missing")?;
                let historic =
                    decode_pool(&run.candidate_pool, historic_pool, &snapshot.asset.mint)
                        .context("Phase 2 authority bytes do not prove the same DLMM role")?;
                ensure!(
                    historic.mints == p.mints
                        && historic.vaults == p.vaults
                        && historic.token_programs == p.token_programs,
                    "DLMM pool relationship differs from Phase 2 evidence"
                );
                ensure!(
                    snapshot.mint_config.token_program == p.token_programs[i],
                    "target mint program changed since Phase 2"
                );
                Some(SnapshotVaultLink {
                    phase2_entity_id: entity.id.clone(),
                    original_classification: entity.entity_type.clone(),
                    phase2_raw_balance: entity.state.raw_balance.clone(),
                    current_raw_balance: state.raw_balance.clone(),
                    phase2_vault_evidence: ProtocolEvidence::lifecycle(
                        snapshot,
                        &entity.token_account,
                        &entity.token_account_evidence,
                        "SPL token account / Token-2022 extensions 3.1.1",
                    )?,
                    phase2_pool_evidence: ProtocolEvidence::lifecycle(
                        snapshot,
                        &run.candidate_pool,
                        &entity.authority_evidence,
                        &decoder,
                    )?,
                })
            } else {
                None
            };
            assets.push(LiquidityAsset {
                mint: p.mints[i].to_string(),
                vault: p.vaults[i].to_string(),
                mint_config,
                state,
                mint_evidence,
                vault_evidence,
                phase2_link,
            });
        }
        Ok(LiquidityExposure { id:format!("meteora-dlmm:{}",run.candidate_pool), protocol:"Meteora".into(),
            product:"DLMM".into(),program_id:PROGRAM_ID.into(),pool_address:run.candidate_pool.clone(),
            authority:run.candidate_pool.clone(),authority_rule:"DLMM pool PDA; each reserve PDA derives from [pool, mint] and its SPL owner equals the pool".into(),
            assets, position_model:None, decoded_pool:p.decoded_fields,
            discovered_at_slot:super::context_slot(&run.evidence[2].result)?, adapter:run.adapter.clone(),
            pool_evidence,program_evidence })
    }
}
