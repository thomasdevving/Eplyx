//! Portable rollout evidence. Same conventions as the interaction artifact:
//! a fresh directory, content-addressed `objects/<sha256>` written first,
//! `report.md`, then `manifest.json` last. Logical names in the manifest
//! (`states/<id>`, `executions/<id>`, `accounts/<id>`, `programs/…`,
//! `proposals/…`) map to CAS hashes; a state, execution or account's id is its
//! own canonical digest, so each is stored exactly once.
use std::{collections::BTreeMap, path::Path};

use anyhow::{ensure, Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use super::{
    verify, world::Execution, world::State, Analysis, Evidence, Input, Report, ScenarioName,
    StepOutcome,
};
use crate::{canonical, lifecycle::artifact as files, replay, types::AccountSnapshot};

pub const ARTIFACT_SCHEMA: &str = "eplyx-rollout-rehearsal-artifact-v1";
const MAX_OBJECTS: usize = 512;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: String,
    pub analysis_input_id: String,
    pub report_sha256: String,
    pub objects: BTreeMap<String, String>,
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    ensure!(
        bytes.len() as u64 <= files::MAX_BYTES,
        "artifact object exceeds byte bound"
    );
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn put(root: &Path, bytes: &[u8]) -> Result<String> {
    let hash = replay::hash_bytes(bytes);
    let path = root.join("objects").join(&hash);
    if path.exists() {
        ensure!(files::read(&path)? == bytes, "CAS collision");
    } else {
        write_new(&path, bytes)?;
    }
    Ok(hash)
}

fn get(root: &Path, hash: &str) -> Result<Vec<u8>> {
    ensure!(
        hash.len() == 64
            && hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid CAS reference"
    );
    let bytes = files::read(files::member(root, &format!("objects/{hash}"))?)?;
    ensure!(replay::hash_bytes(&bytes) == hash, "CAS content mismatch");
    Ok(bytes)
}

/// Every logical object of an analysis and its exact bytes.
fn objects(analysis: &Analysis) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut out = BTreeMap::new();
    out.insert("input.json".into(), doc(&analysis.input)?);
    out.insert("contract.json".into(), doc(&analysis.contract)?);
    out.insert("report.json".into(), doc(&analysis.report)?);
    out.insert(
        "proposals/upgrade.json".into(),
        analysis.input.upgrade.to_document()?.into_bytes(),
    );
    out.insert(
        "proposals/parameter.json".into(),
        analysis.input.parameter.to_document()?.into_bytes(),
    );
    out.insert(
        "programs/candidate.so".into(),
        analysis.input.candidate.elf.clone(),
    );
    for p in &analysis.input.historical.programs {
        out.insert(format!("programs/{}.so", p.program_id), p.elf.clone());
    }
    for (id, state) in &analysis.evidence.states {
        out.insert(format!("states/{id}"), doc(state)?);
    }
    for (id, x) in &analysis.evidence.executions {
        out.insert(format!("executions/{id}"), doc(x)?);
    }
    for (id, a) in &analysis.evidence.accounts {
        out.insert(format!("accounts/{id}"), doc(a)?);
    }
    out.insert("report.md".into(), readable(analysis)?.into_bytes());
    Ok(out)
}

fn doc<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>> {
    Ok(canonical::document(value)?.into_bytes())
}

/// Verify first, then write into a fresh directory; manifest last.
pub fn save(analysis: &Analysis, directory: &Path) -> Result<Manifest> {
    verify(analysis)?;
    let objects = objects(analysis)?;
    ensure!(
        objects.len() <= MAX_OBJECTS,
        "artifact object count exceeds bound"
    );
    std::fs::create_dir(directory).context("output directory must be fresh")?;
    std::fs::create_dir(directory.join("objects"))?;
    let mut index = BTreeMap::new();
    for (name, bytes) in &objects {
        index.insert(name.clone(), put(directory, bytes)?);
    }
    write_new(&directory.join("report.md"), &objects["report.md"])?;
    let manifest = Manifest {
        schema: ARTIFACT_SCHEMA.into(),
        analysis_input_id: analysis.report.analysis_input_id.clone(),
        report_sha256: analysis.report.report_sha256.clone(),
        objects: index,
    };
    write_new(
        &directory.join("manifest.json"),
        canonical::document(&manifest)?.as_bytes(),
    )?;
    Ok(manifest)
}

