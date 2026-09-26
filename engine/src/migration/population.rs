//! Fresh current source-token account enumeration for one selected mint.
//!
//! This is a new bounded read-only acquisition. It never reads, imports or falls
//! back to a historical Phase 2/6/7 population: those files are a different type
//! and cannot enter this path. If the configured provider cannot serve a complete
//! enumeration, the result says so and the trustworthy subset is kept, rather
//! than substituting saved account inventory.
//!
//! Two completeness facts are reported independently:
//!
//! * [`EnumerationCompleteness`] — did the token-account scan itself return and
//!   verify everything the query asked for?
//! * [`AuthorityResolutionCompleteness`] — was every recorded authority of the
//!   positive-balance population actually inspected?
//!
//! Reaching the authority budget never downgrades the enumeration axis, because
//! "we do not know how many token accounts exist" and "we have every token
//! account but not every authority model" are different findings.
use super::population_types::{
    entity_id, AuthorityResolution, AuthorityResolutionCompleteness, EnumerationCompleteness,
    StressBudget, StressEntity, UndecodedAccount,
};
use crate::{
    evidence::authority::{classify_authority, AuthorityObservation, EntityType, EvidenceRef},
    ingest::{observation::Observation, rpc::RpcProvider as SolanaRpc},
    replay::hash_bytes as sha256,
    standard_programs::token::{self as decode, MintConfig},
};
use anyhow::{ensure, Context, Result};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use solana_address::Address;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::Path;

const MAINNET: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";
pub const DECODER: &str = "spl-token-2022-interface/3.1.1; conversion-stress-population/v1";
pub const DECODER_V2: &str = "spl-token-2022-interface/3.1.1; conversion-stress-population/v2";
pub const KIND: &str = "conversion-stress-population";

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
fn base_config() -> Value {
    json!({"encoding":"base64","commitment":"finalized"})
}
fn min_config(slot: u64) -> Value {
    json!({"encoding":"base64","commitment":"finalized","minContextSlot":slot})
}
/// No `dataSize` filter: Token-2022 accounts have extension-dependent sizes, and
/// a fixed size would silently exclude every extension-bearing account.
fn enumeration_config(mint: &str, slot: u64) -> Value {
    json!({"encoding":"base64","commitment":"finalized","minContextSlot":slot,
        "withContext":true,"filters":[{"memcmp":{"offset":0,"bytes":mint}}]})
}
fn context_slot(v: &Value) -> Result<u64> {
    v["context"]["slot"]
        .as_u64()
        .context("missing RPC response context slot")
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    pub schema_version: u32,
    pub kind: String,
    pub run_id: String,
    pub stress_id: String,
    pub mint: String,
    pub budget: StressBudget,
    pub rpc_origin: String,
    pub started_at: String,
    pub completed_at: String,
    pub decoder: String,
    pub observations: Vec<Observation>,
}

fn record(
    rpc: &impl SolanaRpc,
    records: &mut Vec<Observation>,
    method: &str,
    params: Value,
) -> Option<Value> {
    let started_at = now();
    let (result, error) = match rpc.call(method, params.clone()) {
        Ok(v) => (Some(v), None),
        Err(e) => (None, Some(e.to_string())),
    };
    records.push(Observation {
        method: method.into(),
        params,
        started_at,
        completed_at: now(),
        result: result.clone(),
        error,
    });
    result
}

/// Distinct recorded authorities of the positive-balance rows, ascending, capped
/// by the declared budget. Deterministic, so the transcript can be re-derived.
fn authority_plan(
    rows: &[(String, Value)],
    mint: &MintConfig,
    asset_mint: &str,
    budget: &StressBudget,
) -> Vec<String> {
    let mut authorities = BTreeSet::new();
    for (_, account) in rows {
        if let Ok(state) =
            decode::decode_token_account(account, &mint.token_program, asset_mint, mint.decimals)
        {
            if state.raw_balance != "0" {
                authorities.insert(state.owner);
            }
        }
    }
    authorities
        .into_iter()
        .take(budget.max_authority_lookups)
        .collect()
}

