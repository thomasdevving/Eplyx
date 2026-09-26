//! Exact DLMM LbPair and source-vault controller adapter.
use super::{AdapterOutcome, AuthorityAdapter, ControlPath, InvocationKind, ResolutionStatus};
use crate::{
    evidence::authority::EntityType, evidence::paths::PathStatus,
    migration::population_types::StressEntity, protocol::meteora_dlmm,
};
use anyhow::{ensure, Result};
use serde_json::{json, Value};

pub(super) struct MeteoraDlmmAuthorityAdapter;

impl AuthorityAdapter for MeteoraDlmmAuthorityAdapter {
    fn can_resolve(&self, entity: &StressEntity) -> bool {
        entity.authority_model == EntityType::ProgramOwnedAuthority
            && entity.authority_observation.runtime_owner.as_deref()
                == Some(meteora_dlmm::PROGRAM_ID)
    }

    fn resolve(
        &self,
        entity: &StressEntity,
        raw: &Value,
        mint: &str,
        path: &mut ControlPath,
    ) -> Result<AdapterOutcome> {
        let Ok(pool) = meteora_dlmm::decode_pool(&entity.authority, raw, mint) else {
            return Ok(AdapterOutcome {
                resolution: ResolutionStatus::PartiallyResolved,
                conversion: PathStatus::Indeterminate,
                reason: "DLMM runtime owner observed, but exact pool PDA/layout proof failed"
                    .into(),
            });
        };
        let Some(index) = pool
            .vaults
            .iter()
            .position(|v| v.to_string() == entity.token_account)
        else {
            return Ok(AdapterOutcome {
                resolution: ResolutionStatus::PartiallyResolved,
                conversion: PathStatus::Indeterminate,
                reason:
                    "DLMM pool decoded, but this token account is not its verified source vault"
                        .into(),
            });
        };
        ensure!(
            pool.mints[index].to_string() == mint,
            "DLMM vault is not paired with source mint"
        );
        path.controller_program = Some(meteora_dlmm::PROGRAM_ID.into());
        path.controller_state = Some(pool.pool.to_string());
        path.pda_derivation = Some(json!({
            "program": meteora_dlmm::PROGRAM_ID,
            "pool_rule": "find_program_address([base_key, mint_bytes_min, mint_bytes_max], program)",
            "base_key": pool.decoded_fields["base_key"],
            "pool_bump": pool.decoded_fields["pool_bump"],
            "derived_pool": pool.pool.to_string(),
            "source_vault_rule": "find_program_address([pool, source_mint], program)",
            "source_vault_seeds": [pool.pool.to_string(), mint.to_string()],
            "derived_source_vault": entity.token_account,
        }));
        path.invocation_kind = InvocationKind::ProgramInstruction;
        path.required_accounts = vec![pool.pool.to_string(), entity.token_account.clone()];
        path.preconditions = vec![
            "Native liquidity withdrawal requires an independently captured position and owner authority".into(),
        ];
        Ok(AdapterOutcome {
            resolution: ResolutionStatus::ResolvedProtocolInternal,
            conversion: PathStatus::Unsupported,
            reason: "Verified DLMM pool PDA and source vault relationship; no position or owner authorization was captured for sequential withdrawal and conversion".into(),
        })
    }
}
