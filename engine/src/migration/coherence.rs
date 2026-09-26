//! One generic, replay-derived execution-context contract for current checks.
//! A finalized `minContextSlot` is only a lower bound. The returned account
//! batch and its Clock bytes must independently satisfy the final-bank rule.
use crate::standard_programs::clock::CapturedClock;
use crate::standard_programs::token::CLOCK;
use crate::{
    ingest::observation::{Observation, RpcEvidence},
    replay::hash_bytes as sha256,
    standard_programs::token as decode,
};
fn slot(value: &serde_json::Value) -> anyhow::Result<u64> {
    value["context"]["slot"]
        .as_u64()
        .context("missing response context slot")
}
fn read_clock(raw: &serde_json::Value) -> anyhow::Result<CapturedClock> {
    anyhow::ensure!(
        raw["owner"] == crate::migration::world::SYSVAR_OWNER && raw["executable"] == false,
        "invalid Clock runtime identity"
    );
    CapturedClock::from_bytes(&decode::raw_account_bytes(raw)?)
}
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

pub const MAX_FINAL_ATTEMPTS: usize = 3;
pub const FINAL_CAPTURE_TIMEOUT: Duration = Duration::from_secs(90);
pub const COHERENCE_FAILURE: &str = "CouldNotEstablishCoherentExecutionContext";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputClass {
    ExecutionCritical,
    StaticCodeIdentity,
    DiscoveryOnly,
    ProposedOverlay,
}

