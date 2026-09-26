//! Export read-only view models for JavaScript assertions, without a listener.
use anyhow::Result;
use eplyx_engine::dashboard::{store::Store, view};
use serde_json::{json, Map, Value};
use std::{fs, io::Write, path::PathBuf};
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().expect("fixture directory"));
    let output = PathBuf::from(args.next().expect("new output file"));
    let mut projects = Map::new();
    for name in [
        "transition-acceptance",
        "token-migration",
        "second-asset",
        "analytical-kinds",
    ] {
        let store = Store::open(&root.join(name))?;
        let ids = store.run_ids()?.0;
        let reproductions: Vec<_> = store
            .reproduction_ids()?
            .0
            .iter()
            .rev()
            .map(|id| view::reproduction_summary(&store, id))
            .collect();
        let (runs, files) = view::assemble(
            ids.iter().map(|id| view::run_summary(&store, id)).collect(),
            store
                .counterexample_ids()?
                .0
                .iter()
                .map(|id| view::counterexample_summary(&store, id))
                .collect(),
            &reproductions,
        );
        let mut responses = Map::new();
        let project = view::project_payload(
            json!({"name":name}),
            json!({"version":"fixture","config":{"state":"Missing"}}),
            json!({"path":".eplyx/"}),
            &runs,
            &files,
            &reproductions,
            0,
        );
        responses.insert("/api/project".into(), project);
        responses.insert("/api/runs".into(), json!({"runs":runs}));
        responses.insert(
            "/api/counterexamples".into(),
            json!({"counterexamples":files}),
        );
        for row in &runs {
            let id = row["id"].as_str().unwrap();
            let mut detail = view::run_detail(&store, id, &files)?;
            detail["number"] = row["number"].clone();
            let position = runs.iter().position(|r| r["id"] == id).unwrap();
            detail["previous_run"] = runs
                .get(position + 1)
                .map_or(Value::Null, |r| r["id"].clone());
            responses.insert(format!("/api/runs/{id}"), detail);
            for right in &runs {
                let r = right["id"].as_str().unwrap();
                if let Ok(mut c) = view::compare(&store, id, r) {
                    c["left"]["number"] = row["number"].clone();
                    c["right"]["number"] = right["number"].clone();
                    responses.insert(format!("/api/compare?left={id}&right={r}"), c);
                }
            }
        }
        for row in &files {
            let id = row["id"].as_str().unwrap();
            let mut detail = view::counterexample_detail(&store, id, row)?;
            detail["parent_summary"] = runs
                .iter()
                .find(|r| r["id"] == row["parent_run"])
                .cloned()
                .unwrap_or(Value::Null);
            responses.insert(format!("/api/counterexamples/{id}"), detail);
        }
        projects.insert(name.into(), Value::Object(responses));
    }
    let bytes = serde_json::to_vec_pretty(
        &json!({"schema_version":1,"purpose":"Derived presentation models for frontend assertions; not analytical evidence. Cosmetic context is synthetic; report fields are projected by dashboard::view.","projects":projects}),
    )?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    file.write_all(&bytes)?;
    println!("Exported {} bytes of presentation models.", bytes.len());
    Ok(())
}
