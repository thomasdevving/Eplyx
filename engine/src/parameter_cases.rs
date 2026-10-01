//! Bounded local orchestration of the existing single-case Token-2022 contract.
//! References locate evidence; only validated proposal/input/contract identities
//! enter the selected-set identity. No fee calculation or VM implementation here.
use crate::{
    canonical, change::ChangeSpec, lifecycle::artifact, parameter_change as p,
    path::ProbeExecutionPlan, replay::hash_bytes,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};

pub const REVISION: &str = "eplyx-distinct-source-parameter-cases-v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub path: String,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contract {
    pub parameter_runtime: Value,
    pub runner_sha256: String,
    pub decoder_revision: String,
    pub programs: Vec<Value>,
    pub assumptions: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub case_id: String,
    pub input_sha256: String,
    pub input: Reference,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub revision: String,
    pub change: Reference,
    pub change_spec_id: String,
    pub contract: Contract,
    pub cases: Vec<Case>,
    pub case_set_id: String,
}
impl Manifest {
    pub fn id(&self) -> Result<String> {
        canonical::digest(
            &json!({"revision":self.revision,"schema_version":self.schema_version,
            "change_spec_id":self.change_spec_id,"contract":self.contract,
            "cases":self.cases.iter().map(|c|json!({"case_id":c.case_id,"input_sha256":c.input_sha256})).collect::<Vec<_>>() }),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Counts {
    pub selected_cases: usize,
    /// Complete paired reports; a partially attempted internal failure is not a completed pair.
    pub executed_pairs: usize,
    pub reconciled_pairs: usize,
    pub measured_consequence: usize,
    pub no_observed_consequence: usize,
    pub unavailable_or_failed: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    pub schema_version: u32,
    pub revision: String,
    pub case_set_id: String,
    pub change_spec_id: String,
    pub rows: Vec<Value>,
    pub reports: Vec<Reference>,
    pub counts: Counts,
    pub limitations: Vec<String>,
    pub result_sha256: String,
}
impl Summary {
    pub fn digest(&self) -> Result<String> {
        // Location and file formatting do not alter the analytical result identity.
        canonical::digest(
            &json!({"schema_version":self.schema_version,"revision":self.revision,
            "case_set_id":self.case_set_id,"change_spec_id":self.change_spec_id,
            "rows":self.rows,"counts":self.counts,"limitations":self.limitations}),
        )
    }
}

fn read_ref(base: &Path, reference: &Reference) -> Result<Vec<u8>> {
    ensure!(
        Path::new(&reference.path)
            .components()
            .all(|c| matches!(c, Component::Normal(_))),
        "reference must be a nonempty package-relative member"
    );
    let bytes = artifact::read_relative(base, &reference.path)?;
    ensure!(
        hash_bytes(&bytes) == reference.sha256,
        "reference content mismatch: {}",
        reference.path
    );
    Ok(bytes)
}
fn write(base: &Path, name: &str, bytes: &[u8]) -> Result<Reference> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(base.join(name))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(Reference {
        path: name.into(),
        sha256: hash_bytes(bytes),
    })
}
fn contract(input: &p::Input, plan: &ProbeExecutionPlan) -> Contract {
    Contract {
        parameter_runtime: p::runtime(),
        runner_sha256: hash_bytes(include_bytes!("parameter_cases.rs")),
        decoder_revision: input.fixture.decoder_revision.clone(),
        programs: plan
            .programs
            .iter()
            .map(|x| {
                json!({"program_id":x.program_id.to_string(),
            "loader":x.loader.to_string(),"elf_sha256":hash_bytes(&x.bytes)})
            })
            .collect(),
        assumptions: plan.assumptions.clone(),
    }
}
fn preflight(spec: &ChangeSpec, input: &p::Input) -> Result<ProbeExecutionPlan> {
    p::binding(spec)?;
    let change = spec
        .as_protocol_parameter_change()
        .context("parameter proposal required")?;
    ensure!(
        input.context.mint == change.target.config_account
            && input.context.program == change.target.program_id,
        "current_state_mismatch: transfer target differs from proposal"
    );
    let plan = input.validate()?;
    let mint = &plan
        .accounts
        .iter()
        .find(|a| a.address == input.context.mint)
        .context("mint absent")?
        .account;
    let (expected, bps) = change
        .operation
        .values()
        .context("Token-2022 operation required")?;
    // Reuse the engine's exact expected-state/schedule admission, without executing.
    p::mutate(mint, expected, bps, plan.clock.epoch)?;
    Ok(plan)
}
fn admit(manifest: &Manifest, spec: &ChangeSpec, inputs: &[p::Input]) -> Result<()> {
    ensure!(
        manifest.schema_version == 1 && manifest.revision == REVISION,
        "unsupported case-set revision"
    );
    ensure!(
        (2..=3).contains(&manifest.cases.len()) && inputs.len() == manifest.cases.len(),
        "select two or three cases"
    );
    ensure!(
        manifest.change_spec_id == spec.id()?,
        "proposal identity mismatch"
    );
    ensure!(
        manifest.case_set_id == manifest.id()?,
        "case-set identity mismatch"
    );
    let mut sources = BTreeSet::new();
    let mut previous = None;
    for (case, input) in manifest.cases.iter().zip(inputs) {
        ensure!(
            case.input_sha256 == input.sha256()?
                && case.case_id == format!("case_{}", input.sha256()?),
            "case input identity mismatch"
        );
        ensure!(
            previous.is_none_or(|p: &str| p < case.case_id.as_str()),
            "case identifiers must be distinct and sorted"
        );
        previous = Some(case.case_id.as_str());
        ensure!(
            sources.insert(&input.context.source),
            "duplicate source account: {}",
            input.context.source
        );
        let plan =
            preflight(spec, input).with_context(|| format!("case {} admission", case.case_id))?;
        ensure!(
            contract(input, &plan) == manifest.contract,
            "case {} executable/interpretation/runtime contract mismatch",
            case.case_id
        );
    }
    Ok(())
}
struct Loaded {
    manifest: Manifest,
    spec: ChangeSpec,
    change: Vec<u8>,
    inputs: Vec<p::Input>,
    bytes: Vec<Vec<u8>>,
}
fn load(path: &Path) -> Result<Loaded> {
    let base = path.parent().context("manifest parent missing")?;
    let manifest: Manifest = artifact::load(path)?;
    ensure!(
        (2..=3).contains(&manifest.cases.len()),
        "select two or three cases"
    );
    let change = read_ref(base, &manifest.change)?;
    let spec = ChangeSpec::parse(&change)?;
    let bytes = manifest
        .cases
        .iter()
        .map(|c| read_ref(base, &c.input))
        .collect::<Result<Vec<_>>>()?;
    let inputs = bytes
        .iter()
        .map(|b| serde_json::from_slice(b).context("invalid retained input"))
        .collect::<Result<Vec<_>>>()?;
    admit(&manifest, &spec, &inputs)?;
    Ok(Loaded {
        manifest,
        spec,
        change,
        inputs,
        bytes,
    })
}

/// Prepare a private request package from existing exact retained inputs. No VM execution.
pub fn prepare(change: &Path, inputs: &[PathBuf], out: &Path) -> Result<Manifest> {
    ensure!((2..=3).contains(&inputs.len()), "select two or three cases");
    let change_bytes = artifact::read(change)?;
    let spec = ChangeSpec::parse(&change_bytes)?;
    let mut selected = inputs
        .iter()
        .map(|p| -> Result<_> {
            let bytes = artifact::read(p)?;
            let input: p::Input = serde_json::from_slice(&bytes)?;
            let plan = preflight(&spec, &input)?;
            let hash = input.sha256()?;
            Ok((
                Case {
                    case_id: format!("case_{hash}"),
                    input_sha256: hash,
                    input: Reference {
                        path: String::new(),
                        sha256: hash_bytes(&bytes),
                    },
                    label: None,
                },
                input,
                bytes,
                plan,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    selected.sort_by(|a, b| a.0.case_id.cmp(&b.0.case_id));
    let mut manifest = Manifest {
        schema_version: 1,
        revision: REVISION.into(),
        change: Reference {
            path: "change.json".into(),
            sha256: hash_bytes(&change_bytes),
        },
        change_spec_id: spec.id()?,
        contract: contract(&selected[0].1, &selected[0].3),
        cases: selected.iter().map(|x| x.0.clone()).collect(),
        case_set_id: String::new(),
    };
    for (i, c) in manifest.cases.iter_mut().enumerate() {
        c.input.path = format!("input-{i}.json");
    }
    manifest.case_set_id = manifest.id()?;
    admit(
        &manifest,
        &spec,
        &selected.iter().map(|x| x.1.clone()).collect::<Vec<_>>(),
    )?;
    std::fs::create_dir(out).context("output package must be new")?;
    write(out, "change.json", &change_bytes)?;
    for (case, (_, _, bytes, _)) in manifest.cases.iter().zip(&selected) {
        write(out, &case.input.path, bytes)?;
    }
    write(
        out,
        "manifest.json",
        canonical::document(&manifest)?.as_bytes(),
    )?;
    Ok(manifest)
}

fn quantities(report: &Value) -> Value {
    let b = &report["baseline"];
    let p = &report["proposed"];
    let available = b["execution"]["success"] == true
        && p["execution"]["success"] == true
        && b["reconciliation"]["reconciled"] == true
        && p["reconciliation"]["reconciled"] == true;
    let value = |side: &Value, path: &str| {
        if available {
            side.pointer(path)
                .filter(|v| v.is_string())
                .cloned()
                .unwrap_or(Value::Null)
        } else {
            Value::Null
        }
    };
    let credit_b = value(b, "/reconciliation/output_received_raw");
    let credit_p = value(p, "/reconciliation/output_received_raw");
    let delta = |subject: &str, before: &Value, after: &Value| {
        if before.is_null() || after.is_null() {
            return Value::Null;
        }
        if before == after {
            return json!("0");
        }
        report["findings"]
            .as_array()
            .and_then(|f| {
                f.iter().find(|f| {
                    f["fingerprint"].as_str().is_some_and(|s| {
                        s.starts_with(&format!("token-2022/transfer_checked/economic/{subject}/"))
                    }) && f["baseline_raw"] == *before
                        && f["proposed_raw"] == *after
                })
            })
            .and_then(|f| f["delta_raw"].as_str())
            .map(|s| json!(s))
            .unwrap_or(Value::Null)
    };
    let fee_b = value(
        b,
        "/reconciliation/token_accounts/1/withheld_fee_change_raw",
    );
    let fee_p = value(
        p,
        "/reconciliation/token_accounts/1/withheld_fee_change_raw",
    );
    json!({"baseline_recipient_credit_raw":credit_b,"proposed_recipient_credit_raw":credit_p,
        "recipient_credit_difference_raw":delta("recipient_tokens_received",&credit_b,&credit_p),
        "baseline_destination_withheld_change_raw":fee_b,"proposed_destination_withheld_change_raw":fee_p,
        "destination_withheld_difference_raw":delta("destination_withheld_transfer_fee",&fee_b,&fee_p)})
}
fn summarize(manifest: &Manifest, reports: &[Value], refs: Vec<Reference>) -> Result<Summary> {
    let mut counts = Counts {
        selected_cases: reports.len(),
        executed_pairs: 0,
        reconciled_pairs: 0,
        measured_consequence: 0,
        no_observed_consequence: 0,
        unavailable_or_failed: 0,
    };
    let mut rows = Vec::new();
    for (case, r) in manifest.cases.iter().zip(reports) {
        let paired = r["execution_performed"] == true;
        let reconciled = r["baseline"]["reconciliation"]["reconciled"] == true
            && r["proposed"]["reconciliation"]["reconciled"] == true;
        counts.executed_pairs += usize::from(paired);
        counts.reconciled_pairs += usize::from(paired && reconciled);
        match r["status"].as_str() {
            Some("semantic_consequence_observed")
                if reconciled
                    && r["baseline"]["execution"]["success"] == true
                    && r["proposed"]["execution"]["success"] == true =>
            {
                counts.measured_consequence += 1
            }
            Some("no_observed_consequence") => counts.no_observed_consequence += 1,
            _ => counts.unavailable_or_failed += 1,
        }
        rows.push(json!({"case_id":case.case_id,"input_sha256":case.input_sha256,
            "source":r["retained_input"]["context"]["source"],"owner":r["retained_input"]["context"]["owner"],
            "destination":r["retained_input"]["context"]["destination"],
            "destination_owner":r["retained_input"]["context"]["destination_owner"],
            "capture_sha256":r["observed_capture_sha256"],"transfer_fixture_sha256":r["transfer_fixture_sha256"],
            "captured_at":r["retained_input"]["fixture"]["captured_at"],"clock":r["shared_execution"]["clock"],
            "amount_raw":r["retained_input"]["amount_raw"],
            "initial_source_balance_raw":r["baseline"]["raw_token_state"]["source"]["before"]["public_amount_raw"],
            "initial_destination_balance_raw":r["baseline"]["raw_token_state"]["destination"]["before"]["public_amount_raw"],
            "quantities":quantities(r),"status":r["status"],"execution_performed":paired,
            "baseline_execution_success":r["baseline"]["execution"]["success"],"proposed_execution_success":r["proposed"]["execution"]["success"],
            "baseline_reconciliation":r["baseline"]["reconciliation"]["reconciled"],"proposed_reconciliation":r["proposed"]["reconciliation"]["reconciled"],
            "failure":r["failure"],"report_sha256":r["report_sha256"],"limitations":r["limitations"]}));
    }
    let mut s = Summary { schema_version:1,revision:REVISION.into(),case_set_id:manifest.case_set_id.clone(),
        change_spec_id:manifest.change_spec_id.clone(),rows,reports:refs,counts,
        limitations:vec!["Explicit selected cases only: no production coverage, population, representativeness or aggregate funds claim.".into(),
            "Within each case only the mint fee field differs. Cross-case amounts, balances and observation times differ; source accounts do not identify people.".into(),
            "Each single-case report retains its original authorization, signer, captured Clock and local runtime limitations. Offline repetition qualified on one host/runtime only.".into()],result_sha256:String::new() };
    s.result_sha256 = s.digest()?;
    Ok(s)
}
fn bind_report(spec: &ChangeSpec, case: &Case, report: &Value) -> Result<()> {
    p::verify(spec, report)?;
    ensure!(
        report["analysis_input_sha256"] == case.input_sha256,
        "report references another selected input"
    );
    Ok(())
}

/// Every reference and case is admitted before the first paired analysis.
pub fn analyze(manifest_path: &Path, out: &Path) -> Result<Summary> {
    let mut l = load(manifest_path)?;
    std::fs::create_dir(out).context("output package must be new")?;
    l.manifest.change = write(out, "change.json", &l.change)?;
    for (i, (case, bytes)) in l.manifest.cases.iter_mut().zip(&l.bytes).enumerate() {
        case.input = write(out, &format!("input-{i}.json"), bytes)?;
    }
    write(
        out,
        "manifest.json",
        canonical::document(&l.manifest)?.as_bytes(),
    )?;
    let mut reports = Vec::new();
    let mut refs = Vec::new();
    let mut errors = Vec::new();
    for (i, (case, input)) in l.manifest.cases.iter().zip(&l.inputs).enumerate() {
        let report = match p::analyze(&l.spec, input).and_then(|r| {
            bind_report(&l.spec, case, &r)?;
            Ok(r)
        }) {
            Ok(r) => r,
            Err(e) => {
                errors.push(json!({"case_id":case.case_id,"status":"internal_execution_failure","detail":format!("{e:#}"),"report_available":false}));
                continue;
            }
        };
        refs.push(write(
            out,
            &format!("report-{i}.json"),
            canonical::document(&report)?.as_bytes(),
        )?);
        reports.push(report);
    }
    if !errors.is_empty() {
        write(out,"analysis-errors.json",canonical::document(&json!({"status":"case_set_evidence_blocked","case_set_id":l.manifest.case_set_id,"completed_reports":refs,"errors":errors}))?.as_bytes())?;
        anyhow::bail!("case_set_evidence_blocked: per-case internal failures retained in analysis-errors.json; no complete summary");
    }
    let summary = summarize(&l.manifest, &reports, refs)?;
    write(
        out,
        "summary.json",
        canonical::document(&summary)?.as_bytes(),
    )?;
    write(out, "summary.md", markdown(&summary).as_bytes())?;
    Ok(summary)
}
fn markdown(s: &Summary) -> String {
    let mut text = format!("Selected retained cases\n\nProposal `{}`; case set `{}`; result `{}`.\n\n| Source | Raw amount | Baseline credit | Proposed credit | Difference | Status |\n|---|---:|---:|---:|---:|---|\n",s.change_spec_id,s.case_set_id,s.result_sha256);
    let display = |v: &Value| v.as_str().unwrap_or("unavailable").to_owned();
    for r in &s.rows {
        text += &format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            display(&r["source"]),
            display(&r["amount_raw"]),
            display(&r["quantities"]["baseline_recipient_credit_raw"]),
            display(&r["quantities"]["proposed_recipient_credit_raw"]),
            display(&r["quantities"]["recipient_credit_difference_raw"]),
            display(&r["status"])
        );
    }
    text += &format!("\n{} selected cases; {} completed execution pairs; {} reconciled pairs; {} measured consequences; {} no observed consequence; {} unavailable/failed.\n\n",s.counts.selected_cases,s.counts.executed_pairs,s.counts.reconciled_pairs,s.counts.measured_consequence,s.counts.no_observed_consequence,s.counts.unavailable_or_failed);
    for limitation in &s.limitations {
        text += limitation;
        text += "\n\n";
    }
    text += "Complete evidence and per-case identities/coordinates are in summary.json and the intact report-N.json files.\n\nOffline check: eplyx parameter cases verify --package <this-directory>\n\nOffline repeat: eplyx parameter cases reproduce --package <this-directory>\n";
    text
}

/// Verify all individual reports, and optionally repeat each independently.
/// Per-case errors remain visible; an earlier failure does not skip later cases.
pub fn check(package: &Path, reproduce: bool) -> Result<Value> {
    let l = load(&package.join("manifest.json"))?;
    let saved: Summary = artifact::load(&package.join("summary.json"))?;
    ensure!(
        saved.reports.len() == l.manifest.cases.len(),
        "missing or extra case report reference"
    );
    let mut reports = Vec::new();
    let mut attempts = Vec::new();
    for (case, reference) in l.manifest.cases.iter().zip(&saved.reports) {
        let report =
            read_ref(package, reference).and_then(|b| Ok(serde_json::from_slice::<Value>(&b)?));
        let result = report.and_then(|r| {
            bind_report(&l.spec, case, &r)?;
            reports.push(r.clone());
            Ok(r["report_sha256"].clone())
        });
        attempts.push(match result {
            Ok(hash) => json!({"case_id":case.case_id,"report_sha256":hash,"verified":true,"reproduced":Value::Null,"error":Value::Null}),
            Err(e) => json!({"case_id":case.case_id,"verified":false,"reproduced":Value::Null,"error":format!("{e:#}")}),
        });
    }
    let all = attempts.iter().all(|a| a["error"].is_null());
    let summary_matches = if reports.len() == saved.reports.len() {
        let rebuilt = summarize(&l.manifest, &reports, saved.reports.clone())?;
        serde_json::to_value(&saved)? == serde_json::to_value(rebuilt)?
    } else {
        false
    };
    let mut ok = all && summary_matches;
    // Check *every* input/report reference and the complete saved projection
    // before executing. An invalid package leaves every per-case check visible.
    let reproduction_performed = ok && reproduce;
    if reproduction_performed {
        for (attempt, report) in attempts.iter_mut().zip(&reports) {
            match p::reproduce(&l.spec, report) {
                Ok(()) => attempt["reproduced"] = true.into(),
                Err(e) => {
                    attempt["reproduced"] = false.into();
                    attempt["error"] = format!("{e:#}").into();
                    ok = false;
                }
            }
        }
    }
    Ok(
        json!({"status":if ok {"selected_case_set_evaluated"} else {"case_set_evidence_blocked"},
        "case_set_id":l.manifest.case_set_id,"result_sha256":saved.result_sha256,"offline":true,
        "summary_verified":summary_matches,"reproduction_performed":reproduction_performed,
        "reproduction_blocker":if reproduce && !reproduction_performed {json!("invalid package evidence; no replay executed")} else {Value::Null},"cases":attempts}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn synthetic_missing_or_unreconciled_quantities_are_unavailable() {
        for report in [
            json!({}),
            json!({"baseline":{"execution":{"success":true},"reconciliation":{"reconciled":false,"output_received_raw":"9"}},"proposed":{"execution":{"success":true},"reconciliation":{"reconciled":true,"output_received_raw":"8"}}}),
        ] {
            assert!(quantities(&report)
                .as_object()
                .unwrap()
                .values()
                .all(Value::is_null));
        }
        let mut synthetic = json!({"baseline":{"execution":{"success":true},"reconciliation":{"reconciled":true}},"proposed":{"execution":{"success":true},"reconciliation":{"reconciled":true}}});
        assert!(quantities(&synthetic)
            .as_object()
            .unwrap()
            .values()
            .all(Value::is_null));
        synthetic["baseline"]["reconciliation"]["output_received_raw"] = json!("1");
        synthetic["proposed"]["reconciliation"]["output_received_raw"] = json!("1");
        assert_eq!(
            quantities(&synthetic)["recipient_credit_difference_raw"],
            "0"
        );
        synthetic["proposed"]["reconciliation"]["output_received_raw"] = json!("2");
        assert!(quantities(&synthetic)["recipient_credit_difference_raw"].is_null());
        synthetic["baseline"]["execution"]["success"] = json!(false);
        assert!(quantities(&synthetic)
            .as_object()
            .unwrap()
            .values()
            .all(Value::is_null));
    }
}
