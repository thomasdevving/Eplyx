//! One deterministic action-input search over one verified parameter proposal.
//! This module never mutates captured state or implements a second evaluator.
use crate::{
    canonical, change::ChangeSpec, parameter_change as parameter, path::ProbeMessage,
    replay::hash_bytes, standard_programs::token,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub mod store;
pub const REVISION: &str = "eplyx-token-2022-amount-search-v1";
pub const SCHEMA: &str = "eplyx-parameter-amount-search-v1";
pub const MAX_EVALUATIONS: u32 = 64;
pub const MAX_VM_CALLS: u32 = 128;

/// New search quantities deliberately accept only canonical decimal strings.
mod raw {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &u64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        use serde::de::Error;
        let text = String::deserialize(d)?;
        let n: u64 = text.parse().map_err(D::Error::custom)?;
        if n.to_string() != text {
            return Err(D::Error::custom("canonical u64 decimal string required"));
        }
        Ok(n)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dimension {
    TransferAmountRaw,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Predicate {
    RecipientLossExceeds {
        #[serde(with = "raw")]
        threshold_raw: u64,
    },
}
impl Predicate {
    pub fn threshold(&self) -> u64 {
        match self {
            Self::RecipientLossExceeds { threshold_raw } => *threshold_raw,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    pub max_evaluations: u32,
    pub max_refinements: u32,
    #[serde(default = "default_vm_budget")]
    pub max_vm_calls: u32,
}
fn default_vm_budget() -> u32 {
    MAX_VM_CALLS
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    pub schema_version: u32,
    pub dimension: Dimension,
    #[serde(with = "raw")]
    pub min_raw: u64,
    #[serde(with = "raw")]
    pub max_raw: u64,
    pub predicate: Predicate,
    pub budget: Budget,
}
impl Spec {
    pub fn validate(&self, balance: u64) -> Result<()> {
        ensure!(self.schema_version == 1, "unsupported search schema");
        ensure!(self.min_raw >= 1 && self.min_raw <= self.max_raw && self.max_raw <= balance,
            "invalid requested interval [{}, {}]; supported bound [1, {}]: positive ordered u64 amounts may not exceed observed source token balance; no clamping",
            self.min_raw, self.max_raw, balance);
        ensure!((1..=MAX_EVALUATIONS).contains(&self.budget.max_evaluations)
            && self.budget.max_refinements <= 16
            && self.budget.max_refinements <= self.budget.max_evaluations
            && (2..=MAX_VM_CALLS).contains(&self.budget.max_vm_calls), "invalid search budgets: evaluations 1..=64, refinements <=16 and <=evaluations, VM calls 2..=128");
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub change: ChangeSpec,
    pub parent_report: Value,
    pub spec: Spec,
    /// Exact original request-bearing capture bytes, when the parent uses one.
    /// Kept local and never printed in the public summary.
    pub source_capture: Option<String>,
}
fn parent(input: &Input) -> Result<parameter::Input> {
    input.change.validate()?;
    ensure!(
        input
            .change
            .as_protocol_parameter_change()
            .is_some_and(|c| c.operation.values().is_some()),
        "parent_blocked: only active newer Token-2022 fee proposals supported"
    );
    parameter::verify(&input.change, &input.parent_report)?;
    ensure!(input.parent_report["execution_performed"] == true
        && ["baseline", "proposed"].iter().all(|s|
            input.parent_report[s]["execution"]["success"] == true
            && input.parent_report[s]["reconciliation"]["reconciled"] == true),
        "parent_blocked: require successful reconciled baseline/proposed contract (economic findings allowed)");
    let p: parameter::Input =
        serde_json::from_value(input.parent_report["retained_input"].clone())?;
    match (&p.source_capture_sha256, &input.source_capture) {
        (Some(hash), Some(bytes)) => {
            ensure!(hash_bytes(bytes.as_bytes()) == *hash, "parent source capture digest mismatch");
            let captured = crate::path::current::parameter_input(bytes.as_bytes())?;
            ensure!(canonical::digest(&captured)? == p.sha256()?, "parent request-bearing capture/input mismatch");
        }
        (None, None) => (),
        _ => anyhow::bail!("parent_blocked: exact request-bearing source capture required only when parent binds one"),
    }
    let plan = p.validate()?;
    let change = input
        .change
        .as_protocol_parameter_change()
        .context("parameter proposal required")?;
    let (expected, rate) = change
        .operation
        .values()
        .context("only active newer Token-2022 fee rate supported")?;
    let mint = plan
        .accounts
        .iter()
        .find(|a| a.address == p.context.mint)
        .context("mint absent")?;
    parameter::mutate(&mint.account, expected, rate, plan.clock.epoch)?;
    let source = plan
        .accounts
        .iter()
        .find(|a| a.address == p.context.source)
        .context("source absent")?;
    let (balance, _) = token::account_amounts(&p.context.program, &source.account.data)?;
    input.spec.validate(balance)?;
    Ok(p)
}
fn fixed(p: &parameter::Input) -> Result<Value> {
    let plan = p.validate()?;
    Ok(
        json!({"context":p.context,"fixture_sha256":p.fixture_sha256,
        "accounts":plan.accounts,"watch":plan.watch,"account_evidence":plan.account_evidence,
        "programs":plan.programs.iter().map(|p|json!({"program_id":p.program_id.to_string(),"loader":p.loader.to_string(),"elf_sha256":hash_bytes(&p.bytes)})).collect::<Vec<_>>(),
        "clock":crate::path::ProbeClock::from(&plan.clock),"assumptions":plan.assumptions,"runtime":parameter::runtime()}),
    )
}
pub fn input_identity(input: &Input) -> Result<Value> {
    let p = parent(input)?;
    Ok(json!({"schema":SCHEMA,"change_spec_id":input.change.id()?,
        "parent_report_sha256":input.parent_report["report_sha256"],"parent_input_sha256":p.sha256()?,
        "parent_source_capture_sha256":p.source_capture_sha256,"fixture_sha256":p.fixture_sha256,
        "fixed_commitment_sha256":canonical::digest(&fixed(&p)?)?,
        "original_amount_raw":p.amount_raw.to_string(),"spec":input.spec,
        "algorithm_revision":REVISION,"algorithm_source_sha256":hash_bytes(include_bytes!("parameter_search.rs")),
        "derivation":"hypothetical TransferChecked amount only; unchanged captured account state",
        "stop_policy":"refine first witnessed signature once, resume seeds; stop at evaluation or VM budget"}))
}
pub fn derive(p: &parameter::Input, amount: u64) -> Result<parameter::Input> {
    ensure!(amount > 0, "zero amount outside search domain");
    let mut d = p.clone();
    d.amount_raw = amount;
    // A request-bearing original source capture never claims a generated request occurred.
    d.source_capture_sha256 = None;
    d.validate()?; // authoritative typed TransferChecked builder and admission
    let mut normalized = d.clone();
    normalized.amount_raw = p.amount_raw;
    normalized.source_capture_sha256 = p.source_capture_sha256.clone();
    ensure!(
        canonical::document(&normalized)? == canonical::document(p)?,
        "unrelated action/evidence differences"
    );
    // Exact normalized input equality includes every observed byte and action
    // field. The deterministic typed builder reconstructs the same message.
    Ok(d)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    #[serde(with = "raw")]
    pub amount_raw: u64,
    pub reasons: Vec<String>,
}
fn add(queue: &mut Vec<Candidate>, spec: &Spec, n: u64, why: &str) {
    if n < spec.min_raw || n > spec.max_raw {
        return;
    }
    if let Some(c) = queue.iter_mut().find(|c| c.amount_raw == n) {
        if !c.reasons.iter().any(|r| r == why) {
            c.reasons.push(why.into());
        }
    } else {
        queue.push(Candidate {
            amount_raw: n,
            reasons: vec![why.into()],
        });
    }
}
fn neighborhood(queue: &mut Vec<Candidate>, spec: &Spec, n: u64, why: &str) {
    for x in [n.checked_sub(1), Some(n), n.checked_add(1)]
        .into_iter()
        .flatten()
    {
        add(queue, spec, x, why);
    }
}
fn fee(rate: u16, cap: u64) -> spl_token_2022_interface::extension::transfer_fee::TransferFee {
    spl_token_2022_interface::extension::transfer_fee::TransferFee {
        epoch: 0.into(),
        maximum_fee: cap.into(),
        transfer_fee_basis_points: rate.into(),
    }
}
/// Monotonic *individual official fee* only: first fee >= target. This is a
/// bounded helper hint, never a search-result oracle or fee-difference inverse.
fn fee_hint(
    fee: &spl_token_2022_interface::extension::transfer_fee::TransferFee,
    target: u64,
) -> Option<u64> {
    if fee.calculate_fee(u64::MAX)? < target {
        return None;
    }
    let (mut lo, mut hi) = (0, u64::MAX);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if fee.calculate_fee(mid)? >= target {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    Some(lo)
}
pub fn candidates(
    spec: &Spec,
    original: u64,
    current: u16,
    proposed: u16,
    cap: u64,
) -> Vec<Candidate> {
    let mut q = vec![];
    add(&mut q, spec, spec.min_raw, "interval_min");
    add(&mut q, spec, spec.max_raw, "interval_max");
    add(&mut q, spec, original, "original_amount_in_domain");
    neighborhood(&mut q, spec, spec.min_raw, "endpoint_neighbors");
    neighborhood(&mut q, spec, spec.max_raw, "endpoint_neighbors");
    neighborhood(&mut q, spec, original, "original_neighbors");
    let delta = i32::from(proposed) - i32::from(current);
    if delta > 0 {
        // Continuous estimate is only a hint; rounding/capping need actual VM execution.
        let n = (u128::from(spec.predicate.threshold()) + 1) * 10000 / delta as u128;
        if let Ok(n) = u64::try_from(n) {
            neighborhood(&mut q, spec, n, "threshold_neighborhood_hint");
        }
    }
    for (name, rate) in [("baseline", current), ("proposed", proposed)] {
        let f = fee(rate, cap);
        let targets = [
            Some(1),
            Some(2),
            Some(cap),
            spec.predicate.threshold().checked_add(1),
            f.calculate_fee(spec.min_raw).and_then(|n| n.checked_add(1)),
            f.calculate_fee(spec.max_raw),
            f.calculate_fee(original),
        ];
        for target in targets.into_iter().flatten() {
            if let Some(n) = fee_hint(&f, target) {
                neighborhood(
                    &mut q,
                    spec,
                    n,
                    &format!("{name}_official_fee_at_least_{target}_hint"),
                );
            }
        }
    }
    for i in 1..16u128 {
        let n = u128::from(spec.min_raw) + u128::from(spec.max_raw - spec.min_raw) * i / 16;
        neighborhood(&mut q, spec, n as u64, "representative_interior");
    }
    if spec.max_raw - spec.min_raw < 32 {
        for n in spec.min_raw..=spec.max_raw {
            add(&mut q, spec, n, "small_domain_enumeration");
        }
    }
    q
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub status: String,
    pub predicate: String,
    pub baseline_credit_raw: Option<String>,
    pub proposed_credit_raw: Option<String>,
    pub loss_raw: Option<String>,
    pub anomaly: Option<String>,
    pub signature: Option<Value>,
}
fn rollback_failed(side: &Value) -> Result<bool> {
    if side["execution"]["success"] != false {
        return Ok(false);
    }
    let pre: Vec<crate::types::NamedAccount> = serde_json::from_value(side["pre_state"].clone())?;
    let x: crate::executor::ProbeTransactionExecution =
        serde_json::from_value(side["execution"].clone())?;
    Ok(pre
        .iter()
        .any(|a| x.post_accounts.get(&a.address) != Some(&a.account)))
}
pub fn outcome(report: &Value, predicate: &Predicate) -> Result<Outcome> {
    let mut out = Outcome {
        status: "admission_unsupported".into(),
        predicate: "unavailable".into(),
        baseline_credit_raw: None,
        proposed_credit_raw: None,
        loss_raw: None,
        anomaly: None,
        signature: None,
    };
    if report["execution_performed"] != true {
        out.status = match report["status"].as_str() {
            Some("execution_unavailable") => "execution_unavailable",
            Some("config_evidence_missing" | "current_state_mismatch") => "evidence_mismatch",
            _ => "admission_unsupported",
        }
        .into();
        return Ok(out);
    }
    let b = &report["baseline"];
    let p = &report["proposed"];
    let bs = b["execution"]["success"]
        .as_bool()
        .context("baseline success absent")?;
    let ps = p["execution"]["success"]
        .as_bool()
        .context("proposed success absent")?;
    let rollback = rollback_failed(b)? || rollback_failed(p)?;
    let reconciled =
        b["reconciliation"]["reconciled"] == true && p["reconciliation"]["reconciled"] == true;
    if rollback || !reconciled || (bs && !ps) {
        let kind = if rollback {
            "rollback_failure"
        } else if !reconciled {
            "reconciliation_failure"
        } else {
            "baseline_succeeds_proposed_rejects"
        };
        out.status = if rollback || !reconciled {
            "reconciliation_rollback_failure"
        } else {
            "supported_execution_rejection"
        }
        .into();
        out.anomaly = Some(kind.into());
        out.signature = Some(
            json!({"kind":kind,"baseline_success":bs,"proposed_success":ps,
            "baseline_error":b["execution"]["error"],"proposed_error":p["execution"]["error"],
            "baseline_reconciliation_error":b["reconciliation_error"],"proposed_reconciliation_error":p["reconciliation_error"],
            "baseline_reconciled":b["reconciliation"]["reconciled"],"proposed_reconciled":p["reconciliation"]["reconciled"],"rollback_failed":rollback}),
        );
        return Ok(out);
    }
    if !bs || !ps {
        out.status = "supported_execution_rejection".into();
        return Ok(out);
    }
    let parse_credit = |side: &Value| -> Result<i128> {
        let text = side["reconciliation"]["output_received_raw"]
            .as_str()
            .context("reconciled credit missing")?;
        let n: u64 = text.parse()?;
        ensure!(text == n.to_string(), "noncanonical reconciled credit");
        Ok(i128::from(n))
    };
    let before = parse_credit(b)?;
    let after = parse_credit(p)?;
    let loss = before.checked_sub(after).context("signed loss overflow")?;
    let matched = loss > i128::from(predicate.threshold());
    out.status = "supported_successful_comparison".into();
    out.predicate = if matched { "matched" } else { "not_matched" }.into();
    out.baseline_credit_raw = Some(before.to_string());
    out.proposed_credit_raw = Some(after.to_string());
    out.loss_raw = Some(loss.to_string());
    if matched {
        out.signature = Some(serde_json::to_value(predicate)?);
    }
    Ok(out)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub candidate: Candidate,
    pub case_id: String,
    pub derivation: Value,
    pub attempted: bool,
    pub phase: Option<String>,
    pub admission: String,
    pub report: Option<Value>,
    pub vm_calls: u32,
    pub outcome: Option<Outcome>,
    pub exclusion: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema: String,
    pub input: Input,
    pub input_identity: Value,
    pub search_input_sha256: String,
    pub ledger: Vec<Case>,
    pub execution_order: Vec<String>,
    pub witnesses: Vec<Value>,
    pub summary: Value,
    pub report_sha256: String,
}
fn case(p: &parameter::Input, c: Candidate, search: &str, fixed_hash: &Value) -> Result<Case> {
    let d = derive(p, c.amount_raw)?;
    let message = ProbeMessage::from(&d.validate()?.message);
    let derivation = json!({"origin":"hypothetical_derived_action","search_input_sha256":search,
        "parent_input_sha256":p.sha256()?,"parent_source_capture_sha256":p.source_capture_sha256,
        "original_amount_raw":p.amount_raw.to_string(),"hypothetical_amount_raw":c.amount_raw.to_string(),
        "revision":REVISION,"derived_input_sha256":d.sha256()?,"message":message,
        "unchanged_fixed_commitment_sha256":fixed_hash});
    Ok(Case {
        case_id: canonical::digest(&derivation)?,
        candidate: c,
        derivation,
        attempted: false,
        phase: None,
        admission: "not_attempted".into(),
        report: None,
        vm_calls: 0,
        outcome: None,
        exclusion: None,
    })
}
fn refinement(spec: &Spec, witness: u64) -> Vec<Candidate> {
    let mut q = vec![];
    if let Some(n) = witness.checked_sub(1) {
        neighborhood(&mut q, spec, n, "refinement_witness_neighbors");
    }
    let width = witness - spec.min_raw;
    for i in [1u128, 2, 4, 8, 12, 14, 15] {
        let n = u128::from(spec.min_raw) + u128::from(width) * i / 16;
        neighborhood(&mut q, spec, n as u64, "refinement_smaller_samples");
    }
    q.retain(|c| c.amount_raw < witness);
    q
}
fn evaluate(
    c: &mut Case,
    p: &parameter::Input,
    input: &Input,
    phase: &str,
    evaluate: &mut impl FnMut(&parameter::Input) -> Result<(Value, u32)>,
) -> Result<()> {
    let d = derive(p, c.candidate.amount_raw)?;
    c.admission = "admitted".into();
    c.attempted = true;
    c.phase = Some(phase.into());
    let (r, calls) = evaluate(&d)?;
    ensure!(
        calls <= 2 && (r["execution_performed"] != true || calls == 2),
        "invalid paired VM count"
    );
    parameter::verify(&input.change, &r)?;
    ensure!(
        r["analysis_input_sha256"] == d.sha256()?
            && r["change"]["change_spec_id"] == input.change.id()?,
        "cross-case paired report mismatch"
    );
    c.outcome = Some(outcome(&r, &input.spec.predicate)?);
    c.report = Some(r);
    c.vm_calls = calls;
    Ok(())
}
fn run(
    input: &Input,
    mut eval: impl FnMut(&parameter::Input) -> Result<(Value, u32)>,
) -> Result<Report> {
    let p = parent(input)?;
    let identity = input_identity(input)?;
    let search_id = canonical::digest(&identity)?;
    let change = input
        .change
        .as_protocol_parameter_change()
        .context("parameter required")?;
    let (expected, rate) = change.operation.values().context("Token-2022 required")?;
    let queue = candidates(
        &input.spec,
        p.amount_raw,
        expected.basis_points,
        rate,
        expected.maximum_fee_raw,
    );
    let mut ledger = queue
        .into_iter()
        .map(|c| case(&p, c, &search_id, &identity["fixed_commitment_sha256"]))
        .collect::<Result<Vec<_>>>()?;
    let seeds = ledger.len();
    let mut order = vec![];
    let mut evaluations = 0;
    let mut calls = 0;
    let mut refinements = 0;
    let mut first: Option<(usize, Value)> = None;
    let mut refined = false;
    let mut stop = "candidate_plan_completed";
    for index in 0..seeds {
        if ledger[index].attempted {
            continue;
        }
        if evaluations >= input.spec.budget.max_evaluations {
            stop = "budget_exhausted";
            break;
        }
        if calls + 2 > input.spec.budget.max_vm_calls {
            stop = "execution_limit_reached";
            break;
        }
        evaluate(&mut ledger[index], &p, input, "generation", &mut eval)?;
        evaluations += 1;
        calls += ledger[index].vm_calls;
        order.push(ledger[index].case_id.clone());
        if first.is_none() {
            if let Some(signature) = ledger[index]
                .outcome
                .as_ref()
                .and_then(|o| o.signature.clone())
            {
                first = Some((index, signature));
            }
        }
        if !refined {
            if let Some((witness, _)) = &first {
                refined = true;
                let refinements_queue =
                    refinement(&input.spec, ledger[*witness].candidate.amount_raw);
                let mut indices = vec![];
                for c in refinements_queue {
                    if let Some(i) = ledger
                        .iter()
                        .position(|l| l.candidate.amount_raw == c.amount_raw)
                    {
                        for reason in c.reasons {
                            if !ledger[i].candidate.reasons.contains(&reason) {
                                ledger[i].candidate.reasons.push(reason);
                            }
                        }
                        indices.push(i);
                    } else {
                        indices.push(ledger.len());
                        ledger.push(case(
                            &p,
                            c,
                            &search_id,
                            &identity["fixed_commitment_sha256"],
                        )?);
                    }
                }
                for i in indices {
                    if ledger[i].attempted {
                        continue;
                    }
                    if refinements >= input.spec.budget.max_refinements {
                        break;
                    }
                    if evaluations >= input.spec.budget.max_evaluations
                        || calls + 2 > input.spec.budget.max_vm_calls
                    {
                        break;
                    }
                    evaluate(&mut ledger[i], &p, input, "refinement", &mut eval)?;
                    evaluations += 1;
                    refinements += 1;
                    calls += ledger[i].vm_calls;
                    order.push(ledger[i].case_id.clone());
                }
            }
        }
    }
    // Pending refinement suggestions are still considered cases, with an explicit exclusion.
    for (i, c) in ledger.iter_mut().enumerate() {
        if !c.attempted {
            c.exclusion = Some(
                if i < seeds {
                    stop
                } else {
                    "refinement_budget_or_total_limit"
                }
                .into(),
            );
        }
    }
    let mut witnesses = vec![];
    for c in ledger
        .iter()
        .filter(|c| c.outcome.as_ref().is_some_and(|o| o.signature.is_some()))
    {
        let signature = c.outcome.as_ref().unwrap().signature.as_ref().unwrap();
        let mut w = json!({"search_input_sha256":search_id,"parent_report_sha256":input.parent_report["report_sha256"],
            "case_id":c.case_id,"derived_input_sha256":c.derivation["derived_input_sha256"],
            "amount_raw":c.candidate.amount_raw.to_string(),"paired_report_sha256":c.report.as_ref().unwrap()["report_sha256"],
            "signature":signature,"outcome":c.outcome,
            "refinement_origin_case_id":first.as_ref().filter(|(_,s)|s==signature).map(|(i,_)|&ledger[*i].case_id),
            "refinement_executed_case_ids":order.iter().filter(|id|ledger.iter().any(|l|&l.case_id==*id && l.phase.as_deref()==Some("refinement"))).collect::<Vec<_>>()});
        w["witness_sha256"] = canonical::digest(&w)?.into();
        witnesses.push(w);
    }
    let cardinality = u128::from(input.spec.max_raw) - u128::from(input.spec.min_raw) + 1;
    let matched = ledger
        .iter()
        .filter(|c| c.outcome.as_ref().is_some_and(|o| o.predicate == "matched"))
        .collect::<Vec<_>>();
    let smallest = matched.iter().map(|c| c.candidate.amount_raw).min();
    let fully_enumerated = u128::from(evaluations) == cardinality;
    let summary = json!({"completion_status":stop,"requested_min_raw":input.spec.min_raw.to_string(),"requested_max_raw":input.spec.max_raw.to_string(),
        "domain_cardinality":cardinality.to_string(),"generated_considered":ledger.len(),"unique_evaluated_amounts":evaluations,"admitted_pairs":evaluations,
        "fully_reconciled_pairs":ledger.iter().filter(|c|c.report.as_ref().is_some_and(|r|r["execution_performed"]==true && r["baseline"]["reconciliation"]["reconciled"]==true && r["proposed"]["reconciliation"]["reconciled"]==true)).count(),
        "rejected_cases":ledger.iter().filter(|c|c.outcome.as_ref().is_some_and(|o|o.status=="supported_execution_rejection")).count(),
        "unavailable_cases":ledger.iter().filter(|c|c.outcome.as_ref().is_some_and(|o|o.predicate=="unavailable")).count(),
        "refinement_evaluations":refinements,"total_vm_calls":calls,"numeric_domain_fully_enumerated":fully_enumerated,
        "matching_cases":matched.len(),"anomaly_cases":ledger.iter().filter(|c|c.outcome.as_ref().is_some_and(|o|o.anomaly.is_some())).count(),
        "first_witness_case_id":first.as_ref().map(|(i,_)|&ledger[*i].case_id),
        "smallest_matching_amount_among_executed_cases_raw":smallest.map(|n|n.to_string()),
        "maximum_loss_among_tested_successful_cases_raw":ledger.iter().filter_map(|c|c.outcome.as_ref()?.loss_raw.as_ref()?.parse::<i128>().ok()).max().map(|n|n.to_string()),
        "untested_amount_count":(cardinality-u128::from(evaluations)).to_string(),
        "conclusion":if matched.is_empty(){format!("No matching case found among {evaluations} executed cases within the stated domain and budget.")}else{format!("Declared condition matched in {} executed cases; fee impact is not a protocol bug verdict.",matched.len())},
        "minimality_claim":"smallest matching amount among the executed cases; no global minimum certified"});
    let mut report = Report {
        schema: SCHEMA.into(),
        input: input.clone(),
        input_identity: identity,
        search_input_sha256: search_id,
        ledger,
        execution_order: order,
        witnesses,
        summary,
        report_sha256: String::new(),
    };
    report.report_sha256 = report_digest(&report)?;
    Ok(report)
}
fn report_digest(report: &Report) -> Result<String> {
    let mut r = serde_json::to_value(report)?;
    r.as_object_mut().unwrap().remove("report_sha256");
    canonical::digest(&r)
}
pub fn search(input: &Input) -> Result<Report> {
    run(input, |d| {
        let mut calls = 0;
        let report = parameter::analyze_with_vm_counter(&input.change, d, &mut calls)?;
        Ok((report, calls))
    })
}
/// Retained reports drive the identical deterministic scheduler. No execution,
/// network, artifact write or repair occurs in this path.
pub fn verify(report: &Report) -> Result<()> {
    ensure!(
        report.schema == SCHEMA && report.report_sha256 == report_digest(report)?,
        "search report digest/schema mismatch"
    );
    ensure!(
        report.ledger.len() <= 512 && report.execution_order.len() <= MAX_EVALUATIONS as usize,
        "search ledger exceeds hard bound"
    );
    let mut position = 0;
    let rebuilt = run(&report.input, |d| {
        let id = report
            .execution_order
            .get(position)
            .context("omitted execution")?;
        let c = report
            .ledger
            .iter()
            .find(|c| &c.case_id == id)
            .context("missing executed case")?;
        ensure!(
            c.attempted && c.candidate.amount_raw == d.amount_raw,
            "reordered or mismatched execution"
        );
        position += 1;
        let r = c.report.clone().context("attempted report missing")?;
        if r["execution_performed"] != true {
            // Only unavailable calls may lack a pair. Exact reproduction checks
            // the counter; read-only verification checks its bounded contract.
            ensure!(
                r["status"] == "execution_unavailable" && (1..=2).contains(&c.vm_calls),
                "unavailable VM-count contract invalid"
            );
        }
        Ok((r, c.vm_calls))
    })?;
    ensure!(
        position == report.execution_order.len()
            && canonical::document(&rebuilt)? == canonical::document(report)?,
        "search ledger/predicate/summary/derivation or ordering mismatch"
    );
    Ok(())
}
pub fn reproduce(report: &Report) -> Result<()> {
    verify(report)?;
    let rebuilt = search(&report.input)?;
    ensure!(
        canonical::document(&rebuilt)? == canonical::document(report)?,
        "offline completed search differs"
    );
    Ok(())
}
pub fn reproduce_witness(report: &Report, id: &str) -> Result<Value> {
    verify(report)?;
    let w = report
        .witnesses
        .iter()
        .find(|w| w["witness_sha256"] == id)
        .context("unknown witness identity")?;
    let c = report
        .ledger
        .iter()
        .find(|c| c.case_id == w["case_id"])
        .context("witness case absent")?;
    let paired = c.report.as_ref().context("witness report absent")?;
    parameter::reproduce(&report.input.change, paired)?;
    ensure!(
        serde_json::to_value(outcome(paired, &report.input.spec.predicate)?)? == w["outcome"],
        "witness condition differs"
    );
    Ok(
        json!({"witness_sha256":id,"paired_report_sha256":paired["report_sha256"],"reproduced":true,
        "scope":"selected witness pair only; complete search not rerun","vm_calls":2,"outcome":w["outcome"]}),
    )
}
pub fn receipt(report: &Report) -> Result<Value> {
    let v = json!({"schema":SCHEMA,"search_input_sha256":report.search_input_sha256,"report_sha256":report.report_sha256,
        "change_spec_id":report.input.change.id()?,"parent_report_sha256":report.input.parent_report["report_sha256"],
        "fixed_commitment_sha256":report.input_identity["fixed_commitment_sha256"],
        "fixed_state":{"context":report.input.parent_report["retained_input"]["context"],
            "original_amount_raw":report.input.parent_report["retained_input"]["amount_raw"],
            "observed_source_balance_raw":report.input.parent_report["baseline"]["raw_token_state"]["source"]["before"]["public_amount_raw"],
            "fixture_sha256":report.input.parent_report["transfer_fixture_sha256"],
            "declaration":report.input.parent_report["proposed_declaration"],
            "clock":report.input.parent_report["shared_execution"]["clock"],
            "programs":report.input.parent_report["shared_execution"]["programs"]},
        "spec":report.input.spec,"summary":report.summary,
        "witnesses":report.witnesses,"offline":true,"authorization":false,"funds_moved":false});
    crate::cloud::privacy::scan_json("parameter search public summary", &v)?;
    Ok(v)
}
pub fn render(report: &Report) -> Result<String> {
    let receipt = receipt(report)?;
    let fixed = &receipt["fixed_state"];
    let mut rows=String::from("| Amount raw | Baseline credit raw | Proposed credit raw | Loss raw | Witness type | Identity |\n| --- | --- | --- | --- | --- | --- |\n");
    for w in &report.witnesses {
        let o = &w["outcome"];
        let display = |v: &Value| v.as_str().unwrap_or("unavailable").to_string();
        rows.push_str(&format!(
            "| {} | {} | {} | {} | {} | `{}` |\n",
            display(&w["amount_raw"]),
            display(&o["baseline_credit_raw"]),
            display(&o["proposed_credit_raw"]),
            display(&o["loss_raw"]),
            o["anomaly"]
                .as_str()
                .unwrap_or("declared recipient-loss condition"),
            display(&w["witness_sha256"])
        ));
    }
    let selected = report
        .witnesses
        .iter()
        .filter(|w| w["outcome"]["predicate"] == "matched")
        .min_by_key(|w| {
            w["amount_raw"]
                .as_str()
                .and_then(|n| n.parse::<u64>().ok())
                .unwrap_or(u64::MAX)
        })
        .or_else(|| report.witnesses.first());
    Ok(format!("Fixed proposal: {}\nObserved parent report: {}\nMint: {}\nSource: {} (observed balance {} raw)\nDestination: {}\nOriginal retained amount: {} raw. Current/proposed rates: {}/{} bps; unchanged fee cap: {} raw. Captured epoch: {}.\nFixture identity: {}\nHypothetical TransferChecked amounts: {}..={} raw; recipient_loss_exceeds {} raw\n{}\nExecuted {} distinct amounts; {} fresh VM calls; {} refinements. Untested amounts: {}. Domain fully enumerated: {}. Completion: {}.\nSmallest matching amount among executed cases: {} raw (no global minimum certified).\n\n{}\nReproduce a witness: eplyx parameter reproduce-witness --artifact <directory> --witness {}\nExact paired logs, watched states, derived messages, failure signatures and refinement history are retained in the portable CAS ledger.\n",
        report.input.change.id()?,report.input.parent_report["report_sha256"],
        fixed["context"]["mint"],fixed["context"]["source"],fixed["observed_source_balance_raw"],fixed["context"]["destination"],
        fixed["original_amount_raw"],fixed["declaration"]["current_bps"],fixed["declaration"]["proposed_bps"],fixed["declaration"]["maximum_fee_raw"],fixed["clock"]["epoch"],fixed["fixture_sha256"],
        report.input.spec.min_raw,report.input.spec.max_raw,report.input.spec.predicate.threshold(),report.summary["conclusion"].as_str().unwrap_or(""),
        report.summary["unique_evaluated_amounts"],report.summary["total_vm_calls"],report.summary["refinement_evaluations"],
        report.summary["untested_amount_count"],report.summary["numeric_domain_fully_enumerated"],report.summary["completion_status"],
        report.summary["smallest_matching_amount_among_executed_cases_raw"],rows,selected.and_then(|w|w["witness_sha256"].as_str()).unwrap_or("<none>")))
}
