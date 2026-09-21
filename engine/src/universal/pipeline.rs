//! Shared resolve -> execute -> fidelity boundary for schema-2 observations.
use std::collections::BTreeMap;

use anyhow::{ensure, Context, Result};

use super::{
    execution::{
        ExecutionBackend, ExecutionEvidence, ExecutionRequest, LiteSvmBackend, SlotHashesVariant,
    },
    fidelity::{compare_v2, FidelityResult},
    model::ReplayObservationV2,
    resolver::ResolvedReplayInput,
};
use crate::{replay::hash_bytes, types::AccountSnapshot};

fn candidate_seeds(
    record: &ReplayObservationV2,
    resolved: &ResolvedReplayInput,
    candidate: &[u8],
) -> Result<BTreeMap<String, AccountSnapshot>> {
    let mut seeds = resolved.seeds.clone();
    let target = record
        .binaries
        .iter()
        .find(|binary| binary.program_id == record.program_id)
        .context("target historical binary missing")?;
    if candidate == resolved.baseline_elf {
        return Ok(seeds);
    }
    let address = target
        .programdata_address
        .as_deref()
        .unwrap_or(&target.program_id);
    let account = seeds
        .get_mut(address)
        .context("target loader account missing")?;
    if target.programdata_address.is_some() {
        ensure!(
            account.data.len() >= 45,
            "target ProgramData header truncated"
        );
        let capacity = account.data.len() - 45;
        ensure!(
            candidate.len() <= capacity,
            "unsupported_runtime_feature: candidate ELF exceeds historical ProgramData allocation"
        );
        account.data[45..45 + candidate.len()].copy_from_slice(candidate);
        account.data[45 + candidate.len()..].fill(0);
    } else {
        ensure!(candidate.len() <= account.data.len(),
            "unsupported_runtime_feature: candidate ELF exceeds historical legacy account allocation");
        account.data[..candidate.len()].copy_from_slice(candidate);
        account.data[candidate.len()..].fill(0);
    }
    Ok(seeds)
}

pub fn execute(
    record: &ReplayObservationV2,
    resolved: &ResolvedReplayInput,
    candidate: &[u8],
) -> Result<ExecutionEvidence> {
    let seeds = candidate_seeds(record, resolved, candidate)?;
    let run = |variant| {
        LiteSvmBackend.execute(&ExecutionRequest {
            message: &resolved.message,
            seeds: &seeds,
            absent_pre_accounts: &resolved.absent_pre_accounts,
            watched: &resolved.watched,
            runtime_sysvars: &resolved.runtime_sysvars,
            clock: None,
            programs_to_load: &[],
            runtime_profile: &resolved.runtime_profile,
            unlimited_logs: true,
            slot_hashes: variant,
            require_complete_state: true,
        })
    };
    let default = run(SlotHashesVariant::BackendDefault)?;
    if record.runtime.slot_hashes_policy == "materiality_checked_default" {
        let empty = run(SlotHashesVariant::Empty)?;
        let different = run(SlotHashesVariant::Different)?;
        ensure!(
            default == empty && default == different,
            "insufficient_runtime_evidence: SlotHashes variant changes execution"
        );
    }
    Ok(default)
}

pub fn baseline(
    record: &ReplayObservationV2,
    resolved: &ResolvedReplayInput,
) -> Result<(ExecutionEvidence, FidelityResult)> {
    let result = execute(record, resolved, &resolved.baseline_elf)?;
    let fidelity = compare_v2(&record.expected, &resolved.expected_accounts, &result);
    ensure!(
        fidelity.matched(),
        "baseline historical fidelity differs: {}",
        fidelity.failures.join(", ")
    );
    Ok((result, fidelity))
}

pub fn evidence_hash(evidence: &ExecutionEvidence) -> Result<String> {
    Ok(hash_bytes(&serde_json::to_vec(evidence)?))
}
