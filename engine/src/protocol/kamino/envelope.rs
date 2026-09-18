//! Narrow experimental Scope/KLend envelope policy. Scope execution dependency
//! recognition grants no Scope semantic subjects and never changes normal accept.
use super::*;
use crate::{envelope::*, message::ProvenV0};
use solana_address::Address;
pub const SCOPE_ID: &str = "HFn8GnPADiny6XqUoWE8uRPPxb29ikn4yTuPa9MF2fWJ";
pub const ATA_ID: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
const INSTRUCTIONS_ID: &str = "Sysvar1nstructions1111111111111111111111111";
const SCOPE_REFRESH: [u8; 8] = [83, 186, 207, 131, 203, 254, 198, 130];

fn tag(ix: &InstructionSpec) -> Option<[u8; 8]> {
    ix.data.get(..8)?.try_into().ok()
}
pub fn scope_tokens(ix: &InstructionSpec) -> Result<Vec<u16>> {
    anyhow::ensure!(
        ix.program == SCOPE_ID && tag(ix) == Some(SCOPE_REFRESH),
        "unsupported_execution_dependency: Scope instruction identity"
    );
    let count = u32::from_le_bytes(
        ix.data
            .get(8..12)
            .context("Scope vector length absent")?
            .try_into()?,
    ) as usize;
    anyhow::ensure!(
        (1..=512).contains(&count) && ix.data.len() == 12 + 2 * count,
        "envelope_shape_unsupported: exact Scope vector bytes"
    );
    let tokens = ix.data[12..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|v| u16::from_le_bytes([v[0], v[1]]))
        .collect::<Vec<_>>();
    anyhow::ensure!(
        tokens.iter().all(|t| *t < 512) && ix.accounts.len() == 4 + count,
        "envelope_shape_unsupported: Scope token/account arity"
    );
    anyhow::ensure!(
        ix.accounts[0].is_writable
            && ix.accounts[2].is_writable
            && !ix.accounts[1].is_writable
            && ix.accounts[3].address == INSTRUCTIONS_ID
            && !ix.accounts[3].is_writable
            && ix.accounts.iter().all(|a| !a.is_signer),
        "envelope_shape_unsupported: Scope account privileges/sysvar"
    );
    Ok(tokens)
}
fn ata_existing_shape(
    tx: &HistoricalTransaction,
    index: usize,
    ix: &InstructionSpec,
) -> Result<()> {
    anyhow::ensure!(
        ix.data == [1] && ix.accounts.len() == 6,
        "unsupported_execution_dependency: only ATA CreateIdempotent existing-account shape"
    );
    let a = &ix.accounts;
    anyhow::ensure!(
        a[0].is_signer
            && a[0].is_writable
            && a[1].is_writable
            && a[4].address == SYSTEM_PROGRAM_ID
            && [SPL_TOKEN_PROGRAM_ID, TOKEN_2022_PROGRAM_ID].contains(&a[5].address.as_str()),
        "envelope_shape_unsupported: ATA roles/programs"
    );
    let wallet: Address = a[2].address.parse()?;
    let token: Address = a[5].address.parse()?;
    let mint: Address = a[3].address.parse()?;
    let ata: Address = ATA_ID.parse()?;
    let (expected, _) =
        Address::find_program_address(&[wallet.as_ref(), token.as_ref(), mint.as_ref()], &ata);
    anyhow::ensure!(
        expected.to_string() == a[1].address,
        "envelope_shape_unsupported: ATA derived identity"
    );
    let key = tx
        .account_keys
        .iter()
        .position(|k| k.address == a[1].address)
        .context("ATA message key absent")?;
    anyhow::ensure!(tx.pre_balances.as_ref().and_then(|v|v.get(key)).is_some_and(|b|*b>0) && tx.pre_token_balances.as_ref().is_some_and(|v|v.iter().any(|b|b.account_index==key && b.mint==a[3].address && b.program_id==a[5].address)) && !tx.inner_instruction_frames.iter().any(|f|usize::from(f.outer_index)==index),"unsupported_execution_dependency: ATA creation/unknown lifecycle; existing-token boundary and no-creation CPI evidence required");
    Ok(())
}
fn problem(code: &str, index: Option<usize>, detail: impl Into<String>) -> EnvelopeBlocker {
    EnvelopeBlocker {
        code: code.into(),
        outer_index: index,
        detail: detail.into(),
    }
}

