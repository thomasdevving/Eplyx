//! Fixture-corpus comparison, generation, reproduction and listing commands.
use super::{CompareArgs, Format, GenerateArgs, ListArgs, ReproduceArgs};
use anyhow::{anyhow, Context, Result};
use eplyx_engine::{
    compare_all, corpus, default_artifact, fixture_program_id, load_versions, report::render_text,
    report::Report, types::Category,
};
use std::process::ExitCode;

fn parse_category(name: &str) -> Result<Category> {
    Category::ALL
        .into_iter()
        .find(|c| c.as_str() == name)
        .ok_or_else(|| {
            anyhow!(
                "unknown category {name:?}; valid categories: {}",
                Category::ALL
                    .iter()
                    .map(|c| c.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

pub(crate) fn compare(args: CompareArgs) -> Result<ExitCode> {
    if let Some(path) = &args.corpus {
        anyhow::ensure!(
            args.fixture.is_none() && args.category.is_none(),
            "select replay records with corpus build --limit"
        );
        let records = eplyx_engine::replay::load_corpus(path)?;
        let (v1, v2) = load_versions(
            &args.v1.clone().unwrap_or_else(|| default_artifact("v1")),
            &args.v2.clone().unwrap_or_else(|| default_artifact("v2")),
        )?;
        let dependency_dir = args
            .dependencies
            .clone()
            .unwrap_or_else(|| eplyx_engine::replay::dependency_directory(path));
        let load_start = std::time::Instant::now();
        let dependencies = eplyx_engine::replay::load_dependencies(&records, &dependency_dir)?;
        let dependency_load = load_start.elapsed().as_micros();
        let start = std::time::Instant::now();
        let report =
            eplyx_engine::replay::compare_with_dependencies(&records, &v1, &v2, &dependencies)?;
        let elapsed = start.elapsed().as_micros();
        let rendered = match args.format {
            Format::Json => serde_json::to_string_pretty(&report)?,
            Format::Text => {
                let mut text=String::from("OFFLINE TRUSTED REPLAY\nCapital represents interaction observations, not unique TVL. Native transfers are reported separately without fiat estimates.\n");
                for item in &report.observations {
                    text.push_str(&format!("{} | slot {} | {:?} | fidelity {:?}\n  source {}\n  pre {}\n  V1  {} (matches original)\n  V2  {}\n",item.id,item.source_slot,item.state_source,item.fidelity,item.source_signature,item.pre_state_hash,item.post_v1_state_hash,item.post_v2_state_hash));
                }
                if let Some(native) = &report.native_impact {
                    text.push_str(&format!("Native transfer represented: {} lamports\nCandidate prevented:          {} lamports\n",native.v1_transferred_lamports,native.candidate_prevented_lamports));
                }
                for item in &report.observations {
                    if item.dependency_programs.is_empty() {
                        continue;
                    }
                    text.push_str(&format!("\nPROGRAM DEPENDENCIES ({})\n", item.id));
                    for program in &item.dependency_programs {
                        text.push_str(&format!(
                            "  {:44} {:18} {}\n",
                            program.program_id,
                            program.source.as_str(),
                            match (&program.binary_sha256, program.deployed_slot) {
                                (Some(hash), Some(slot)) =>
                                    format!("deployed at slot {slot}, sha256 {hash}"),
                                (Some(hash), None) => format!("sha256 {hash}"),
                                _ => "provided by the runtime".into(),
                            }
                        ));
                    }
                }
                for item in &report.observations {
                    if item.original_cpi_graph.is_empty() && item.cpi_graph_v1.is_empty() {
                        continue;
                    }
                    text.push_str(&format!("\nCPI GRAPH ({})\n", item.id));
                    text.push_str("  mainnet, from validator metadata:\n");
                    for line in eplyx_engine::replay::render_cpi_graph(
                        &report.analysis.program_id,
                        &item.original_cpi_graph,
                    )
                    .lines()
                    {
                        text.push_str(&format!("    {line}\n"));
                    }
                    text.push_str(&format!(
                        "  V1 replay reproduced it: {}\n  V2 invocation graph: {}\n",
                        if item.cpi_graph_v1 == item.original_cpi_graph {
                            "yes"
                        } else {
                            "no"
                        },
                        if item.cpi_graph_changed {
                            "differs from V1; a changed invocation graph is reported, \
                             not scored"
                        } else {
                            "identical to V1"
                        }
                    ));
                }
                let summarized: Vec<_> = report
                    .observations
                    .iter()
                    .filter(|item| !item.economic_summary.is_empty())
                    .collect();
                if !summarized.is_empty() {
                    text.push_str("\nPROTOCOL RESULT\n");
                    for item in summarized {
                        text.push_str(&format!("  {}\n", item.id));
                        for row in &item.economic_summary {
                            text.push_str(&format!(
                                "    {:26} V1 {:>22}   V2 {:>22}{}\n",
                                row.field,
                                row.v1,
                                row.v2,
                                row.delta
                                    .filter(|delta| !delta.is_zero())
                                    .map(|delta| format!("   delta {delta}"))
                                    .unwrap_or_default()
                            ));
                        }
                    }
                }
                let economic: Vec<_> = report
                    .observations
                    .iter()
                    .flat_map(|item| {
                        item.economic_changes
                            .iter()
                            .map(move |change| (item.id.as_str(), change))
                    })
                    .collect();
                text.push_str("\nPROTOCOL ECONOMICS\n");
                if economic.is_empty() {
                    text.push_str("  No protocol-level field differs between the two builds.\n");
                } else {
                    text.push_str(&format!(
                        "  {} of {} observation(s) changed economically. Whether a change is \
                         intended is not classified here.\n",
                        report.economic_findings,
                        report.observations.len()
                    ));
                    for (id, change) in economic {
                        text.push_str(&format!(
                            "  {id}\n    {} ({}) {}\n      V1 {}\n      V2 {}{}\n",
                            change.account_label,
                            change.account_kind,
                            change.field,
                            change.v1,
                            change.v2,
                            change
                                .delta
                                .map(|delta| format!("\n      delta {delta}"))
                                .unwrap_or_default(),
                        ));
                    }
                }
                let analysis = render_text(&report.analysis).replace(
                    "synthetic corpus, valued from fixture state",
                    "replay observations, valued from captured state",
                );
                // The fixture protocol's position/USD aggregation is meaningless
                // for an adapter-owned record: there are no positions to value,
                // and printing zeroed collateral would read as a measurement.
                text.push_str(
                    &if records.iter().any(|record| record.adapter().is_some()) {
                        analysis
                        .split("ECONOMIC COVERAGE")
                        .next()
                        .unwrap_or(&analysis)
                        .to_string()
                        + "(Position and USD aggregation belongs to the fixture lending protocol \
                           and is omitted: this record's economics are protocol token amounts, \
                           reported above.)\n\n"
                        + analysis
                            .split_once("VERDICT:")
                            .map(|(_, rest)| format!("VERDICT:{rest}"))
                            .unwrap_or_default()
                            .as_str()
                    } else {
                        analysis
                    },
                );
                text
            }
        };
        if let Some(out) = &args.out {
            std::fs::write(out, rendered)?;
        } else {
            println!("{rendered}");
        }
        let total_micros = |select: fn(&eplyx_engine::replay::ReplayTiming) -> u128| {
            report.timings.iter().map(select).sum::<u128>()
        };
        eprintln!(
            "Replay performance: dependency load {dependency_load} us for {} binary(ies); \
             V1 replay {} us; V2 replay {} us; comparison total {elapsed} us; \
             {} VM executions; {} accounts loaded; {} program dependencies \
             ({} loaded from history, {} runtime-provided)",
            dependencies.programs().len(),
            total_micros(|timing| timing.v1_micros),
            total_micros(|timing| timing.v2_micros),
            records.len() * 2,
            records
                .iter()
                .map(|record| record.accounts.len())
                .sum::<usize>(),
            records
                .iter()
                .map(|record| record.dependencies.programs.len())
                .sum::<usize>(),
            records
                .iter()
                .flat_map(|record| record.dependencies.loadable())
                .count(),
            records
                .iter()
                .flat_map(|record| record.dependencies.programs.iter())
                .filter(
                    |program| program.source == eplyx_engine::dependencies::ProgramSource::Builtin
                )
                .count(),
        );
        // A protocol-level economic change is a gate failure even when the
        // protocol-agnostic classifier only saw bytes move.
        return Ok(
            if args.fail_on_critical
                && (report.analysis.summary.critical > 0 || report.economic_findings > 0)
            {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            },
        );
    }
    let program_id = fixture_program_id();
    let mut fixtures = corpus::generate(&program_id);

    if let Some(id) = &args.fixture {
        fixtures.retain(|f| &f.id == id);
        if fixtures.is_empty() {
            return Err(anyhow!("no fixture with id {id:?}"));
        }
    }
    if let Some(name) = &args.category {
        let category = parse_category(name)?;
        fixtures.retain(|f| f.category == category);
    }

    let v1_path = args.v1.unwrap_or_else(|| default_artifact("v1"));
    let v2_path = args.v2.unwrap_or_else(|| default_artifact("v2"));
    let (v1, v2) = load_versions(&v1_path, &v2_path)?;

    let diffs = compare_all(&fixtures, &program_id, &v1, &v2)?;
    let mut report = Report::new(
        program_id.to_string(),
        v1_path.display().to_string(),
        v2_path.display().to_string(),
        &fixtures,
        diffs,
    );
    if !args.no_minimize {
        eplyx_engine::minimize_clusters(
            &mut report,
            &fixtures,
            &program_id,
            &v1,
            &v2,
            eplyx_engine::shrink::ShrinkConfig::default(),
        )?;
    }

    let rendered = match args.format {
        Format::Text => render_text(&report),
        Format::Json => report.to_json()?,
    };

    match &args.out {
        Some(path) => {
            std::fs::write(path, &rendered)
                .with_context(|| format!("writing report to {}", path.display()))?;
            eprintln!("wrote {}", path.display());
        }
        None => println!("{rendered}"),
    }

    if args.fail_on_critical && report.summary.critical > 0 {
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn generate(args: GenerateArgs) -> Result<ExitCode> {
    let program_id = fixture_program_id();
    let fixtures = corpus::generate(&program_id);
    let out = args
        .out
        .unwrap_or_else(|| eplyx_engine::repo_root().join("fixtures/states"));
    std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;

    for fixture in &fixtures {
        let path = out.join(format!("{}.json", fixture.id));
        std::fs::write(&path, serde_json::to_string_pretty(fixture)? + "\n")
            .with_context(|| format!("writing {}", path.display()))?;
    }

    let index: Vec<_> = fixtures
        .iter()
        .map(|f| {
            serde_json::json!({
                "id": f.id,
                "category": f.category.as_str(),
                "scenario": f.scenario,
            })
        })
        .collect();
    let index_path = out.join("index.json");
    std::fs::write(&index_path, serde_json::to_string_pretty(&index)? + "\n")?;

    println!("wrote {} fixtures to {}", fixtures.len(), out.display());
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn reproduce(args: ReproduceArgs) -> Result<ExitCode> {
    let program_id = fixture_program_id();
    let fixtures = corpus::generate(&program_id);
    let v1_path = args.v1.unwrap_or_else(|| default_artifact("v1"));
    let v2_path = args.v2.unwrap_or_else(|| default_artifact("v2"));
    let (v1, v2) = load_versions(&v1_path, &v2_path)?;

    let target = args
        .target
        .strip_prefix("regression-group:")
        .unwrap_or(&args.target)
        .to_string();

    // A single fixture needs two executions; a regression group needs the whole
    // corpus, so try the cheap resolution first.
    if let Some(fixture) = fixtures.iter().find(|f| f.id == target) {
        let diff = eplyx_engine::compare_fixture(fixture, &program_id, &v1, &v2)?;
        println!("{}", eplyx_engine::report::render_reproduction(&diff));
        return Ok(ExitCode::SUCCESS);
    }

    let diffs = compare_all(&fixtures, &program_id, &v1, &v2)?;
    let mut report = Report::new(
        program_id.to_string(),
        v1_path.display().to_string(),
        v2_path.display().to_string(),
        &fixtures,
        diffs,
    );
    if !args.no_minimize {
        eplyx_engine::minimize_clusters(
            &mut report,
            &fixtures,
            &program_id,
            &v1,
            &v2,
            eplyx_engine::shrink::ShrinkConfig::default(),
        )?;
    }

    match report.cluster(&target) {
        Some(cluster) => {
            println!(
                "{}",
                eplyx_engine::report::render_cluster_reproduction(&report, cluster)
            );
            Ok(ExitCode::SUCCESS)
        }
        None => Err(anyhow!(
            "no fixture or regression group {target:?}\n\navailable regression groups:\n{}",
            report
                .clusters
                .iter()
                .map(|c| format!(
                    "  {:<34} {} fixtures{}",
                    c.id,
                    c.fixture_count(),
                    if c.critical { "  [CRITICAL]" } else { "" }
                ))
                .collect::<Vec<_>>()
                .join("\n")
        )),
    }
}

pub(crate) fn list(args: ListArgs) -> Result<ExitCode> {
    let program_id = fixture_program_id();
    let mut fixtures = corpus::generate(&program_id);
    if let Some(name) = &args.category {
        let category = parse_category(name)?;
        fixtures.retain(|f| f.category == category);
    }
    for fixture in &fixtures {
        println!(
            "{:<28} {:<22} {}",
            fixture.id,
            fixture.category.as_str(),
            fixture.scenario
        );
    }
    println!("\n{} fixtures", fixtures.len());
    Ok(ExitCode::SUCCESS)
}
