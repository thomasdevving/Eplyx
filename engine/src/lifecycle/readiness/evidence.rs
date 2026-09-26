//! Frozen measurement reader. Integrity is relative to the explicitly trusted policy's
//! digest manifest, not signed chain inclusion or a new VM attestation.
use super::*;
use crate::{
    lifecycle::frozen::coverage::{CaseStatus, CoverageClassification, CoverageReport},
    lifecycle::frozen::selection::{
        pipeline::{
            CaptureManifest, ExecutionEvidence, ExecutionIndex, LifecycleCoverageDeltaReport,
        },
        ExpansionPlan,
    },
    lifecycle::resolution::LifecycleResolution,
    lifecycle::{exposure::sha256, policy::LifecycleScenario, LifecycleSnapshot},
    path::position::WithdrawalReport,
};
use serde::de::DeserializeOwned;
use std::path::Path;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessEvidenceManifest {
    pub schema_version: u32,
    pub snapshot: ArtifactRef,
    pub scenario: ArtifactRef,
    pub direct_resolution: ArtifactRef,
    pub position_resolution: ArtifactRef,
    pub coverage: ArtifactRef,
    pub baseline_coverage: ArtifactRef,
    pub expansion_plan: ArtifactRef,
    pub execution_index: ArtifactRef,
    pub capture_manifest: ArtifactRef,
    pub withdrawal_fixture: ArtifactRef,
    pub withdrawal_discovery: ArtifactRef,
    pub official_research: ArtifactRef,
}
fn read<T: DeserializeOwned>(r: &ArtifactRef, base: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&r.read(base)?)?)
}
fn pinned(path: &Path, r: &ArtifactRef) -> Result<()> {
    ensure!(
        sha256(&crate::lifecycle::artifact::read(path)?) == r.sha256,
        "named readiness input digest mismatch: {}",
        path.display()
    );
    Ok(())
}
fn reference(id: &str, r: &ArtifactRef, description: &str) -> EvidenceReference {
    EvidenceReference {
        id: id.into(),
        artifact: r.clone(),
        description: description.into(),
    }
}
fn number(v: &str) -> Result<u128> {
    let n = v.parse::<u128>()?;
    ensure!(n.to_string() == v, "noncanonical evidence amount");
    Ok(n)
}
impl ReadinessEvidenceManifest {
    /// All relative references are resolved against this manifest, including nested
    /// captures/index references normalized back to that same directory in the report.
    pub fn verify(
        &self,
        base: &Path,
        snapshot_path: &Path,
        scenario_path: &Path,
        direct_path: &Path,
        position_path: &Path,
        coverage_path: &Path,
    ) -> Result<VerifiedReadinessEvidence> {
        ensure!(
            self.schema_version == 1,
            "unknown readiness manifest schema"
        );
        for (path, r) in [
            (snapshot_path, &self.snapshot),
            (scenario_path, &self.scenario),
            (direct_path, &self.direct_resolution),
            (position_path, &self.position_resolution),
            (coverage_path, &self.coverage),
        ] {
            pinned(path, r)?;
        }
        let inputs = [
            ("snapshot", &self.snapshot),
            ("scenario", &self.scenario),
            ("direct-resolution", &self.direct_resolution),
            ("position-resolution", &self.position_resolution),
            ("coverage", &self.coverage),
            ("baseline-coverage", &self.baseline_coverage),
            ("expansion-plan", &self.expansion_plan),
            ("execution-index", &self.execution_index),
            ("capture-manifest", &self.capture_manifest),
            ("withdrawal-fixture", &self.withdrawal_fixture),
            ("withdrawal-discovery", &self.withdrawal_discovery),
            ("official-research", &self.official_research),
        ];
        let mut refs = vec![];
        for (id, r) in inputs {
            r.read(base)?;
            refs.push(reference(id,r,"Pinned existing frozen artifact; references are relative to the readiness evidence manifest."));
        }
        let s = LifecycleSnapshot::load(snapshot_path)?;
        let scenario = LifecycleScenario::load(scenario_path)?;
        let direct: LifecycleResolution = read(&self.direct_resolution, base)?;
        let withdrawal: WithdrawalReport = read(&self.position_resolution, base)?;
        let delta: LifecycleCoverageDeltaReport = read(&self.coverage, base)?;
        let baseline: CoverageReport = read(&self.baseline_coverage, base)?;
        let plan: ExpansionPlan = read(&self.expansion_plan, base)?;
        let index: ExecutionIndex = read(&self.execution_index, base)?;
        let captures: CaptureManifest = read(&self.capture_manifest, base)?;
        let official: serde_json::Value = read(&self.official_research, base)?;
        ensure!(
            baseline.snapshot_sha256 == self.snapshot.sha256
                && baseline.scenario_sha256 == self.scenario.sha256
                && baseline.asset_mint == scenario.policy.asset_mint,
            "population scenario/snapshot mismatch"
        );
        ensure!(
            direct.snapshot_sha256 == self.snapshot.sha256
                && direct.scenario_sha256 == self.scenario.sha256
                && direct.asset_mint == baseline.asset_mint
                && direct.coverage_sha256 == self.coverage.sha256,
            "direct evidence context mismatch"
        );
        ensure!(
            delta.baseline_sha256 == self.baseline_coverage.sha256
                && delta.plan_sha256 == self.expansion_plan.sha256
                && delta.execution_index_sha256 == self.execution_index.sha256
                && delta.capture_manifest_sha256 == self.capture_manifest.sha256
                && delta.evidence == index.results,
            "coverage/index linkage mismatch"
        );
        ensure!(
            plan.snapshot_sha256 == self.snapshot.sha256
                && plan.coverage_sha256 == self.baseline_coverage.sha256
                && index.plan_sha256 == self.expansion_plan.sha256
                && captures.plan_sha256 == self.expansion_plan.sha256
                && index.capture_manifest_sha256 == self.capture_manifest.sha256,
            "plan/capture linkage mismatch"
        );
        let source = s
            .entities
            .iter()
            .find(|e| e.id == direct.entity_id)
            .ok_or_else(|| anyhow::anyhow!("direct entity absent from population"))?;
        ensure!(
            source.state.raw_balance == direct.observed_public_balance_raw
                && source.state.owner == direct.owner_authority,
            "direct population binding mismatch"
        );
        ensure!(
            official["entity_id"] == direct.entity_id
                && official["assessment"]["status"] == "NotTested"
                && official["assessment"]["execution_attempted"] == false
                && official["updated_resolution"] == serde_json::to_value(&direct)?,
            "official investigation/direct matrix mismatch"
        );
        let mut fixtures = BTreeMap::new();
        for b in &captures.bindings {
            if let Some(hash) = &b.fixture_sha256 {
                let parent = Path::new(&self.capture_manifest.file)
                    .parent()
                    .unwrap_or(Path::new("."));
                let r = ArtifactRef {
                    file: crate::lifecycle::artifact::display_path(&parent.join(&b.fixture_file)),
                    sha256: hash.clone(),
                };
                let bytes = r.read(base)?;
                let fixture: crate::path::CapturedExecutionFixture =
                    serde_json::from_slice(&bytes)?;
                ensure!(
                    fixtures
                        .insert(
                            b.group_id.clone(),
                            (hash.clone(), fixture, b.capture_context)
                        )
                        .is_none(),
                    "duplicate capture group"
                );
                refs.push(reference(
                    &format!("fixture:{}", b.group_id),
                    &r,
                    "Existing coherent captured route state and deployed program inputs.",
                ));
            }
        }
        let mut facts = vec![];
        let mut measurements = BTreeMap::new();
        let mut counts = BTreeMap::new();
        for e in &index.results {
            let parent = Path::new(&self.execution_index.file)
                .parent()
                .unwrap_or(Path::new("."));
            let r = ArtifactRef {
                file: crate::lifecycle::artifact::display_path(&parent.join(&e.result_file)),
                sha256: e.result_sha256.clone(),
            };
            let m: ExecutionEvidence = read(&r, base)?;
            ensure!(
                m.case_id == e.case_id
                    && m.group_id == e.group_id
                    && m.status == e.status
                    && m.fixture_sha256 == e.fixture_sha256
                    && m.plan_sha256 == self.expansion_plan.sha256,
                "measurement index mismatch"
            );
            let group = plan
                .selected
                .iter()
                .find(|g| g.candidate.id == m.group_id)
                .ok_or_else(|| anyhow::anyhow!("measurement group not selected"))?;
            ensure!(
                m.entity_id == group.candidate.entity_id
                    && m.path_type == group.candidate.path_type
                    && m.context_id == group.candidate.context_id
                    && m.authority.pubkey == group.candidate.authority
                    && group
                        .amount_matrix
                        .iter()
                        .any(|a| a.raw == m.input_raw && a.invalid_control == m.invalid_control),
                "measurement selection/amount mismatch"
            );
            if let Some((hash, f, context)) = fixtures.get(&m.group_id) {
                ensure!(
                    m.fixture_sha256.as_ref() == Some(hash) && m.capture_context == *context,
                    "measurement fixture mismatch"
                );
                let route = f
                    .evidence
                    .get(3)
                    .ok_or_else(|| anyhow::anyhow!("missing final execution batch"))?;
                if let Some(clock) = &m.vm_clock {
                    ensure!(
                        route.result["context"]["slot"].as_u64() == Some(clock.slot),
                        "measurement bank mismatch"
                    );
                }
            } else {
                ensure!(
                    m.execution.is_none() && m.status != CaseStatus::Succeeded,
                    "missing fixture cannot grant execution evidence"
                );
            }
            let attempted = m.execution.is_some();
            let reconciled = m
                .deltas
                .as_ref()
                .is_some_and(|d| d.reconciled && d.input_debited_raw == m.input_raw);
            let status = match m.status {
                CaseStatus::Succeeded => {
                    ensure!(
                        !m.invalid_control
                            && m.execution
                                .as_ref()
                                .is_some_and(|x| x.success && x.error.is_none())
                            && reconciled,
                        "success missing actual execution/reconciliation"
                    );
                    PathStatus::Proven
                }
                CaseStatus::Failed => {
                    ensure!(
                        m.execution
                            .as_ref()
                            .is_some_and(|x| !x.success && x.error.is_some())
                            && m.rollback_verified == Some(true),
                        "failure missing actual VM failure/rollback"
                    );
                    PathStatus::Failed
                }
                CaseStatus::Indeterminate => PathStatus::Indeterminate,
                CaseStatus::Unsupported => PathStatus::Unsupported,
                CaseStatus::Untested => PathStatus::NotTested,
            };
            let evidence_id = format!("measurement:{}", m.case_id);
            refs.push(reference(&evidence_id,&r,"Existing actual local transaction/precondition result; no fresh execution occurs in this gate."));
            let mut reason=m.execution.as_ref().and_then(|x|x.error.clone()).or(m.blocker.clone()).unwrap_or_else(||"Existing successful transaction with exact token/state reconciliation; original authority locally assumed to sign.".into());
            if let Some(x) = &m.execution {
                for log in &x.logs {
                    if log.contains("Error Code:") || log.contains("Error Message:") {
                        reason.push(' ');
                        reason.push_str(log);
                    }
                }
            }
            let scope = EvidenceScope {
                asset_mint: baseline.asset_mint.clone(),
                entity_id: m.entity_id.clone(),
                state_shape: StateShape::DirectTokenAccount,
                authority: m.authority.pubkey.clone(),
                exact_amount_raw: Some(m.input_raw.clone()),
                range: None,
                bps_to_remove: None,
                venue: if m.path_type == ExitPathType::SecondaryMarketExit {
                    Some(m.context_id.clone())
                } else {
                    None
                },
                context_id: Some(m.context_id.clone()),
                captured_slot: m.vm_clock.as_ref().map(|c| c.slot),
                clock: m.vm_clock,
                capture_context: Some(m.capture_context),
                captured_state_sha256: m.captured_state_sha256.clone(),
                fixture_sha256: m.fixture_sha256.clone(),
                source_before_raw: m.source_before_raw.clone(),
                scenario_sha256: self.scenario.sha256.clone(),
            };
            facts.push(PathFact {
                scope,
                path_type: m.path_type,
                status,
                execution_attempted: attempted,
                reconciled,
                rollback_verified: m.rollback_verified,
                signer: SignerAssumption {
                    authority: m.authority.pubkey.clone(),
                    signer_possession_known: m.authority.signer_possession_known,
                    signer_assumed_locally: m.authority.signer_assumed_locally,
                    wording: m.authority.wording.clone(),
                },
                evidence_ids: vec![evidence_id],
                reason,
            });
            *counts.entry(format!("{:?}", m.status)).or_insert(0) += 1;
            ensure!(
                measurements.insert(m.case_id.clone(), m).is_none(),
                "duplicate measured case"
            );
        }
        ensure!(
            counts == delta.status_counts
                && measurements.len()
                    == plan
                        .selected
                        .iter()
                        .map(|g| g.amount_matrix.len())
                        .sum::<usize>(),
            "case completeness/count mismatch"
        );
        for row in &direct.paths {
            for context in &row.contexts {
                for a in &context.attempts {
                    let m = measurements
                        .get(&a.case_id)
                        .ok_or_else(|| anyhow::anyhow!("direct attempt missing measured case"))?;
                    let fact = facts
                        .iter()
                        .find(|f| f.evidence_ids == [format!("measurement:{}", a.case_id)])
                        .unwrap();
                    ensure!(
                        a.evidence.sha256
                            == index
                                .results
                                .iter()
                                .find(|e| e.case_id == a.case_id)
                                .unwrap()
                                .result_sha256
                            && a.entity_id == m.entity_id
                            && a.path_type == m.path_type
                            && a.status == fact.status
                            && a.exact_input_raw == m.input_raw
                            && a.context_id == m.context_id
                            && a.clock == m.vm_clock
                            && a.fixture_sha256 == m.fixture_sha256
                            && a.signer.authority == m.authority.pubkey,
                        "direct exact attempt mismatch"
                    );
                }
            }
            if row.contexts.iter().all(|c| c.attempts.is_empty()) {
                facts.push(PathFact {
                    scope: EvidenceScope {
                        asset_mint: baseline.asset_mint.clone(),
                        entity_id: direct.entity_id.clone(),
                        state_shape: StateShape::DirectTokenAccount,
                        authority: direct.owner_authority.clone(),
                        exact_amount_raw: None,
                        range: None,
                        bps_to_remove: None,
                        venue: None,
                        context_id: None,
                        captured_slot: None,
                        clock: None,
                        capture_context: None,
                        captured_state_sha256: None,
                        fixture_sha256: None,
                        source_before_raw: None,
                        scenario_sha256: self.scenario.sha256.clone(),
                    },
                    path_type: row.path_type,
                    status: row.status,
                    execution_attempted: false,
                    reconciled: false,
                    rollback_verified: None,
                    signer: SignerAssumption {
                        authority: direct.owner_authority.clone(),
                        signer_possession_known: false,
                        signer_assumed_locally: false,
                        wording: "No execution proof for this path.".into(),
                    },
                    evidence_ids: vec!["direct-resolution".into(), "official-research".into()],
                    reason: row.reason.clone(),
                });
            }
        }
        let p = &withdrawal.position;
        ensure!(
            p.snapshot_sha256 == self.snapshot.sha256
                && p.scenario_sha256 == self.scenario.sha256
                && p.fixture_sha256 == self.withdrawal_fixture.sha256
                && p.discovery_sha256
                    == digest(&read::<serde_json::Value>(
                        &self.withdrawal_discovery,
                        base
                    )?)?
                && p.assets.contains(&baseline.asset_mint),
            "position context mismatch"
        );
        ensure!(
            withdrawal.scope.position_id == p.position_id
                && withdrawal.scope.pool == p.pool
                && withdrawal.scope.authority == p.authority
                && withdrawal.scope.fixture_sha256 == p.fixture_sha256
                && withdrawal.scope.lower_bin_id == withdrawal.probe.lower_bin_id
                && withdrawal.scope.upper_bin_id == withdrawal.probe.upper_bin_id
                && withdrawal.scope.bps_to_remove == withdrawal.probe.bps_to_remove,
            "position withdrawal scope mismatch"
        );
        let success = withdrawal
            .execution
            .as_ref()
            .is_some_and(|x| x.success && x.error.is_none());
        ensure!(
            withdrawal.status == PathStatus::Proven
                && withdrawal.execution_attempted
                && success
                && withdrawal.all_position_liquidity_removed == Some(true)
                && withdrawal.position_retained == Some(true)
                && withdrawal.scope.bps_to_remove == 10000,
            "principal withdrawal proof incomplete"
        );
        for t in &withdrawal.token_reconciliation {
            ensure!(
                number(&t.user_credit_raw)?.checked_add(number(&t.withheld_credit_raw)?)
                    == Some(number(&t.reserve_debit_raw)?)
                    && number(&t.reserve_before_raw)?.checked_sub(number(&t.reserve_after_raw)?)
                        == Some(number(&t.reserve_debit_raw)?)
                    && number(&t.destination_after_raw)?
                        .checked_sub(number(&t.destination_before_raw)?)
                        == Some(number(&t.user_credit_raw)?)
                    && t.reserve_debit_raw == t.protocol_calculated_principal_raw
                    && t.reserve_debit_raw == t.bin_principal_decrease_raw,
                "withdrawal token conservation mismatch"
            );
        }
        for b in &withdrawal.bin_reconciliation {
            ensure!(
                b.shares_after == "0"
                    && number(&b.shares_before)?.checked_sub(number(&b.shares_after)?)
                        == Some(number(&b.removed_shares)?)
                    && number(&b.supply_before)?.checked_sub(number(&b.supply_after)?)
                        == Some(number(&b.removed_shares)?),
                "withdrawal share conservation mismatch"
            );
        }
        let post = withdrawal
            .post_position_fields
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("post position missing"))?;
        ensure!(
            post["owner"] == p.authority
                && post["pool"] == p.pool
                && post["liquidity_shares"]
                    .as_array()
                    .is_some_and(|v| v.iter().all(|n| n == "0"))
                && withdrawal.fees_claimed_raw == Some(["0".into(), "0".into()]),
            "position owner/share/fee ledger mismatch"
        );
        let asset_index = p
            .assets
            .iter()
            .position(|m| m == &baseline.asset_mint)
            .unwrap();
        let lp_scope = EvidenceScope {
            asset_mint: baseline.asset_mint.clone(),
            entity_id: format!("solana-program-position:{}", p.position_id),
            state_shape: StateShape::ProtocolPosition,
            authority: p.authority.clone(),
            exact_amount_raw: Some(p.principal_exposure_raw[asset_index].clone()),
            range: Some([withdrawal.scope.lower_bin_id, withdrawal.scope.upper_bin_id]),
            bps_to_remove: Some(withdrawal.scope.bps_to_remove),
            venue: Some(p.pool.clone()),
            context_id: Some(p.pool.clone()),
            captured_slot: withdrawal.captured_clock.as_ref().map(|c| c.slot),
            clock: withdrawal.captured_clock,
            capture_context: Some(
                crate::lifecycle::frozen::selection::pipeline::CaptureContext::CurrentFinalizedProduction,
            ),
            captured_state_sha256: None,
            fixture_sha256: Some(p.fixture_sha256.clone()),
            source_before_raw: None,
            scenario_sha256: self.scenario.sha256.clone(),
        };
        ensure!(
            lp_scope.captured_slot == Some(p.captured_slot),
            "position bank mismatch"
        );
        for row in &withdrawal.paths {
            let tested = row.path_type == ExitPathType::Withdrawal;
            ensure!(
                tested || row.status != PathStatus::Proven,
                "withdrawal cannot escalate another path"
            );
            let mut scope = lp_scope.clone();
            if !tested {
                scope.exact_amount_raw = None;
                scope.range = None;
                scope.bps_to_remove = None;
                scope.context_id = None;
                scope.venue = None;
                scope.captured_slot = None;
                scope.clock = None;
                scope.capture_context = None;
                scope.captured_state_sha256 = None;
                scope.fixture_sha256 = None;
            }
            facts.push(PathFact {
                scope,
                path_type: row.path_type,
                status: row.status,
                execution_attempted: tested,
                reconciled: tested,
                rollback_verified: withdrawal.rollback_verified,
                signer: p.signer.clone(),
                evidence_ids: vec!["position-resolution".into()],
                reason: row.reason.clone(),
            });
        }
        let residual = p
            .assets
            .iter()
            .enumerate()
            .map(|(i, m)| {
                Ok((
                    m.clone(),
                    post["pending_fees_raw"][i]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("pending fee missing"))?
                        .into(),
                ))
            })
            .collect::<Result<BTreeMap<String, String>>>()?;
        let complete = CompleteExitFact {
            scope: lp_scope,
            principal_unwind: withdrawal.status,
            fee_collection: PathStatus::NotTested,
            position_closure: PathStatus::NotTested,
            residual_fees_raw: residual,
            principal_removed_raw: withdrawal
                .token_reconciliation
                .iter()
                .map(|t| (t.mint.clone(), t.reserve_debit_raw.clone()))
                .collect(),
            owner_received_raw: withdrawal
                .token_reconciliation
                .iter()
                .map(|t| (t.mint.clone(), t.user_credit_raw.clone()))
                .collect(),
            destination_withheld_raw: withdrawal
                .token_reconciliation
                .iter()
                .map(|t| (t.mint.clone(), t.withheld_credit_raw.clone()))
                .collect(),
            evidence_ids: vec!["position-resolution".into()],
        };
        let mut entities = baseline
            .entities
            .iter()
            .map(|e| (e.entity_id.clone(), e.clone()))
            .collect::<BTreeMap<_, _>>();
        ensure!(
            entities.len() == baseline.entities.len(),
            "duplicate population entity"
        );
        for u in &delta.entity_updates {
            ensure!(
                entities.contains_key(&u.entity_id),
                "unknown population update"
            );
            entities.insert(u.entity_id.clone(), u.clone());
        }
        ensure!(
            entities.len() == s.entities.len(),
            "population completeness mismatch"
        );
        let mut represented = 0u128;
        let mut covered = 0u128;
        let mut popfacts = vec![];
        for se in &s.entities {
            let e = entities
                .get(&se.id)
                .ok_or_else(|| anyhow::anyhow!("snapshot entity missing coverage"))?;
            ensure!(
                e.owner_authority == se.state.owner
                    && e.represented_balance_raw == se.state.raw_balance
                    && e.account_type == format!("{:?}", se.entity_type),
                "coverage population identity/balance mismatch"
            );
            represented = represented
                .checked_add(number(&e.represented_balance_raw)?)
                .ok_or_else(|| anyhow::anyhow!("population overflow"))?;
            covered = covered
                .checked_add(number(&e.represented_amount_covered_raw)?)
                .ok_or_else(|| anyhow::anyhow!("coverage overflow"))?;
            let paths = e
                .paths
                .iter()
                .filter(|p| {
                    p.classification == CoverageClassification::Proven
                        && p.represented_amount_covered_raw == e.represented_balance_raw
                        && e.represented_balance_raw != "0"
                })
                .map(|p| p.path_type)
                .collect();
            popfacts.push(PopulationEntityFact {
                entity_id: e.entity_id.clone(),
                account_type: e.account_type.clone(),
                balance_raw: e.represented_balance_raw.clone(),
                proven_full_amount_paths: paths,
                proven_candidate_conversion: false,
            });
        }
        let a = &delta.after.portfolio;
        let measured = popfacts
            .iter()
            .filter(|e| !e.proven_full_amount_paths.is_empty())
            .count();
        ensure!(
            a.entities == popfacts.len()
                && a.positive_balance_entities
                    == popfacts.iter().filter(|e| e.balance_raw != "0").count()
                && a.represented_amount_raw == represented.to_string()
                && a.represented_amount_covered_raw == covered.to_string()
                && a.represented_amount_without_evidence_raw
                    == represented
                        .checked_sub(covered)
                        .ok_or_else(|| anyhow::anyhow!("covered amount exceeds population"))?
                        .to_string()
                && delta.after.measured_entities == measured,
            "population aggregate mismatch"
        );
        Ok(VerifiedReadinessEvidence {
            asset_mint: baseline.asset_mint,
            scenario_sha256: self.scenario.sha256.clone(),
            lifecycle_event: scenario.policy,
            policy_evaluated_at: direct
                .policy_evaluated_at
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            path_facts: facts,
            conversion_facts: vec![],
            complete_exits: vec![complete],
            population: PopulationEvidence {
                token_account_entities: a.entities,
                positive_balance_entities: a.positive_balance_entities,
                distinct_owner_authorities: a.distinct_owner_authorities,
                entities_with_measured_amount: measured,
                represented_amount_raw: a.represented_amount_raw.clone(),
                covered_amount_raw: a.represented_amount_covered_raw.clone(),
                without_evidence_raw: a.represented_amount_without_evidence_raw.clone(),
                account_types: delta.after.by_account_type,
                execution_status_counts: counts,
                evidence_ids: vec![
                    "coverage".into(),
                    "baseline-coverage".into(),
                    "snapshot".into(),
                ],
                entities: popfacts,
            },
            evidence_refs: refs,
            isolation_verified: true,
            stress: None,
        })
    }
}