pub fn analyse(message: &ProvenV0) -> EnvelopeAnalysis {
    let tx = message.transaction();
    let mut instructions = Vec::new();
    let mut targets = Vec::new();
    let mut blockers = Vec::new();
    for (index, ix) in tx.instructions.iter().enumerate() {
        let mut row = EnvelopeInstruction {
            outer_index: index,
            instruction: ix.clone(),
            identity: "unknown".into(),
            role: InstructionRole::UnsupportedCompanion,
            semantic_supported: false,
            required_state: ix.accounts.iter().map(|a| a.address.clone()).collect(),
            may_write: ix
                .accounts
                .iter()
                .filter(|a| a.is_writable)
                .map(|a| a.address.clone())
                .collect(),
            consumed_by: vec![],
            historical_binary_required: true,
        };
        let recognised: Result<()> = match ix.program.as_str() {
            PROGRAM_ID => {
                if let Some(op) = KlendOp::from_discriminator(&ix.data) {
                    row.identity = op.name().into();
                    row.role = InstructionRole::SemanticTarget;
                    row.semantic_supported = true;
                    targets.push(TargetObservation {
                        program_id: PROGRAM_ID.into(),
                        outer_index: index,
                        action_id: op.action_id().into(),
                        instruction_identity: op.name().into(),
                    });
                    if ix.data.len() == ACTION_DATA_LEN
                        && ix.accounts.len() == op.account_count()
                        && ix.accounts[0].is_signer
                        && u64::from_le_bytes(ix.data[8..16].try_into().unwrap()) > 0
                    {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!(
                            "target data/arity/signer/amount outside unchanged U2 contract"
                        ))
                    }
                } else {
                    match tag(ix) {
                        Some(REFRESH_RESERVE) if ix.data.len() == 8 && ix.accounts.len() == 6 => {
                            row.identity = "refreshReserve".into();
                            row.role = InstructionRole::TargetPrerequisite;
                            Ok(())
                        }
                        Some(REFRESH_OBLIGATION)
                            if ix.data.len() == 8 && ix.accounts.len() >= 2 =>
                        {
                            row.identity = "refreshObligation".into();
                            row.role = InstructionRole::TargetPrerequisite;
                            Ok(())
                        }
                        _ => Err(anyhow::anyhow!("unsupported KLend prerequisite/action")),
                    }
                }
            }
            SCOPE_ID => scope_tokens(ix).map(|_| {
                row.identity = "refreshPriceList".into();
                row.role = InstructionRole::ExecutionDependency;
            }),
            ATA_ID => ata_existing_shape(tx, index, ix).map(|_| {
                row.identity = "CreateIdempotentExistingAccount".into();
                row.role = InstructionRole::ExecutionDependency;
            }),
            COMPUTE_BUDGET_PROGRAM_ID => {
                row.historical_binary_required = false;
                if ix.accounts.is_empty()
                    && matches!(
                        (ix.data.first(), ix.data.len()),
                        (Some(2), 5) | (Some(3), 9)
                    )
                {
                    row.identity = if ix.data[0] == 2 {
                        "SetComputeUnitLimit"
                    } else {
                        "SetComputeUnitPrice"
                    }
                    .into();
                    row.role = InstructionRole::StandardCompanion;
                    Ok(())
                } else {
                    Err(anyhow::anyhow!("unsupported compute-budget shape"))
                }
            }
            _ => Err(anyhow::anyhow!(
                "unknown external companion; no program wildcard"
            )),
        };
        if let Err(e) = recognised {
            row.role = InstructionRole::UnsupportedCompanion;
            row.semantic_supported = false;
            blockers.push(problem(
                "unsupported_execution_dependency",
                Some(index),
                e.to_string(),
            ));
        }
        instructions.push(row);
    }
    // Failed originals never become eligible even when their LUT proof is exact.
    if !tx.success || tx.error.is_some() {
        blockers.insert(
            0,
            problem(
                "failed_original",
                None,
                "existing successful-original corpus policy",
            ),
        );
    }
    for target in &targets {
        let ix = &tx.instructions[target.outer_index];
        if ix.accounts.len() < 5 {
            continue;
        }
        let reserve = &ix.accounts[4].address;
        let obligation = &ix.accounts[1].address;
        let market = &ix.accounts[2].address;
        let rr = instructions.iter().find(|r| {
            r.identity == "refreshReserve"
                && r.outer_index < target.outer_index
                && r.instruction.accounts[0].address == *reserve
                && r.instruction.accounts[1].address == *market
        });
        let ro = instructions.iter().find(|r| {
            r.identity == "refreshObligation"
                && r.outer_index < target.outer_index
                && r.instruction.accounts[0].address == *market
                && r.instruction.accounts[1].address == *obligation
        });
        if rr.is_none()
            || ro.is_none()
            || rr
                .zip(ro)
                .is_some_and(|(r, o)| r.outer_index >= o.outer_index)
        {
            blockers.push(problem(
                "envelope_shape_unsupported",
                Some(target.outer_index),
                "matching refreshReserve must precede matching refreshObligation and target",
            ));
        }
    }
    for index in 0..instructions.len() {
        if instructions[index].instruction.program != SCOPE_ID
            || instructions[index].role != InstructionRole::ExecutionDependency
        {
            continue;
        }
        let price = instructions[index].instruction.accounts[0].address.clone();
        let consumers = instructions
            .iter()
            .filter(|r| {
                r.identity == "refreshReserve" && r.instruction.accounts[5].address == price
            })
            .map(|r| r.outer_index)
            .collect::<Vec<_>>();
        if consumers.is_empty()
            || consumers.iter().any(|i| *i <= index)
            || tx.instructions[..index]
                .iter()
                .any(|ix| ix.program != COMPUTE_BUDGET_PROGRAM_ID)
        {
            blockers.push(problem("envelope_shape_unsupported",Some(index),"Scope must be top-level after compute-budget-only prefix and before consuming refreshReserve"));
        }
        instructions[index].consumed_by = consumers;
    }
    if targets.is_empty() {
        blockers.push(problem(
            "envelope_shape_unsupported",
            None,
            "no supported semantic target",
        ));
    }
    if targets.len() > 1 {
        let priority = usize::from(!tx.success || tx.error.is_some());
        blockers.insert(priority,problem("unsupported_multi_action_attribution",None,"all target observations preserved; whole-transaction deltas cannot be attributed independently by the existing evaluator"));
    }
    if targets.len() == 1 {
        // This profile covers the four observed primary envelopes. It does not
        // admit arbitrary combinations of individually recognized instructions.
        let names = instructions
            .iter()
            .map(|r| r.identity.as_str())
            .collect::<Vec<_>>();
        let n = names.len();
        let token_count = tx
            .instructions
            .first()
            .and_then(|ix| ix.data.get(8..12))
            .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()));
        let observed_arity = matches!(
            (targets[0].instruction_identity.as_str(), token_count, n),
            (
                "depositReserveLiquidityAndObligationCollateralV2",
                Some(4),
                8
            ) | ("borrowObligationLiquidityV2", Some(8), 10)
                | ("borrowObligationLiquidityV2", Some(2 | 6), 8)
        );
        let target_market = tx.instructions[targets[0].outer_index]
            .accounts
            .get(2)
            .map(|a| a.address.as_str());
        let mut reserves = std::collections::BTreeSet::new();
        let distinct_matching_refreshes = instructions
            .iter()
            .filter(|r| r.identity == "refreshReserve")
            .all(|r| {
                Some(r.instruction.accounts[1].address.as_str()) == target_market
                    && reserves.insert(r.instruction.accounts[0].address.as_str())
            });
        let exact_profile = observed_arity
            && distinct_matching_refreshes
            && names[0] == "refreshPriceList"
            && names[1] == "CreateIdempotentExistingAccount"
            && names[2..n - 4].iter().all(|n| *n == "refreshReserve")
            && names[n - 4] == "refreshObligation"
            && instructions[n - 3].role == InstructionRole::SemanticTarget
            && names[n - 2] == "SetComputeUnitLimit"
            && names[n - 1] == "SetComputeUnitPrice";
        if !exact_profile {
            blockers.push(problem("envelope_shape_unsupported", None, "outside observed Scope/ATA/refresh/target/compute envelope; unexpected extra or reordered dependency"));
        }
    }
    if tx.inner_instructions.len() != tx.inner_instruction_frames.len()
        || tx.pre_token_balances.is_none()
        || tx.post_token_balances.is_none()
    {
        blockers.push(problem(
            "envelope_shape_unsupported",
            None,
            "complete invocation and boundary token metadata required",
        ));
    }
    // A Scope dependency is execution-only. It has no action ID or semantic subjects.
    let structurally_classified = instructions
        .iter()
        .all(|r| r.role != InstructionRole::UnsupportedCompanion);
    EnvelopeAnalysis {
        signature: tx.signature.clone(),
        execution_slot: tx.slot,
        message_proof_id: message.proof().proof_id.clone(),
        instructions,
        targets,
        structurally_classified,
        envelope_admissible: blockers.is_empty(),
        blockers,
        attribution: if tx
            .instructions
            .iter()
            .filter(|ix| ix.program == PROGRAM_ID && recognises(&ix.data))
            .count()
            > 1
        {
            "unsupported_multi_action_attribution"
        } else {
            "single_target_only; semantic evaluation still requires baseline fidelity"
        }
        .into(),
    }
}
pub fn admit(message: &ProvenV0) -> Option<AdmittedEnvelope> {
    analyse(message).seal(message)
}
