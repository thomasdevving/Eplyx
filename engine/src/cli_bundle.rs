//! CI bundle assembly and verification, and corpus store commands.
use super::{BundleBuildArgs, BundleVerifyArgs, CorpusSelectArgs, Format};
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::ExitCode;

/// Assemble an offline-executable CI bundle.
pub(crate) fn bundle_build(args: BundleBuildArgs) -> Result<ExitCode> {
    use eplyx_engine::bundle;

    let corpus_manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(args.corpus.join("manifest.json"))?)?;
    if corpus_manifest["schema_version"] == 2 {
        anyhow::ensure!(
            args.target_size.is_none(),
            "schema-2 bundle selection is not implemented; bundle the complete reviewed corpus"
        );
        let built =
            eplyx_engine::universal::bundle::build(&args.corpus, &args.baseline, &args.out)?;
        println!("EPLYX CI BUNDLE\n===============\n\nprogram: {}\nrecords: {}\nbundle sha256: {}\n\nwrote {}",
            built.manifest.program_id, built.manifest.record_count, built.manifest.bundle_sha256, args.out.display());
        return Ok(ExitCode::SUCCESS);
    }

    let store = eplyx_engine::corpus_store::CorpusStore::open(&args.corpus)?;
    let all = store.load()?;
    let dependencies = args
        .dependencies
        .clone()
        .unwrap_or_else(|| args.corpus.join("dependencies"));

    // Selecting first keeps the bundle small enough to run on every pull
    // request; the policy and its stated limitations travel with it, so the
    // gate can report what the corpus does not cover.
    let (records, policy, policy_version, limitations) = match args.target_size {
        Some(target) => {
            let observed: eplyx_engine::select::ObservedCounts = match &args.observed {
                Some(text) => {
                    serde_json::from_str(text).context("--observed must be a JSON object")?
                }
                None => Default::default(),
            };
            let selected = eplyx_engine::select::select(&all, target, &observed)?;
            let keep: std::collections::BTreeSet<&str> =
                selected.selected.iter().map(|s| s.id.as_str()).collect();
            let records: Vec<_> = all
                .iter()
                .filter(|r| keep.contains(r.id.as_str()))
                .cloned()
                .collect();
            let limitations = selected
                .limitations
                .iter()
                .map(|l| bundle::BundledLimitation {
                    code: l.code.clone(),
                    detail: l.detail.clone(),
                })
                .collect();
            (
                records,
                Some(selected.selection_policy),
                Some(selected.selection_policy_version),
                limitations,
            )
        }
        None => (all, None, None, Vec::new()),
    };

    let built = bundle::build(
        bundle::BundleInputs {
            records: &records,
            baseline: &args.baseline,
            dependencies: &dependencies,
            selection_policy: policy,
            selection_policy_version: policy_version,
            limitations,
            // A corpus is acquired until something reproduces it. This is the
            // first step holding the baseline and every dependency, so it is
            // the first step that can make "validated" true.
            validation: bundle::Validation::AgainstBaseline,
        },
        &args.out,
    )?;
    print_bundle(&built);
    println!("\nwrote {}", args.out.display());
    Ok(ExitCode::SUCCESS)
}

