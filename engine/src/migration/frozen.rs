//! Fixed-ratio conversion packages as TokenMigrationV1 examples.
//!
//! The hackathon candidate conversion (`fixed_ratio_conversion_v1`) is a special
//! case of a token migration: burn the source, pay a fixed raw ratio from a proposed
//! reserve held by a program-derived authority, holder wallets signing. This module
//! maps such a package to its equivalent TokenMigrationV1 specification and rebuilds
//! an observed rehearsal world from a saved run's exact frozen captures, so the demo
//! stays reproducible as a generic example. The original package, its candidate and
//! its saved report are untouched and still replay byte for byte; the mapping never
//! claims the demo is an official issuer migration.
use super::{
    capture::Collector,
    spec::{
        AmountPolicy, Authorities, AuthorityExpectations, Conversion, DestinationFunding,
        Eligibility, Fee, FeePayer, HolderAuthorization, MigrationAuthority, OwnerAuthorityClass,
        RatioBasis, Reserve, Rounding, SourceDisposition, TokenMigrationV1, TokenSide, Window,
        WindowBoundary,
    },
    world::{PopulationIndex, World, WorldAccount, WorldClock, WorldKind, MAINNET_GENESIS},
};
use crate::{
    ingest::observation::Observation,
    migration::{population, population_types::StressBudget},
    standard_programs::token::CLOCK,
};
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use std::{collections::BTreeMap, path::Path};

/// Read-only projection of an archived fixed-ratio manifest, not a runnable product.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub source_mint: String,
    pub replacement_mint: String,
    pub terms: Terms,
    pub effective_at: chrono::DateTime<chrono::Utc>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Terms {
    pub numerator: String,
    pub denominator: String,
    pub rounding: String,
    pub fee_bps: u16,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub reserve_funded_replacement_raw: String,
}

/// The TokenMigrationV1 equivalent of a fixed-ratio conversion package. Token
/// programs and decimals are read from the captured mints.
pub fn spec_from_fixed_ratio(
    manifest: &Manifest,
    config: &Config,
    world: &World,
) -> Result<TokenMigrationV1> {
    let source = world.mint(&manifest.source_mint)?;
    let destination = world.mint(&manifest.replacement_mint)?;
    let spec = TokenMigrationV1 {
        version: super::spec::VERSION,
        source: TokenSide {
            mint: manifest.source_mint.clone(),
            token_program: source.token_program,
            decimals: source.decimals,
        },
        destination: TokenSide {
            mint: manifest.replacement_mint.clone(),
            token_program: destination.token_program,
            decimals: destination.decimals,
        },
        conversion: Conversion {
            ratio_basis: RatioBasis::Raw,
            numerator: manifest.terms.numerator.clone(),
            denominator: manifest.terms.denominator.clone(),
            rounding: match manifest.terms.rounding.as_str() {
                "ceiling" => Rounding::Ceiling,
                _ => Rounding::Floor,
            },
            fee: if manifest.terms.fee_bps == 0 {
                Fee::None
            } else {
                Fee::SourceBps {
                    bps: manifest.terms.fee_bps,
                }
            },
            minimum_output_raw: "1".into(),
        },
        window: Window {
            activation: Some(WindowBoundary::UnixTimestamp {
                value: manifest.effective_at.timestamp().max(0).to_string(),
            }),
            deadline: None,
        },
        eligibility: Eligibility {
            amount_policy: AmountPolicy::FullBalance,
            minimum_source_balance_raw: "1".into(),
            holder_authorization: vec![HolderAuthorization::Owner],
            owner_authority_classes: vec![OwnerAuthorityClass::Wallet],
            excluded_accounts: vec![],
        },
        source_disposition: SourceDisposition::Burn,
        destination_funding: DestinationFunding::ReserveTransfer {
            reserve: Reserve::Proposed {
                funded_raw: config.reserve_funded_replacement_raw.clone(),
            },
        },
        authorities: Authorities {
            migration_authority: MigrationAuthority::ProgramDerived,
            expected: AuthorityExpectations::default(),
            fee_payer: FeePayer::Relayer,
        },
    };
    spec.validate()?;
    Ok(spec)
}

#[derive(Deserialize)]
struct Bindings {
    budget: StressBudget,
}

#[derive(Deserialize)]
struct ConversionCapture {
    observations: Vec<Observation>,
}

#[derive(Deserialize)]
struct CaseCapture {
    observations: Vec<Observation>,
}

#[derive(Deserialize)]
struct CaseBundle {
    cases: Vec<CaseCapture>,
}