/// Deterministically ordered rows, truncated to the declared decode budget.
fn ordered_rows(value: &Value, budget: &StressBudget) -> Result<(usize, Vec<(String, Value)>)> {
    let entries = value["value"]
        .as_array()
        .context("enumeration response is not a contextual account array")?;
    let mut rows: Vec<(String, Value)> = Vec::with_capacity(entries.len());
    let mut seen = BTreeSet::new();
    for entry in entries {
        let pubkey = entry["pubkey"]
            .as_str()
            .context("enumeration row missing its account address")?;
        let _: Address = pubkey.parse().context("invalid enumeration row address")?;
        ensure!(
            seen.insert(pubkey.to_string()),
            "duplicate account in enumeration response"
        );
        rows.push((pubkey.to_string(), entry["account"].clone()));
    }
    let returned = rows.len();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows.truncate(budget.max_decoded_accounts);
    Ok((returned, rows))
}

pub fn capture(
    mint: String,
    run_id: String,
    stress_id: String,
    budget: StressBudget,
    rpc: &impl SolanaRpc,
) -> Result<Capture> {
    budget.validate()?;
    let _: Address = mint.parse().context("invalid asset mint")?;
    let mut c = Capture {
        schema_version: 2,
        kind: KIND.into(),
        run_id,
        stress_id,
        mint,
        budget,
        rpc_origin: "configured-read-only-provider".into(),
        started_at: now(),
        completed_at: String::new(),
        decoder: DECODER_V2.into(),
        observations: vec![],
    };
    eprintln!("CURRENT_STAGE:Validating mainnet identity");
    let genesis = record(rpc, &mut c.observations, "getGenesisHash", json!([]));
    if genesis.as_ref().and_then(Value::as_str) == Some(MAINNET) {
        eprintln!("CURRENT_STAGE:Fetching current mint state");
        let mint_response = record(
            rpc,
            &mut c.observations,
            "getAccountInfo",
            json!([c.mint, base_config()]),
        );
        let decoded = mint_response
            .as_ref()
            .and_then(|m| decode::decode_mint(&m["value"]).ok().map(|d| (m, d)));
        if let Some((response, config)) = decoded {
            let mint_slot = context_slot(response)?;
            eprintln!("CURRENT_STAGE:Enumerating current token accounts");
            let scan = record(
                rpc,
                &mut c.observations,
                "getProgramAccounts",
                json!([config.token_program, enumeration_config(&c.mint, mint_slot)]),
            );
            if let Some(scan) = scan {
                if let Ok(enumeration_slot) = context_slot(&scan) {
                    if let Ok((_, rows)) = ordered_rows(&scan, &c.budget) {
                        let authorities = authority_plan(&rows, &config, &c.mint, &c.budget);
                        let batches = authorities.len().div_ceil(c.budget.authority_batch_size);
                        for (index, batch) in authorities
                            .chunks(c.budget.authority_batch_size)
                            .enumerate()
                        {
                            eprintln!(
                                "CURRENT_STAGE:Resolving authority models {}/{batches}",
                                index + 1
                            );
                            if record(
                                rpc,
                                &mut c.observations,
                                "getMultipleAccounts",
                                json!([batch, min_config(enumeration_slot)]),
                            )
                            .is_none()
                            {
                                // A failed batch stops resolution; the remaining
                                // authorities stay explicitly unresolved.
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
    c.completed_at = now();
    Ok(c)
}

pub fn save(c: &Capture, path: &Path) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(&serde_json::to_vec(c)?)?;
    file.sync_all()?;
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Acquisition {
    pub started_at: String,
    pub completed_at: String,
    pub rpc_origin: String,
    pub genesis_hash: String,
    pub commitment: String,
    #[serde(with = "crate::numfmt::u64_strings")]
    pub context_slots: Vec<u64>,
    /// Always false: the mint query, the scan and each authority batch are
    /// separate finalized observations, not one atomic validator bank.
    pub atomic_single_slot: bool,
    pub consistency: String,
    pub decoder: String,
    pub rpc_methods: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Enumeration {
    pub method: String,
    pub program: String,
    pub filters: Value,
    pub completeness: EnumerationCompleteness,
    #[serde(with = "crate::numfmt::optional_u64_string")]
    pub enumeration_slot: Option<u64>,
    pub rows_returned: usize,
    pub rows_decoded: usize,
    pub rows_undecodable: usize,
    pub decode_limit: usize,
    #[serde(with = "crate::numfmt::u64_string")]
    pub response_byte_limit: u64,
    pub gaps: Vec<String>,
    pub scope: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoritySummary {
    pub completeness: AuthorityResolutionCompleteness,
    pub distinct_positive_balance_authorities: usize,
    pub authorities_inspected: usize,
    pub authorities_unresolved: usize,
    pub batches_performed: usize,
    pub lookup_limit: usize,
    pub batch_size: usize,
    pub gaps: Vec<String>,
    pub independence: String,
}
pub const AUTHORITY_INDEPENDENCE: &str = "Authority resolution is independent of token-account enumeration. An incomplete authority resolution does not mean the token-account enumeration was incomplete, and a complete enumeration does not imply every authority model is known.";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PopulationCounts {
    pub token_accounts_observed: usize,
    pub positive_balance_accounts_observed: usize,
    pub zero_balance_accounts_observed: usize,
    pub undecodable_rows_observed: usize,
    pub distinct_recorded_authorities: usize,
    pub distinct_positive_balance_authorities: usize,
    pub authority_model_counts: BTreeMap<String, usize>,
    pub positive_balance_authority_model_counts: BTreeMap<String, usize>,
    pub frozen_accounts: usize,
    pub uninitialized_accounts: usize,
    pub delegate_present_accounts: usize,
    pub active_delegation_accounts: usize,
    pub close_authority_accounts: usize,
    pub account_extension_counts: BTreeMap<String, usize>,
    /// Summed over positive-balance entities, each counted exactly once.
    pub observed_public_balance_raw: String,
    pub terminology: String,
}
pub const TERMINOLOGY: &str = "These are token accounts observed in this capture, not holders. No human ownership, legal ownership, identity or key possession is established anywhere in this milestone. Public base balances exclude encrypted balances and withheld fees; a public zero is not a known confidential zero.";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PopulationObservation {
    pub schema_version: u32,
    pub kind: String,
    pub run_id: String,
    pub stress_id: String,
    pub mint: String,
    pub capture_sha256: String,
    pub budget: StressBudget,
    pub mint_config: Option<MintConfig>,
    #[serde(with = "crate::numfmt::optional_u64_string")]
    pub mint_slot: Option<u64>,
    pub acquisition: Acquisition,
    pub enumeration: Enumeration,
    pub authority_resolution: AuthoritySummary,
    #[serde(skip)]
    pub entities: Vec<StressEntity>,
    pub undecoded: Vec<UndecodedAccount>,
    pub summary: PopulationCounts,
    pub limitations: Vec<String>,
}

/// Map a recorded provider failure onto the enumeration axis. The RPC layer
/// deliberately reports only error codes, never provider text that could carry
/// a URL or credential, so this matches on those codes.
fn scan_failure(error: &str) -> (EnumerationCompleteness, String) {
    if error.contains("-32601") || error.contains("-32010") {
        (
            EnumerationCompleteness::Unsupported,
            "The configured provider cannot return this filtered getProgramAccounts enumeration as one complete response. Population discovery is unsupported for this query; no saved population was substituted.".into(),
        )
    } else if error.contains("exceeds observation budget") {
        (
            EnumerationCompleteness::Unavailable,
            "The enumeration response exceeded the declared response byte budget, so no rows were accepted. The number of current token accounts is unknown for this run.".into(),
        )
    } else {
        (
            EnumerationCompleteness::Unavailable,
            format!("Population discovery incomplete: the enumeration request did not return usable rows ({error}). No saved population was substituted."),
        )
    }
}

fn checked<'a>(c: &'a Capture, i: usize, method: &str, params: &Value) -> Result<&'a Observation> {
    let r = c
        .observations
        .get(i)
        .context("missing acquisition record")?;
    ensure!(
        r.method == method && r.params == *params,
        "population acquisition request binding mismatch at record {i}"
    );
    let start = chrono::DateTime::parse_from_rfc3339(&r.started_at)?;
    let end = chrono::DateTime::parse_from_rfc3339(&r.completed_at)?;
    ensure!(
        start <= end
            && start >= chrono::DateTime::parse_from_rfc3339(&c.started_at)?
            && end <= chrono::DateTime::parse_from_rfc3339(&c.completed_at)?,
        "invalid population retrieval interval at record {i}"
    );
    ensure!(
        r.result.is_some() != r.error.is_some(),
        "ambiguous population acquisition outcome at record {i}"
    );
    Ok(r)
}

pub fn evaluate_bytes(bytes: &[u8], budget: &StressBudget) -> Result<PopulationObservation> {
    budget.validate()?;
    ensure!(
        bytes.len() as u64 <= budget.max_artifact_bytes,
        "population capture exceeds the declared artifact budget"
    );
    let capture: Capture = serde_json::from_slice(bytes)?;
    ensure!(
        capture.budget == *budget,
        "population capture was taken under a different declared budget"
    );
    evaluate(&capture, &sha256(bytes))
}

/// Rebuild every reported fact from the retained raw responses. No analytical
/// status, count or classification is ever read out of the capture file.
pub fn evaluate(c: &Capture, capture_sha256: &str) -> Result<PopulationObservation> {
    ensure!(
        ((c.schema_version == 1 && c.decoder == DECODER)
            || (c.schema_version == 2 && c.decoder == DECODER_V2))
            && c.kind == KIND,
        "unsupported population capture version"
    );
    c.budget.validate()?;
    let _: Address = c.mint.parse().context("invalid asset mint")?;
    ensure!(
        !c.observations.is_empty() && c.observations.len() <= 4 + c.budget.max_authority_lookups,
        "incomplete or excessive population acquisition"
    );
    let mut slots: Vec<u64> = vec![];
    let mut limitations = vec![
        "This is one bounded read-only capture of current state. It is not an atomic validator bank and carries no guarantee about any later slot.".to_string(),
        "Observed token accounts are not holders, wallets or people. No human identity, legal ownership or signing access is established.".to_string(),
        "Zero public balances are excluded from exposure, and unknown or encrypted balances are never treated as zero.".to_string(),
        "No historical population, balance or classification from an earlier phase is read, imported or substituted anywhere in this run.".to_string(),
    ];

    let genesis = checked(c, 0, "getGenesisHash", &json!([]))?;
    let genesis_hash = genesis
        .result
        .as_ref()
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut enumeration = Enumeration {
        method: "getProgramAccounts (memcmp on mint at offset 0; no dataSize filter)".into(),
        program: String::new(),
        filters: json!([{"memcmp":{"offset":0,"bytes":c.mint}}]),
        completeness: EnumerationCompleteness::Unavailable,
        enumeration_slot: None,
        rows_returned: 0,
        rows_decoded: 0,
        rows_undecodable: 0,
        decode_limit: c.budget.max_decoded_accounts,
        response_byte_limit: c.budget.max_response_bytes,
        gaps: vec![],
        scope: "All current token accounts of the selected mint under its own token program, at one finalized enumeration context.".into(),
    };
    let mut authority = AuthoritySummary {
        completeness: AuthorityResolutionCompleteness::NotPerformed,
        distinct_positive_balance_authorities: 0,
        authorities_inspected: 0,
        authorities_unresolved: 0,
        batches_performed: 0,
        lookup_limit: c.budget.max_authority_lookups,
        batch_size: c.budget.authority_batch_size,
        gaps: vec![],
        independence: AUTHORITY_INDEPENDENCE.into(),
    };
    let mut entities: Vec<StressEntity> = vec![];
    let mut undecoded: Vec<UndecodedAccount> = vec![];
    let mut mint_config = None;
    let mut mint_slot = None;

    if genesis_hash != MAINNET {
        enumeration.gaps.push(
            "Mainnet identity could not be confirmed, so no population enumeration was attempted."
                .into(),
        );
        ensure!(
            c.observations.len() == 1,
            "records after a failed cluster identity check"
        );
    } else {
        let mint_record = checked(c, 1, "getAccountInfo", &json!([c.mint, base_config()]))?;
        match mint_record
            .result
            .as_ref()
            .map(|m| decode::decode_mint(&m["value"]))
        {
            Some(Ok(config)) => {
                let response = mint_record.result.as_ref().unwrap();
                let slot = context_slot(response)?;
                slots.push(slot);
                mint_slot = Some(slot);
                enumeration.program = config.token_program.clone();
                let scan = checked(
                    c,
                    2,
                    "getProgramAccounts",
                    &json!([config.token_program, enumeration_config(&c.mint, slot)]),
                )?;
                match &scan.result {
                    None => {
                        let (completeness, gap) =
                            scan_failure(scan.error.as_deref().unwrap_or("unknown error"));
                        enumeration.completeness = completeness;
                        enumeration.gaps.push(gap);
                        ensure!(
                            c.observations.len() == 3,
                            "authority records after a failed enumeration"
                        );
                    }
                    Some(value) => {
                        let enumeration_slot = context_slot(value)?;
                        ensure!(
                            enumeration_slot >= slot,
                            "enumeration context predates the mint observation"
                        );
                        slots.push(enumeration_slot);
                        enumeration.enumeration_slot = Some(enumeration_slot);
                        let (returned, rows) = ordered_rows(value, &c.budget)?;
                        // v1 retained the sorted index as a pointer into the
                        // unsorted RPC response. Keep its replay bytes intact;
                        // v2 points to the exact raw row's account object.
                        let original_indices: BTreeMap<&str, usize> = value["value"]
                            .as_array()
                            .context("missing enumeration rows")?
                            .iter()
                            .enumerate()
                            .filter_map(|(index, row)| {
                                row["pubkey"].as_str().map(|address| (address, index))
                            })
                            .collect();
                        enumeration.rows_returned = returned;
                        let truncated = returned > rows.len();
                        if truncated {
                            enumeration.gaps.push(format!(
                                "The scan returned {returned} rows, above the declared decode budget of {}. A deterministic address-ordered subset was decoded; this run is not a complete population.",
                                c.budget.max_decoded_accounts
                            ));
                        }
                        // Decode every accepted row, keeping unusable rows separate.
                        let mut decoded: Vec<(String, decode::TokenAccountState)> = vec![];
                        for (index, (address, raw)) in rows.iter().enumerate() {
                            let evidence = EvidenceRef {
                                rpc_id: 2,
                                pointer: if c.schema_version == 2 {
                                    format!("/value/{}/account", original_indices[address.as_str()])
                                } else {
                                    format!("/value/{index}")
                                },
                                slot: enumeration_slot,
                            };
                            let verified = (|| -> Result<decode::TokenAccountState> {
                                ensure!(
                                    raw["owner"].as_str() == Some(config.token_program.as_str()),
                                    "row is not owned by the mint's own token program"
                                );
                                let state = decode::decode_token_account(
                                    raw,
                                    &config.token_program,
                                    &c.mint,
                                    config.decimals,
                                )?;
                                ensure!(
                                    state.mint == c.mint,
                                    "row belongs to a different mint than the selected asset"
                                );
                                Ok(state)
                            })();
                            match verified {
                                Ok(state) => decoded.push((address.clone(), state)),
                                Err(error) => undecoded.push(UndecodedAccount {
                                    address: address.clone(),
                                    reason: error.to_string(),
                                    raw_data_sha256: decode::raw_account_bytes(raw)
                                        .ok()
                                        .map(|b| sha256(&b)),
                                    evidence,
                                }),
                            }
                        }
                        enumeration.rows_decoded = decoded.len();
                        enumeration.rows_undecodable = undecoded.len();
                        if !undecoded.is_empty() {
                            enumeration.gaps.push(format!(
                                "{} returned rows could not be verified or decoded for this mint and token program. Their original bytes are retained and they are not counted as zero balances.",
                                undecoded.len()
                            ));
                        }
                        enumeration.completeness = if truncated || !undecoded.is_empty() {
                            EnumerationCompleteness::Partial
                        } else {
                            EnumerationCompleteness::CompleteForQuery
                        };

                        // Authority resolution: an independent, separately bounded axis.
                        let mut wanted: BTreeSet<String> = BTreeSet::new();
                        for (_, state) in &decoded {
                            if state.raw_balance != "0" {
                                wanted.insert(state.owner.clone());
                            }
                        }
                        authority.distinct_positive_balance_authorities = wanted.len();
                        let planned: Vec<String> = wanted
                            .iter()
                            .take(c.budget.max_authority_lookups)
                            .cloned()
                            .collect();
                        if planned.len() < wanted.len() {
                            authority.gaps.push(format!(
                                "{} distinct positive-balance authorities exceed the declared lookup budget of {}. The remainder stays explicitly unresolved and receives no assumed signer.",
                                wanted.len(),
                                c.budget.max_authority_lookups
                            ));
                        }
                        let mut resolved: BTreeMap<
                            String,
                            (EntityType, AuthorityObservation, String, EvidenceRef),
                        > = BTreeMap::new();
                        let expected_batches =
                            planned.len().div_ceil(c.budget.authority_batch_size.max(1));
                        for (batch_index, batch) in
                            planned.chunks(c.budget.authority_batch_size).enumerate()
                        {
                            let index = 3 + batch_index;
                            let Some(observation) = c.observations.get(index) else {
                                authority.gaps.push(
                                    "Authority resolution stopped before every planned batch was requested.".into(),
                                );
                                break;
                            };
                            let _ = checked(
                                c,
                                index,
                                "getMultipleAccounts",
                                &json!([batch, min_config(enumeration_slot)]),
                            )?;
                            let Some(result) = &observation.result else {
                                authority.gaps.push(format!(
                                    "An authority batch could not be retrieved ({}). The authorities in it and every later batch stay unresolved.",
                                    observation.error.as_deref().unwrap_or("unknown error")
                                ));
                                ensure!(
                                    c.observations.len() == index + 1,
                                    "authority records continue after a failed batch"
                                );
                                break;
                            };
                            let batch_slot = context_slot(result)?;
                            ensure!(
                                batch_slot >= enumeration_slot,
                                "authority observation predates the enumeration context"
                            );
                            slots.push(batch_slot);
                            let values = result["value"]
                                .as_array()
                                .context("invalid authority batch response")?;
                            ensure!(
                                values.len() == batch.len(),
                                "incomplete authority batch response"
                            );
                            authority.batches_performed += 1;
                            for (offset, (address, raw)) in
                                batch.iter().zip(values.iter()).enumerate()
                            {
                                let (model, observed, reason) = classify_authority(address, raw)?;
                                resolved.insert(
                                    address.clone(),
                                    (
                                        model,
                                        observed,
                                        reason,
                                        EvidenceRef {
                                            rpc_id: index,
                                            pointer: format!("/value/{offset}"),
                                            slot: batch_slot,
                                        },
                                    ),
                                );
                            }
                        }
                        ensure!(
                            c.observations.len() <= 3 + expected_batches,
                            "unplanned authority records in the population capture"
                        );
                        authority.authorities_inspected = resolved.len();
                        authority.authorities_unresolved = wanted.len() - resolved.len();
                        // Nothing to resolve counts as fully resolved: it is an
                        // empty obligation, not an unperformed one.
                        authority.completeness = if resolved.len() == wanted.len() {
                            AuthorityResolutionCompleteness::Complete
                        } else if resolved.is_empty() {
                            AuthorityResolutionCompleteness::NotPerformed
                        } else {
                            AuthorityResolutionCompleteness::Partial
                        };

                        for (index, (address, state)) in decoded.into_iter().enumerate() {
                            let evidence = EvidenceRef {
                                rpc_id: 2,
                                pointer: if c.schema_version == 2 {
                                    format!("/value/{}/account", original_indices[address.as_str()])
                                } else {
                                    format!("/value/{index}")
                                },
                                slot: enumeration_slot,
                            };
                            let found = resolved.get(&state.owner);
                            let (model, observation, reason, authority_evidence, resolution) =
                                match found {
                                    Some((m, o, r, e)) => (
                                        m.clone(),
                                        o.clone(),
                                        r.clone(),
                                        Some(e.clone()),
                                        AuthorityResolution::Resolved,
                                    ),
                                    None => (
                                        EntityType::Unknown,
                                        AuthorityObservation {
                                            is_on_curve: state
                                                .owner
                                                .parse::<Address>()
                                                .map(|a| a.is_on_curve())
                                                .unwrap_or(false),
                                            account_exists: false,
                                            runtime_owner: None,
                                            executable: None,
                                            multisig: None,
                                        },
                                        "Authority account was not inspected in this capture; its model is unknown and no signing control is assumed.".to_string(),
                                        None,
                                        AuthorityResolution::NotResolved,
                                    ),
                                };
                            entities.push(StressEntity {
                                entity_id: entity_id(capture_sha256, &address),
                                token_account: address.clone(),
                                mint: c.mint.clone(),
                                token_program: config.token_program.clone(),
                                authority: state.owner.clone(),
                                authority_model: model,
                                authority_resolution: resolution,
                                authority_observation: observation,
                                classification_reason: reason,
                                state,
                                token_account_evidence: evidence,
                                authority_evidence,
                            });
                        }
                    }
                }
                mint_config = Some(config);
            }
            Some(Err(error)) => {
                enumeration.gaps.push(format!(
                    "The selected address could not be decoded as a supported token mint, so no population enumeration was attempted: {error}"
                ));
                enumeration.completeness = EnumerationCompleteness::Unsupported;
                ensure!(
                    c.observations.len() == 2,
                    "records after an undecodable mint"
                );
            }
            None => {
                enumeration.gaps.push(
                    "The current mint state could not be retrieved, so no population enumeration was attempted.".into(),
                );
                ensure!(
                    c.observations.len() == 2,
                    "records after a failed mint observation"
                );
            }
        }
    }

    entities.sort_by(|a, b| a.token_account.cmp(&b.token_account));
    undecoded.sort_by(|a, b| a.address.cmp(&b.address));

    let mut summary = PopulationCounts {
        token_accounts_observed: entities.len(),
        positive_balance_accounts_observed: 0,
        zero_balance_accounts_observed: 0,
        undecodable_rows_observed: undecoded.len(),
        distinct_recorded_authorities: 0,
        distinct_positive_balance_authorities: authority.distinct_positive_balance_authorities,
        authority_model_counts: BTreeMap::new(),
        positive_balance_authority_model_counts: BTreeMap::new(),
        frozen_accounts: 0,
        uninitialized_accounts: 0,
        delegate_present_accounts: 0,
        active_delegation_accounts: 0,
        close_authority_accounts: 0,
        account_extension_counts: BTreeMap::new(),
        observed_public_balance_raw: "0".into(),
        terminology: TERMINOLOGY.into(),
    };
    let mut authorities = BTreeSet::new();
    // Balances are attributed to an exact entity id once, so nothing is double counted.
    let mut balances: BTreeMap<String, u64> = BTreeMap::new();
    for e in &entities {
        authorities.insert(e.authority.clone());
        let key = serde_json::to_value(&e.authority_model)
            .expect("entity type")
            .as_str()
            .expect("entity type string")
            .to_string();
        *summary
            .authority_model_counts
            .entry(key.clone())
            .or_insert(0) += 1;
        let balance = e.balance()?;
        if balance > 0 {
            summary.positive_balance_accounts_observed += 1;
            *summary
                .positive_balance_authority_model_counts
                .entry(key)
                .or_insert(0) += 1;
            balances.insert(e.entity_id.clone(), balance);
        } else {
            summary.zero_balance_accounts_observed += 1;
        }
        if e.state.is_frozen {
            summary.frozen_accounts += 1;
        }
        if !e.state.is_initialized {
            summary.uninitialized_accounts += 1;
        }
        if e.state.delegate.is_some() {
            summary.delegate_present_accounts += 1;
        }
        if e.state.has_active_delegate {
            summary.active_delegation_accounts += 1;
        }
        if e.state.close_authority.is_some() {
            summary.close_authority_accounts += 1;
        }
        for extension in &e.state.extensions {
            *summary
                .account_extension_counts
                .entry(extension.extension_type.clone())
                .or_insert(0) += 1;
        }
    }
    summary.distinct_recorded_authorities = authorities.len();
    summary.observed_public_balance_raw = super::population_types::sum_once(&balances);

    if enumeration.completeness != EnumerationCompleteness::CompleteForQuery {
        limitations.push("Population discovery is incomplete for this run. Counts describe only what this capture actually observed and are not the token's holder population.".into());
    }
    if authority.completeness != AuthorityResolutionCompleteness::Complete {
        limitations.push("Some recorded authorities were not inspected. Their authority model is unknown, never assumed wallet-compatible, and never given an assumed local signer.".into());
    }
    slots.sort_unstable();
    slots.dedup();

    Ok(PopulationObservation {
        schema_version: c.schema_version,
        kind: KIND.into(),
        run_id: c.run_id.clone(),
        stress_id: c.stress_id.clone(),
        mint: c.mint.clone(),
        capture_sha256: capture_sha256.into(),
        budget: c.budget.clone(),
        mint_config,
        mint_slot,
        acquisition: Acquisition {
            started_at: c.started_at.clone(),
            completed_at: c.completed_at.clone(),
            rpc_origin: c.rpc_origin.clone(),
            genesis_hash,
            commitment: "finalized".into(),
            context_slots: slots,
            atomic_single_slot: false,
            consistency: "The mint query, the account enumeration and each authority batch are separate finalized observations with their own contexts. No single-slot atomic population state is claimed.".into(),
            decoder: c.decoder.clone(),
            rpc_methods: c.observations.iter().map(|o| o.method.clone()).collect(),
        },
        enumeration,
        authority_resolution: authority,
        entities,
        undecoded,
        summary,
        limitations,
    })
}

impl PopulationObservation {
    /// True only when both independent axes are fully satisfied.
    pub fn fully_resolved(&self) -> bool {
        self.enumeration.completeness == EnumerationCompleteness::CompleteForQuery
            && self.authority_resolution.completeness == AuthorityResolutionCompleteness::Complete
    }
    pub fn positive_entities(&self) -> impl Iterator<Item = &StressEntity> {
        self.entities.iter().filter(|e| e.state.raw_balance != "0")
    }
}