fn strict<T: DeserializeOwned + Serialize>(bytes: &[u8], what: &str) -> Result<T> {
    let value: T =
        serde_json::from_slice(bytes).with_context(|| format!("{what} does not parse"))?;
    ensure!(
        canonical::document(&value)?.as_bytes() == bytes,
        "{what} is not in canonical form"
    );
    Ok(value)
}

/// Read-only. Checks every reference and byte hash, parses every object
/// strictly, then [`verify`]s the analysis. Never executes a VM.
pub fn load(directory: &Path) -> Result<Analysis> {
    ensure!(
        !std::fs::symlink_metadata(directory)?
            .file_type()
            .is_symlink(),
        "artifact directory symlink forbidden"
    );
    let bytes = files::read(files::member(directory, "manifest.json")?)?;
    let manifest: Manifest = strict(&bytes, "manifest")?;
    ensure!(
        manifest.schema == ARTIFACT_SCHEMA && manifest.objects.len() <= MAX_OBJECTS,
        "manifest schema or object count differs"
    );
    let object = |name: &str| -> Result<Vec<u8>> {
        get(
            directory,
            manifest
                .objects
                .get(name)
                .with_context(|| format!("manifest lacks {name}"))?,
        )
    };
    let input: Input = strict(&object("input.json")?, "input")?;
    let contract: serde_json::Value = strict(&object("contract.json")?, "contract")?;
    let report: Report = strict(&object("report.json")?, "report")?;
    let mut evidence = Evidence::default();
    for id in &report.evidence.states {
        evidence.states.insert(
            id.clone(),
            strict::<State>(&object(&format!("states/{id}"))?, "state")?,
        );
    }
    for id in &report.evidence.executions {
        evidence.executions.insert(
            id.clone(),
            strict::<Execution>(&object(&format!("executions/{id}"))?, "execution")?,
        );
    }
    for id in &report.evidence.accounts {
        evidence.accounts.insert(
            id.clone(),
            strict::<AccountSnapshot>(&object(&format!("accounts/{id}"))?, "account")?,
        );
    }
    let analysis = Analysis {
        input,
        contract,
        report,
        evidence,
    };
    verify(&analysis)?;
    let expected = objects(&analysis)?;
    ensure!(
        expected.keys().eq(manifest.objects.keys()),
        "manifest names objects the analysis does not derive, or omits some"
    );
    for (name, bytes) in &expected {
        ensure!(
            manifest.objects[name] == replay::hash_bytes(bytes),
            "object {name} differs from the verified analysis"
        );
        get(directory, &manifest.objects[name])?;
    }
    ensure!(
        manifest.analysis_input_id == analysis.report.analysis_input_id
            && manifest.report_sha256 == analysis.report.report_sha256,
        "manifest identities differ from the report"
    );
    ensure!(
        files::read(files::member(directory, "report.md")?)? == expected["report.md"],
        "readable report differs"
    );
    Ok(analysis)
}

fn word<T: Serialize>(v: T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default()
}