#[derive(Clone, Debug, Serialize)]
pub struct RequiredAccount {
    pub address: String,
    pub class: InputClass,
    pub data_sha256: Option<String>,
    pub runtime_owner: Option<String>,
    #[serde(with = "crate::numfmt::optional_u64_string")]
    pub lamports: Option<u64>,
    pub executable: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SeparateInput {
    pub identity: String,
    pub class: InputClass,
    pub origin: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct CaptureAttempt {
    #[serde(with = "crate::numfmt::u64_string")]
    pub min_context_slot: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub response_context_slot: u64,
    #[serde(with = "crate::numfmt::u64_string")]
    pub clock_slot: u64,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct DiscoveryContext {
    pub method: String,
    #[serde(with = "crate::numfmt::optional_u64_string")]
    pub response_context_slot: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
// This private constructor seal also excludes sibling modules in this crate.
#[allow(clippy::manual_non_exhaustive)]
pub struct ExecutionContext {
    #[serde(skip)]
    _verified: (),
    pub rule: &'static str,
    #[serde(with = "crate::numfmt::u64_string")]
    pub final_context_slot: u64,
    pub atomic_single_slot: bool,
    pub max_final_attempts: usize,
    pub discovery_contexts: Vec<DiscoveryContext>,
    pub attempts: Vec<CaptureAttempt>,
    pub required_accounts: Vec<RequiredAccount>,
    pub separate_inputs: Vec<SeparateInput>,
    pub clock_data_sha256: String,
    #[serde(with = "crate::numfmt::u64_string")]
    pub clock_slot: u64,
    pub coherence_status: &'static str,
}

/// Descriptive diagnostics for incomplete captures. These fields are never
/// treated as a proof; `verify` is the only constructor for a verified context.
pub fn diagnostics(records: &[Observation]) -> Value {
    json!({
        "atomic_single_slot": false,
        "coherence_status": "Unverified",
        "failure_reason": COHERENCE_FAILURE,
        "max_final_attempts": MAX_FINAL_ATTEMPTS,
        "discovery_contexts": records.iter().take(4).map(|record| json!({
            "method": record.method,
            "response_context_slot": record.result.as_ref().and_then(|value| slot(value).ok()).map(|slot| slot.to_string()),
        })).collect::<Vec<_>>(),
        "attempts": records.iter().skip(4).map(|record| {
            let response = record.result.as_ref();
            let clock_index = record.params[0].as_array().and_then(|addresses|
                addresses.iter().position(|address| address == CLOCK));
            let clock = response.and_then(|value| clock_index.and_then(|index|
                value["value"].as_array().and_then(|values| values.get(index))))
                .and_then(|raw| read_clock(raw).ok());
            json!({
                "min_context_slot": record.params[1]["minContextSlot"].as_u64().map(|slot| slot.to_string()),
                "response_context_slot": response.and_then(|value| slot(value).ok()).map(|slot| slot.to_string()),
                "clock_slot": clock.map(|value| value.slot.to_string()),
                "reason": record.error.as_deref().unwrap_or("ClockContextMismatchOrIncomplete"),
            })
        }).collect::<Vec<_>>()
    })
}

pub fn config(min: u64) -> Value {
    json!({"encoding":"base64","commitment":"finalized","minContextSlot":min})
}

fn clock_slot(response: &Value, clock_index: usize) -> Result<u64> {
    let values = response["value"]
        .as_array()
        .context("missing final account values")?;
    let clock = values
        .get(clock_index)
        .context("Clock missing from final account batch")?;
    Ok(read_clock(clock)?.slot)
}

fn next_min(response: &Value, clock_index: usize, min: u64) -> Result<Option<u64>> {
    let context = slot(response)?;
    ensure!(context >= min, "final batch predates minContextSlot");
    let clock = clock_slot(response, clock_index)?;
    if context == clock {
        Ok(None)
    } else {
        Ok(Some(
            context.max(clock).checked_add(1).context("slot overflow")?,
        ))
    }
}

/// Records at most three serial final-batch attempts. The callback stores each
/// RPC observation in the parent capture, including provider errors.
pub fn capture_final(
    addresses: &[String],
    initial_min: u64,
    mut call: impl FnMut(Value) -> Option<Value>,
) {
    let Some(clock_index) = addresses.iter().position(|a| a == CLOCK) else {
        return;
    };
    let deadline = Instant::now() + FINAL_CAPTURE_TIMEOUT;
    let mut min = initial_min;
    for _ in 0..MAX_FINAL_ATTEMPTS {
        if Instant::now() >= deadline {
            break;
        }
        let Some(response) = call(json!([addresses, config(min)])) else {
            break;
        };
        match next_min(&response, clock_index, min) {
            Ok(Some(next)) => min = next,
            Ok(None) | Err(_) => break,
        }
    }
}

/// Recomputes the entire retry chain from raw RPC records; a claimed coherent
/// flag cannot construct this value. The accepted final batch is the only bank.
pub fn verify(
    evidence: &[RpcEvidence],
    addresses: &[String],
    first_min: u64,
    source_account: &str,
) -> Result<ExecutionContext> {
    verify_after(evidence, 4, addresses, first_min, source_account)
}

/// Same contract with an explicit discovery prefix for migration's account plan.
pub fn verify_after(
    evidence: &[RpcEvidence],
    discovery_count: usize,
    addresses: &[String],
    first_min: u64,
    source_account: &str,
) -> Result<ExecutionContext> {
    let max_records = discovery_count
        .checked_add(MAX_FINAL_ATTEMPTS)
        .context("discovery prefix overflow")?;
    ensure!(
        !addresses.is_empty()
            && addresses.len() <= 100
            && addresses
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == addresses.len(),
        "invalid or duplicate final account set"
    );
    ensure!(
        (discovery_count + 1..=max_records).contains(&evidence.len()),
        "invalid final execution recapture budget"
    );
    let clock_index = addresses
        .iter()
        .position(|a| a == CLOCK)
        .context("Clock omitted from execution-critical account set")?;
    let mut min = first_min;
    let discovery_contexts = evidence[..discovery_count]
        .iter()
        .map(|record| DiscoveryContext {
            method: record.method.clone(),
            response_context_slot: slot(&record.result).ok(),
        })
        .collect();
    let mut attempts = Vec::new();
    let mut first_values: Option<Vec<Value>> = None;
    for (index, record) in evidence[discovery_count..].iter().enumerate() {
        ensure!(
            record.method == "getMultipleAccounts"
                && record.params == json!([addresses, config(min)]),
            "final recapture request or minContextSlot chain changed"
        );
        let slot = slot(&record.result)?;
        ensure!(
            slot >= min,
            "final recapture response predates its lower bound"
        );
        let values = record.result["value"]
            .as_array()
            .context("missing final account batch")?;
        ensure!(
            values.len() == addresses.len(),
            "incomplete final account batch"
        );
        if let Some(previous) = &first_values {
            for (account_index, address) in addresses.iter().enumerate() {
                if account_index != clock_index && previous[account_index] != values[account_index]
                {
                    anyhow::bail!(if address == source_account {
                        "SourceStateChanged"
                    } else if previous[account_index]["executable"] == true
                        || values[account_index]["executable"] == true
                    {
                        "ProgramCodeChanged"
                    } else {
                        "ExecutionCriticalStateChanged"
                    });
                }
            }
        } else {
            first_values = Some(values.clone());
        }
        let clock = clock_slot(&record.result, clock_index)?;
        let coherent = clock == slot;
        attempts.push(CaptureAttempt {
            min_context_slot: min,
            response_context_slot: slot,
            clock_slot: clock,
            reason: if coherent {
                "Accepted"
            } else {
                "ClockContextMismatch"
            }
            .into(),
        });
        if coherent {
            ensure!(
                index + discovery_count + 1 == evidence.len(),
                "recapture continued after coherence"
            );
            let required_accounts = addresses
                .iter()
                .zip(values)
                .map(|(address, raw)| {
                    Ok(RequiredAccount {
                        address: address.clone(),
                        class: if address == CLOCK || !raw.is_null() && raw["executable"] != true {
                            InputClass::ExecutionCritical
                        } else {
                            InputClass::StaticCodeIdentity
                        },
                        data_sha256: if raw.is_null() {
                            None
                        } else {
                            Some(sha256(&decode::raw_account_bytes(raw)?))
                        },
                        runtime_owner: raw["owner"].as_str().map(str::to_string),
                        lamports: raw["lamports"].as_u64(),
                        executable: raw["executable"].as_bool(),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let clock_data_sha256 = sha256(&decode::raw_account_bytes(&values[clock_index])?);
            return Ok(ExecutionContext {
                _verified: (),
                rule: "finalized complete getMultipleAccounts batch; Clock bytes slot equals response context; monotonic minContextSlot retry",
                final_context_slot: slot,
                atomic_single_slot: false,
                max_final_attempts: MAX_FINAL_ATTEMPTS,
                discovery_contexts,
                attempts,
                required_accounts,
                separate_inputs: Vec::new(),
                clock_data_sha256,
                clock_slot: clock,
                coherence_status: "Verified",
            });
        }
        min = slot.max(clock).checked_add(1).context("slot overflow")?;
    }
    anyhow::bail!(COHERENCE_FAILURE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn raw(owner: &str, bytes: &[u8], executable: bool) -> Value {
        json!({"owner":owner,"lamports":100,"executable":executable,
            "data":[base64::engine::general_purpose::STANDARD.encode(bytes),"base64"]})
    }
    fn response(context: u64, clock: u64, source: u8, program: u8) -> Value {
        let mut clock_bytes = [0u8; 40];
        clock_bytes[..8].copy_from_slice(&clock.to_le_bytes());
        json!({"context":{"slot":context},"value":[
            raw("token", &[source], false),
            raw("Sysvar1111111111111111111111111111111111111", &clock_bytes, false),
            raw("loader", &[program], true)
        ]})
    }
    fn addresses() -> Vec<String> {
        vec!["source".into(), CLOCK.into(), "program".into()]
    }
    fn transcript(attempts: &[(u64, Value)]) -> Vec<RpcEvidence> {
        let mut evidence: Vec<RpcEvidence> = (0..4)
            .map(|id| RpcEvidence {
                id,
                method: "discovery".into(),
                params: json!([]),
                result: json!(null),
            })
            .collect();
        for (min, result) in attempts {
            evidence.push(RpcEvidence {
                id: evidence.len(),
                method: "getMultipleAccounts".into(),
                params: json!([addresses(), config(*min)]),
                result: result.clone(),
            });
        }
        evidence
    }

    #[test]
    fn later_clock_and_earlier_account_context_never_grant_proof() {
        let evidence = transcript(&[(100, response(100, 102, 1, 1))]);
        assert!(
            verify(&evidence, &addresses(), 100, "source")
                .err()
                .is_some_and(|error| error.to_string().contains(COHERENCE_FAILURE)),
            "later_clock_earlier_bank_rejected"
        );
    }

    #[test]
    fn earlier_clock_and_later_account_context_never_grant_proof() {
        let evidence = transcript(&[(100, response(102, 100, 1, 1))]);
        assert!(verify(&evidence, &addresses(), 100, "source")
            .unwrap_err()
            .to_string()
            .contains(COHERENCE_FAILURE));
    }

    #[test]
    fn coherent_final_batch_passes_with_exact_clock_bytes_and_no_atomicity_claim() {
        let evidence = transcript(&[(100, response(100, 100, 1, 1))]);
        let context = verify(&evidence, &addresses(), 100, "source").unwrap();
        assert_eq!(context.final_context_slot, 100);
        assert_eq!(context.clock_slot, 100);
        assert!(!context.atomic_single_slot);
        assert_eq!(context.required_accounts.len(), 3);
        assert_eq!(context.attempts.len(), 1);
    }

    #[test]
    fn retry_lower_bound_is_monotonic_and_exact() {
        let evidence = transcript(&[
            (100, response(100, 102, 1, 1)),
            (103, response(103, 103, 1, 1)),
        ]);
        let context = verify(&evidence, &addresses(), 100, "source").unwrap();
        assert_eq!(context.attempts[0].reason, "ClockContextMismatch");
        assert_eq!(context.attempts[1].min_context_slot, 103);
        let mut altered = evidence;
        altered[5].params[1]["minContextSlot"] = json!(102);
        assert!(verify(&altered, &addresses(), 100, "source").is_err());
    }

    #[test]
    fn retry_budget_stops_without_approximation_or_peer_replacement() {
        let mut calls = 0;
        let expected = addresses();
        capture_final(&expected, 100, |params| {
            assert_eq!(params[0], json!(expected));
            calls += 1;
            let min = params[1]["minContextSlot"].as_u64().unwrap();
            Some(response(min, min + 2, 1, 1))
        });
        assert_eq!(calls, MAX_FINAL_ATTEMPTS);
        let evidence = transcript(&[
            (100, response(100, 102, 1, 1)),
            (103, response(103, 105, 1, 1)),
            (106, response(106, 108, 1, 1)),
        ]);
        assert!(verify(&evidence, &addresses(), 100, "source")
            .unwrap_err()
            .to_string()
            .contains(COHERENCE_FAILURE));
    }

    #[test]
    fn changed_source_or_program_during_recapture_is_indeterminate() {
        let source = transcript(&[
            (100, response(100, 102, 1, 1)),
            (103, response(103, 103, 2, 1)),
        ]);
        assert!(
            verify(&source, &addresses(), 100, "source")
                .err()
                .is_some_and(|error| error.to_string().contains("SourceStateChanged")),
            "recapture_source_drift_rejected"
        );
        let code = transcript(&[
            (100, response(100, 102, 1, 1)),
            (103, response(103, 103, 1, 2)),
        ]);
        assert!(verify(&code, &addresses(), 100, "source")
            .unwrap_err()
            .to_string()
            .contains("ProgramCodeChanged"));
    }

    #[test]
    fn serialized_success_or_rewritten_min_chain_cannot_bypass_replay() {
        let mut evidence = transcript(&[(100, response(100, 102, 1, 1))]);
        evidence[4].result["coherence_status"] = json!("Verified");
        assert!(
            verify(&evidence, &addresses(), 100, "source").is_err(),
            "serialized_coherence_flag_rejected"
        );
        evidence[4].result["context"]["slot"] = json!(102);
        evidence[4].params[1]["minContextSlot"] = json!(101);
        assert!(verify(&evidence, &addresses(), 100, "source").is_err());
    }
}
