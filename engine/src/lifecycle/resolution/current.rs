//! Current-run adapter. Only replay-created execution values cross this boundary.
use super::*;
use crate::{
    lifecycle::frozen::selection::pipeline::CaptureContext,
    path::current::VerifiedExecution as CurrentExecution,
};
use anyhow::Context;

pub struct VerifiedCurrentPaths {
    pub(crate) rows: Vec<PathResolution>,
}
impl VerifiedCurrentPaths {
    pub fn rows(&self) -> &[PathResolution] {
        &self.rows
    }
}
pub(crate) struct CurrentEntityScope<'a> {
    pub run: &'a str,
    pub wallet_hash: &'a str,
    pub source: &'a str,
    pub mint: &'a str,
    pub owner: &'a str,
    pub balance: &'a str,
    pub scenario_hash: &'a str,
}
pub(crate) fn resolve(
    scope: &CurrentEntityScope<'_>,
    checks: &[CurrentExecution],
) -> Result<VerifiedCurrentPaths> {
    let CurrentEntityScope {
        run,
        wallet_hash,
        source,
        mint,
        owner,
        balance,
        scenario_hash,
    } = *scope;
    let entity = format!("current:{run}:{source}");
    let discovery = DiscoveryManifest {
        schema_version: 1,
        asset_mint: mint.into(),
        snapshot_sha256: wallet_hash.into(),
        scenario_sha256: scenario_hash.into(),
        sources: vec![DiscoverySource {
            id: "current-wallet".into(),
            kind: EvidenceKind::ObservedOnchainState,
            reference: format!("run:{run}"),
            artifact: ArtifactRef {
                file: format!("{run}.capture.json"),
                sha256: wallet_hash.into(),
            },
            description:
                "Exact current owner-scoped token account observation; no execution inferred".into(),
        }],
        paths: PATHS
            .iter()
            .map(|path| PathDiscovery {
                path_type: *path,
                boundary: if *path == ExitPathType::Redemption {
                    MechanismBoundary::NoSupportedAdapter
                } else {
                    MechanismBoundary::Unresolved
                },
                requested_contexts: vec![],
                evidence_ids: vec!["current-wallet".into()],
                facts: vec![],
                reason: if *path == ExitPathType::Redemption {
                    "No redemption adapter is implemented"
                } else {
                    "No selected current-run execution on this path"
                }
                .into(),
                limitations: vec![
                    "Proposed successor identity supplies no conversion mechanism or proof".into(),
                ],
            })
            .collect(),
        investigation_scope: vec!["Selected current direct token account only".into()],
    };
    let mut executions = Vec::new();
    for checked in checks {
        let v = checked.value();
        ensure!(
            v["run_id"] == run && v["wallet_capture_sha256"] == wallet_hash,
            "current path run binding mismatch"
        );
        ensure!(
            v["source"] == source && v["mint"] == mint && v["owner"] == owner,
            "current path entity binding mismatch"
        );
        let path: ExitPathType = serde_json::from_value(v["path"].clone())?;
        ensure!(
            matches!(
                path,
                ExitPathType::Transfer | ExitPathType::SecondaryMarketExit
            ),
            "unsupported current proof path"
        );
        let text = |key: &str| -> Result<String> {
            Ok(v[key]
                .as_str()
                .context("missing current evidence field")?
                .into())
        };
        let optional = |key: &str| v[key].as_str().map(str::to_string);
        let delta = |key: &str| v["reconciliation"][key].as_str().map(str::to_string);
        let status: PathStatus = serde_json::from_value(v["status"].clone())?;
        let id = text("check_id")?;
        executions.push(VerifiedExecution(PathAttempt {
            case_id: id.clone(),
            entity_id: entity.clone(),
            path_type: path,
            status,
            context_id: format!("current-check:{id}"),
            exact_input_raw: text("amount_raw")?,
            invalid_control: false,
            source_before_raw: Some(balance.into()),
            captured_state_sha256: Some(text("execution_capture_sha256")?),
            fixture_sha256: optional("execution_fixture_sha256"),
            capture_context: CaptureContext::CurrentFinalizedProduction,
            clock: serde_json::from_value(v["clock"].clone())?,
            output_mint: v["market_parameters"]["output_mint"]
                .as_str()
                .map(str::to_string),
            destination: optional("recipient"),
            actual_output_raw: delta("output_received_raw"),
            token_transfer_withheld_raw: v["reconciliation"]["fees"]["token_2022_transfer_fee_raw"]
                .as_str()
                .map(str::to_string),
            dlmm_fee_raw: v["reconciliation"]["fees"]["dlmm_swap_fee_raw"]
                .as_str()
                .map(str::to_string),
            dlmm_protocol_fee_raw: v["reconciliation"]["fees"]["dlmm_protocol_fee_raw"]
                .as_str()
                .map(str::to_string),
            signer: SignerAssumption {
                authority: owner.into(),
                signer_possession_known: false,
                signer_assumed_locally: v["signer_assumed_locally"] == true,
                wording: "Local owner signature assumed; possession unknown".into(),
            },
            execution_attempted: v["execution_performed"] == true,
            error: v["reason"].as_str().map(str::to_string),
            rollback_verified: if status == PathStatus::Failed {
                Some(v["reconciliation"]["reconciled"] == true)
            } else {
                None
            },
            execution_assumptions: vec![
                "Exact current fixture, amount, destination, route and minimum output only".into(),
            ],
            evidence: ArtifactRef {
                file: format!("{id}.capture.json"),
                sha256: text("execution_capture_sha256")?,
            },
        }));
    }
    Ok(VerifiedCurrentPaths {
        rows: LifecyclePathResolver::resolve_entity(&entity, mint, true, &discovery, &executions)?,
    })
}