/// Deterministic presentation of the sealed report; never a second analysis.
pub fn readable(analysis: &Analysis) -> Result<String> {
    use std::fmt::Write;
    let r = &analysis.report;
    let c = &r.comparison;
    let mut t = String::new();
    writeln!(t, "# Program-upgrade rollout rehearsal\n\n{}\n", r.question)?;
    writeln!(t, "Status: `{}`\n\n{}\n", word(c.status), c.statement)?;
    if let Some(f) = &c.finding {
        writeln!(t, "Finding: `{f}`\n")?;
    }
    writeln!(
        t,
        "Analysis input `{}`\nReport `{}`\nUpgrade ChangeSpec `{}`\nParameter ChangeSpec `{}`\n",
        r.analysis_input_id, r.report_sha256, r.upgrade_change_spec_id, r.parameter_change_spec_id
    )?;
    let candidate = &analysis.input.candidate;
    writeln!(
        t,
        "Candidate `{}` ({} bytes), profile `{}`. Constructed rollout counterexample; not an upstream release; not intended for deployment.\n",
        candidate.elf_sha256,
        candidate.elf.len(),
        r.preflight
            .candidate_profile
            .clone()
            .unwrap_or_else(|| "none (not qualified)".into())
    )?;
    let cap = &r.preflight.capacity;
    writeln!(
        t,
        "ProgramData `{}`: capacity {} bytes, candidate {} bytes, sufficient: {}.{}\n",
        cap.programdata_address,
        cap.capacity_bytes,
        cap.required_bytes,
        cap.sufficient,
        r.preflight
            .upgrade_blocker
            .as_ref()
            .map(|b| format!(" Blocker `{}`: {}", b.reason, b.detail))
            .unwrap_or_default()
    )?;
    writeln!(
        t,
        "## Anchors\n\n| Anchor | Status | Reason |\n| --- | --- | --- |\n| S0 → DepositSol historical fidelity | {} | {} |\n| Installed V2 vs overlay V2 | {} | {} |\n",
        word(r.anchors.baseline_world_fidelity.status),
        r.anchors.baseline_world_fidelity.reason.clone().unwrap_or_default(),
        word(r.anchors.installed_overlay.status),
        r.anchors.installed_overlay.reason.clone().unwrap_or_default()
    )?;
    writeln!(
        t,
        "## Scenarios\n\nEach scenario starts from S0 `{}`.\n",
        r.scenarios[0].initial_state_id
    )?;
    for s in &r.scenarios {
        writeln!(
            t,
            "### {}\n\nScenario `{}`\n\n| # | Step | Outcome | Reason | Slot | After state |\n| ---: | --- | --- | --- | ---: | --- |",
            word(s.name),
            s.scenario_id
        )?;
        for step in &s.steps {
            writeln!(
                t,
                "| {} | {} | {} | {} | {} | {} |",
                step.index,
                word(step.step),
                word(step.outcome),
                step.reason.clone().unwrap_or_default().replace('|', "/"),
                step.clock_before
                    .as_ref()
                    .map(|c| c.slot.to_string())
                    .unwrap_or_default(),
                step.after_state_id.clone().unwrap_or_default()
            )?;
        }
        writeln!(t)?;
    }
    writeln!(
        t,
        "## Final DepositSol\n\nOrder A: `{}`; order B: `{}`.\n\n| Metric | Order A | Order B |\n| --- | ---: | ---: |",
        word(c.final_action_outcomes[0]),
        word(c.final_action_outcomes[1])
    )?;
    for (m, q) in &c.final_action_metrics {
        let show = |q: &super::Quantity| {
            q.value.clone().unwrap_or_else(|| {
                format!(
                    "unavailable: {}",
                    q.unavailable_reason.clone().unwrap_or_default()
                )
            })
        };
        writeln!(
            t,
            "| {m} | {} | {} |",
            show(&q.order_a).replace('|', "/"),
            show(&q.order_b).replace('|', "/")
        )?;
    }
    if let Some(d) = &c.first_divergence {
        writeln!(
            t,
            "\nFirst divergence: `{}` (order A step {}: {}; order B step {}: {}).",
            word(d.step),
            d.order_a_index,
            word(d.order_a),
            d.order_b_index,
            word(d.order_b)
        )?;
    }
    for s in [ScenarioName::OrderA, ScenarioName::OrderB] {
        if let Some(s) = r.scenario(s) {
            let stopped = s.steps.iter().find(|x| x.outcome != StepOutcome::Verified);
            if let Some(x) = stopped {
                writeln!(
                    t,
                    "\n{}: step {} {} — {}",
                    word(s.name),
                    x.index,
                    word(x.outcome),
                    x.detail.clone().unwrap_or_default()
                )?;
            }
        }
    }
    writeln!(t, "\n## Limitations\n")?;
    for l in &r.limitations {
        writeln!(t, "- {l}")?;
    }
    writeln!(
        t,
        "\n## Portable evidence\n\nEvery state, execution and account is in `objects/`, named by `manifest.json` (written last). No repository, bundle, provider or credential is needed.\n\nVerify without execution: `eplyx rollout verify --artifact <directory>`\nRe-execute offline: `eplyx rollout reproduce --artifact <directory>`"
    )?;
    Ok(t)
}