/// Re-hash every byte a bundle pins.
pub(crate) fn bundle_verify(args: BundleVerifyArgs) -> Result<ExitCode> {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(args.bundle.join("bundle.json"))?)?;
    if manifest["schema_version"] == 2 {
        let bundle = eplyx_engine::universal::bundle::UniversalBundle::open(&args.bundle)?;
        match args.format {
            Format::Json => println!("{}", serde_json::to_string_pretty(&bundle.manifest)?),
            Format::Text => println!("EPLYX CI BUNDLE\n===============\n\nprogram: {}\nrecords: {}\nbundle sha256: {}\n\nEvery hash and historical reference verified against the bytes on disk.",
                bundle.manifest.program_id, bundle.manifest.record_count, bundle.manifest.bundle_sha256),
        }
        return Ok(ExitCode::SUCCESS);
    }
    let bundle = eplyx_engine::bundle::CiBundle::open(&args.bundle)?;
    match args.format {
        Format::Json => println!("{}", serde_json::to_string_pretty(bundle.manifest())?),
        Format::Text => {
            print_bundle(&bundle);
            println!("\nEvery hash verified against the bytes on disk.");
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn print_bundle(bundle: &eplyx_engine::bundle::CiBundle) {
    let manifest = bundle.manifest();
    let adapter = bundle.adapter();
    println!("EPLYX CI BUNDLE");
    println!("===============\n");
    println!("program:           {}", manifest.program_id);
    println!(
        "adapter:           {}@{}{}",
        adapter.name,
        adapter.version,
        if adapter.supports_cpi {
            ", cross-program invocation"
        } else {
            ""
        }
    );
    if let (Some(policy), Some(version)) = (
        &manifest.selection_policy,
        manifest.selection_policy_version,
    ) {
        println!("selection policy:  {policy} v{version}");
    }
    println!("\nbaseline sha256:   {}", manifest.baseline_program_sha256);
    println!("corpus sha256:     {}", manifest.corpus_sha256);
    println!("bundle sha256:     {}", manifest.bundle_sha256);
    println!(
        "\nrecords:           {} validated historical observations",
        manifest.record_count
    );
    println!(
        "production window: slots {} -> {}",
        manifest.source_slot_range.first, manifest.source_slot_range.last
    );
    println!("\nCOVERAGE");
    for action in &adapter.actions {
        println!(
            "  {:<16} {:>3} tested",
            action.semantic_action, action.observations
        );
    }
    if !manifest.dependencies.is_empty() {
        println!("\nPINNED DEPENDENCIES");
        for dependency in &manifest.dependencies {
            println!(
                "  {}  {}  {} bytes",
                dependency.program_id,
                &dependency.sha256[..16],
                dependency.len
            );
        }
    }
    if !adapter.limitations.is_empty() {
        println!("\nKNOWN LIMITATIONS");
        for limitation in &adapter.limitations {
            println!("  - {}", limitation.code);
            println!("    {}", limitation.detail);
        }
    }
}

/// Select a deterministic regression corpus from validated replay records.
pub(crate) fn corpus_select(args: CorpusSelectArgs) -> Result<ExitCode> {
    let store = eplyx_engine::corpus_store::CorpusStore::open(&args.corpus)?;
    let records = store.load()?;
    anyhow::ensure!(
        !records.is_empty(),
        "no validated records in {}",
        args.corpus.display()
    );
    let observed: eplyx_engine::select::ObservedCounts = match &args.observed {
        Some(text) => serde_json::from_str(text).context("--observed must be a JSON object")?,
        None => Default::default(),
    };
    let corpus = eplyx_engine::select::select(&records, args.target_size, &observed)?;
    let rendered = match args.format {
        Format::Text => eplyx_engine::select::render(&corpus),
        Format::Json => serde_json::to_string_pretty(&corpus)?,
    };
    match &args.out {
        Some(path) => {
            std::fs::write(path, &rendered)
                .with_context(|| format!("writing {}", path.display()))?;
            eprintln!("wrote {}", path.display());
        }
        None => println!("{rendered}"),
    }
    Ok(ExitCode::SUCCESS)
}

/// Publish the deterministic index for an existing immutable record store.
pub(crate) fn corpus_publish(corpus: PathBuf) -> Result<ExitCode> {
    let store = eplyx_engine::corpus_store::CorpusStore::open(&corpus)?;
    let records = store.load_v2()?;
    for record in &records {
        store.insert_v2(record)?;
    }
    let manifest = store.publish_v2()?;
    println!(
        "published {} schema-2 observations: {}",
        manifest.record_count, manifest.canonical_hash
    );
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn corpus_build(
    cache: PathBuf,
    snapshots: PathBuf,
    out: PathBuf,
    limit: Option<usize>,
) -> Result<ExitCode> {
    let start = std::time::Instant::now();
    let count = eplyx_engine::ingest::build_corpus(&cache, &snapshots, &out, limit)?;
    println!(
        "Built {count} replay records in {} ms: {}",
        start.elapsed().as_millis(),
        out.display()
    );
    Ok(ExitCode::SUCCESS)
}
