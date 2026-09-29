//! Immutable local order evidence using the existing content-addressed store.
//! Replay has no provider parameter and never repairs evidence.
use super::{
    order::{self, Analysis},
    world::World,
};
use crate::{
    canonical,
    change::ChangeSpec,
    universal::evidence::{EvidenceKind, EvidenceRef, EvidenceStore},
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

const MAX_FILE: u64 = 512 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: String,
    change: EvidenceRef,
    world: EvidenceRef,
    candidate: EvidenceRef,
    analysis: EvidenceRef,
    binding_id: String,
    order_case_ids: [String; 2],
}
// Share complete AccountSnapshot content across world, initial, intermediate and
// final states. Explicit KnownAbsent stays in the canonical snapshot document.
fn externalize(store: &EvidenceStore, value: &mut Value) -> Result<()> {
    match value {
        Value::Object(map) => {
            if map.contains_key("lamports")
                && map.contains_key("rent_epoch")
                && map.contains_key("data")
                && map.contains_key("owner")
            {
                let reference = store.put(
                    EvidenceKind::AccountContent,
                    canonical::document(map)?.as_bytes(),
                )?;
                *value = json!({"order_account_content": reference});
            } else {
                for child in map.values_mut() {
                    externalize(store, child)?;
                }
            }
        }
        Value::Array(items) => {
            for child in items {
                externalize(store, child)?;
            }
        }
        _ => (),
    }
    Ok(())
}
fn bounded_read(path: &Path, max: u64) -> Result<Vec<u8>> {
    ensure!(!path.is_symlink(), "symlinked order evidence");
    let file = fs::File::open(path)?;
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= max,
        "order evidence exceeds bound"
    );
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= max, "order evidence exceeds bound");
    Ok(bytes)
}
fn read(store: &EvidenceStore, reference: &EvidenceRef) -> Result<Vec<u8>> {
    let path = store.path(reference)?;
    // Also refuse symlinked intermediate directories before reading any CAS member.
    for ancestor in path.ancestors() {
        ensure!(!ancestor.is_symlink(), "symlinked order evidence");
    }
    let bytes = bounded_read(&path, MAX_FILE)?;
    ensure!(
        crate::replay::hash_bytes(&bytes) == reference.sha256,
        "order evidence content hash differs"
    );
    Ok(bytes)
}
fn hydrate(store: &EvidenceStore, value: &mut Value) -> Result<()> {
    match value {
        Value::Object(map) if map.contains_key("order_account_content") => {
            ensure!(map.len() == 1, "invalid account reference");
            let reference: EvidenceRef =
                serde_json::from_value(map["order_account_content"].clone())?;
            ensure!(
                reference.kind == EvidenceKind::AccountContent,
                "wrong account evidence kind"
            );
            let account: crate::types::AccountSnapshot =
                serde_json::from_slice(&read(store, &reference)?)?;
            *value = serde_json::to_value(account)?;
        }
        Value::Object(map) => {
            for child in map.values_mut() {
                hydrate(store, child)?;
            }
        }
        Value::Array(items) => {
            for child in items {
                hydrate(store, child)?;
            }
        }
        _ => (),
    }
    Ok(())
}
fn put<T: Serialize>(store: &EvidenceStore, kind: EvidenceKind, value: &T) -> Result<EvidenceRef> {
    let mut value = serde_json::to_value(value)?;
    externalize(store, &mut value)?;
    store.put(kind, canonical::document(&value)?.as_bytes())
}
fn get<T: serde::de::DeserializeOwned>(
    store: &EvidenceStore,
    reference: &EvidenceRef,
) -> Result<T> {
    let mut value: Value = serde_json::from_slice(&read(store, reference)?)?;
    hydrate(store, &mut value)?;
    Ok(serde_json::from_value(value)?)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

pub fn save(
    root: &Path,
    change: &ChangeSpec,
    world: &World,
    candidate: &[u8],
    sources: [&str; 2],
) -> Result<Analysis> {
    let analysis = order::analyse(change, world, candidate, sources)?;
    for ancestor in root.ancestors() {
        ensure!(!ancestor.is_symlink(), "symlinked output directory");
    }
    fs::create_dir(root).context("order output must be a new directory")?;
    let store = EvidenceStore::at(root.join("evidence"));
    let manifest = Manifest {
        version: order::VERSION.into(),
        change: put(&store, EvidenceKind::ClosureProof, change)?,
        world: put(&store, EvidenceKind::Checkpoint, world)?,
        candidate: store.put(EvidenceKind::ProgramBinary, candidate)?,
        analysis: put(&store, EvidenceKind::Execution, &analysis)?,
        binding_id: analysis.binding.id()?,
        order_case_ids: analysis.comparison.order_case_ids.clone(),
    };
    write_new(&root.join("report.md"), markdown(&analysis).as_bytes())?;
    // Written last: incomplete directories are not replayable cases.
    write_new(
        &root.join("case.json"),
        canonical::document(&manifest)?.as_bytes(),
    )?;
    Ok(analysis)
}

pub fn reproduce(root: &Path) -> Result<Analysis> {
    reproduce_inner(root).map_err(order::evidence)
}
fn reproduce_inner(root: &Path) -> Result<Analysis> {
    for ancestor in root.ancestors() {
        ensure!(!ancestor.is_symlink(), "symlinked order evidence");
    }
    let manifest: Manifest =
        serde_json::from_slice(&bounded_read(&root.join("case.json"), MAX_FILE)?)?;
    ensure!(
        manifest.version == order::VERSION,
        "unsupported order artifact version"
    );
    let store = EvidenceStore::at(root.join("evidence"));
    let change: ChangeSpec = get(&store, &manifest.change)?;
    let world: World = get(&store, &manifest.world)?;
    let candidate = read(&store, &manifest.candidate)?;
    let expected: Analysis = get(&store, &manifest.analysis)?;
    ensure!(
        expected.binding.id()? == manifest.binding_id
            && expected.comparison.order_case_ids == manifest.order_case_ids,
        "order manifest identity mismatch"
    );
    change.validate()?;
    ensure!(
        change.id()? == expected.binding.change_spec_id
            && world.sha256()? == expected.binding.world_id
            && order::world_content_id(&world)? == expected.binding.world_content_sha256
            && crate::change::ExecutableArtifact::of(&candidate) == expected.binding.candidate
            && expected.binding.runtime_id == order::runtime_identity()?,
        "proposal/candidate/world/runtime identity mismatch"
    );
    for (id, state) in &expected.states {
        ensure!(state.id()? == *id, "tampered intermediate state identity");
    }
    let actual = order::analyse(
        &change,
        &world,
        &candidate,
        [
            &expected.binding.units[0].source_account,
            &expected.binding.units[1].source_account,
        ],
    )?;
    ensure!(actual == expected, "offline order replay differs: inputs, closure, scenarios, intermediate/final states, UnitExecution, reconciliation or comparison");
    ensure!(
        bounded_read(&root.join("report.md"), MAX_FILE)? == markdown(&actual).as_bytes(),
        "saved order report differs"
    );
    Ok(actual)
}

pub fn markdown(a: &Analysis) -> String {
    let b = &a.binding;
    let mut s = format!("# Bounded migration order analysis\n\n## Initial conditions\n\nChangeSpec: `{}`\n\nWorld: `{}` ({:?})\n\nCandidate: `{}` ({} bytes)\n\nFixed Clock: slot {}, Unix {}; runtime `{}`. No time advancement.\n\nShared reserve: `{}`, {} raw.\n\nA: `{}` (`{}`), {} source raw.\n\nB: `{}` (`{}`), {} source raw.\n",
        b.change_spec_id,b.world_id,b.world_kind,b.candidate.sha256,b.candidate.len,b.clock.slot,b.clock.unix_timestamp,b.runtime_id,b.reserve,b.initial_reserve_raw,
        b.units[0].unit_id,b.units[0].source_account,b.units[0].amount_raw,b.units[1].unit_id,b.units[1].source_account,b.units[1].amount_raw);
    for (label, scenario) in ["A alone", "B alone", "A → B", "B → A"]
        .iter()
        .zip(&a.scenarios)
    {
        s.push_str(&format!(
            "\n## {label}\n\nCase `{}`; run `{}`.\n\nInitial state `{}`.\n",
            scenario.case_id, scenario.run_id, scenario.initial_state_id
        ));
        for (i, step) in scenario.steps.iter().enumerate() {
            s.push_str(&format!("\nStep {} — `{}`: {:?}. Reserve {} → {} raw; destination credit {} raw; source debit {} raw.\n\nState `{}` → `{}`.\n\nReconciliation: {}. Failure: {}.\n",i+1,step.execution.unit_id,step.execution.outcome,step.observed_reserve_before_raw,step.observed_reserve_after_raw,step.execution.deltas.destination_net_credit_raw,step.execution.deltas.source_debit_raw,step.before_state_id,step.after_state_id,
                if step.execution.reconciled { "exact" } else if step.execution.failure.as_ref().is_some_and(|f|f.rollback_verified) { "rejection rollback verified (fee retained)" } else { "mismatch" },
                step.execution.failure.as_ref().map(|f|format!("{} / {}",f.stage,f.error_name.as_deref().unwrap_or(&f.error))).unwrap_or_else(||"none".into())));
        }
        s.push_str(&format!(
            "\nFinal state `{}`. Stopped: {:?}.\n",
            scenario.final_state_id, scenario.stopped
        ));
    }
    s.push_str(&format!("\n## Comparison\n\n{:?}\n", a.comparison.status));
    if let Some(finding) = &a.comparison.finding {
        s.push_str(&format!("\n`{finding}`\n\nBoth units succeeded alone. Under this pinned world and transaction ordering, reversing the order changed which unit succeeded; the second transaction rejected after the shared reserve was reduced.\n"));
    }
    for difference in &a.comparison.affected_units {
        s.push_str(&format!(
            "\n`{}`: A → B {:?}, B → A {:?}.\n",
            difference.unit_id, difference.a_then_b.outcome, difference.b_then_a.outcome
        ));
    }
    for limitation in &a.comparison.limitations {
        s.push_str(&format!("\n- {limitation}\n"));
    }
    s
}