/// Rebuild the observed world of a saved fixed-ratio package run, offline. Each
/// account keeps the artifact name (`{label}/<file>`) and JSON pointer it came from.
pub fn world_from_package_run(result: &Path, label: &str, source_mint: &str) -> Result<World> {
    let read =
        |name: &str| std::fs::read(result.join(name)).with_context(|| format!("missing {name}"));
    let bindings: Bindings = serde_json::from_slice(&read("bindings.json")?)?;
    let population_bytes = read("population.capture.json")?;
    let observation = population::evaluate_bytes(&population_bytes, &bindings.budget)?;
    ensure!(
        observation.mint == source_mint,
        "the saved run is for a different source mint"
    );
    let capture: population::Capture = serde_json::from_slice(&population_bytes)?;
    let artifact = |file: &str| format!("{label}/{file}");
    let mut c = Collector::new();
    for (record, o) in capture.observations.iter().enumerate() {
        let Some(result) = &o.result else { continue };
        let slot = || result["context"]["slot"].as_u64().context("context slot");
        match o.method.as_str() {
            "getAccountInfo" => {
                let address = o.params[0].as_str().context("address")?;
                c.observe(
                    address,
                    &result["value"],
                    &artifact("population.capture.json"),
                    record,
                    "/result/value".into(),
                    slot()?,
                )?;
            }
            "getProgramAccounts" => {
                let slot = slot()?;
                for (index, row) in result["value"].as_array().into_iter().flatten().enumerate() {
                    let address = row["pubkey"].as_str().context("row pubkey")?;
                    c.observe(
                        address,
                        &row["account"],
                        &artifact("population.capture.json"),
                        record,
                        format!("/result/value/{index}/account"),
                        slot,
                    )?;
                }
            }
            _ => c.batch(o, &artifact("population.capture.json"), record)?,
        }
    }
    let conversion: ConversionCapture = serde_json::from_slice(&read("conversion.capture.json")?)?;
    let absorb = |c: &mut Collector,
                  observations: &[Observation],
                  file: &str,
                  prefix: &dyn Fn(usize) -> String|
     -> Result<()> {
        for (record, o) in observations.iter().enumerate() {
            let Some(result) = &o.result else { continue };
            match o.method.as_str() {
                "getAccountInfo" => {
                    let address = o.params[0].as_str().context("address")?;
                    let slot = result["context"]["slot"].as_u64().context("slot")?;
                    c.observe(
                        address,
                        &result["value"],
                        file,
                        record,
                        format!("{}/result/value", prefix(record)),
                        slot,
                    )?;
                }
                "getMultipleAccounts" => c.batch_at(o, file, record, &prefix(record))?,
                _ => {}
            }
        }
        Ok(())
    };
    absorb(
        &mut c,
        &conversion.observations,
        &artifact("conversion.capture.json"),
        &|record| format!("/observations/{record}"),
    )?;
    let bundle: CaseBundle = serde_json::from_slice(&read("stress.cases.json")?)?;
    for (case, capture) in bundle.cases.iter().enumerate() {
        absorb(
            &mut c,
            &capture.observations,
            &artifact("stress.cases.json"),
            &|record| format!("/cases/{case}/observations/{record}"),
        )?;
    }
    let (clock_account, _) = c
        .accounts
        .get(CLOCK)
        .context("the saved run has no Clock")?;
    let clock = WorldClock::from_bytes(&clock_account.account.data)?;
    let mut token_accounts: Vec<String> = observation
        .entities
        .iter()
        .map(|e| e.token_account.clone())
        .collect();
    token_accounts.sort();
    let min = c.slots.iter().min().copied().unwrap_or(clock.slot);
    let max = c.slots.iter().max().copied().unwrap_or(clock.slot);
    let accounts: BTreeMap<String, WorldAccount> =
        c.accounts.into_iter().map(|(k, (a, _))| (k, a)).collect();
    let world = World {
        kind: WorldKind::ObservedCapture,
        cluster: "solana-mainnet".into(),
        genesis_hash: MAINNET_GENESIS.into(),
        clock,
        observed_slots: Some((min, max)),
        inspected_absent: c.absent.into_iter().filter(|a| !accounts.contains_key(a)).collect(),
        accounts,
        population: PopulationIndex {
            source_mint: source_mint.into(),
            token_accounts,
            enumeration_completeness: observation.enumeration.completeness.key().into(),
            authority_resolution_completeness: observation.authority_resolution.completeness.key().into(),
            undecoded_accounts: observation.undecoded.iter().map(|u| u.address.clone()).collect(),
        },
        limitations: vec![
            format!("Rebuilt from the frozen captures of saved run {label}: a composite of finalized read-only observations between slots {min} and {max}, not a historical validator bank."),
            "Only destination accounts the original run happened to inspect are known; every other holder's destination is not inspected.".into(),
        ],
        derived_from: None,
    };
    world.validate()?;
    Ok(world)
}

/// The saved run's manifest and config, for the spec mapping.
pub fn package_parts(package: &Path) -> Result<(Manifest, Config)> {
    let manifest: Manifest = serde_json::from_slice(&std::fs::read(package.join("eplyx.json"))?)?;
    let config: Config = serde_json::from_slice(&std::fs::read(package.join("config.json"))?)?;
    Ok((manifest, config))
}
