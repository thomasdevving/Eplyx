const BASE58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const DEADLINE_STATUSES = new Set([
  "PostDeadlineTransitionRequired",
  "Expired",
  "NoIssuerEntitlement",
  "Unknown",
]);

function lifecyclePublicKey(value) {
  const text = String(value || "").trim();
  if (text.length < 32 || text.length > 44) return false;

  let bytes = [0];
  for (const character of text) {
    const digit = BASE58_ALPHABET.indexOf(character);
    if (digit < 0) return false;

    let carry = digit;
    for (let index = 0; index < bytes.length; index += 1) {
      carry += bytes[index] * 58;
      bytes[index] = carry & 255;
      carry >>= 8;
    }
    while (carry > 0) {
      bytes.push(carry & 255);
      carry >>= 8;
    }
  }

  for (let index = 0; index < text.length - 1 && text[index] === "1"; index += 1) bytes.push(0);
  return bytes.length === 32;
}

function lifecycleUtcInputToUnix(value) {
  const text = String(value || "").trim();
  const match = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})(?::(\d{2}))?$/.exec(text);
  if (!match) return null;
  const [, year, month, day, hour, minute, second = "00"] = match;
  const parts = [year, month, day, hour, minute, second].map(Number);
  const milliseconds = Date.UTC(parts[0], parts[1] - 1, parts[2], parts[3], parts[4], parts[5]);
  if (!Number.isFinite(milliseconds)) return null;
  const roundTrip = new Date(milliseconds).toISOString().slice(0, 19);
  if (roundTrip !== `${year}-${month}-${day}T${hour}:${minute}:${second}`) return null;
  const seconds = milliseconds / 1000;
  return Number.isSafeInteger(seconds) ? String(seconds) : null;
}

function lifecycleUnixToRfc3339(value) {
  const seconds = Number(value);
  if (!Number.isSafeInteger(seconds)) return null;
  return new Date(seconds * 1000).toISOString().replace(".000Z", "Z");
}

export function canEnterPreparedLifecycle(capability) {
  return capability?.kind === "lifecycle_change" && capability?.can_submit === true;
}

export function validateLifecycleDraft(draft) {
  const errors = {};
  const effective = lifecycleUtcInputToUnix(draft.effective_at);
  const captured = lifecycleUtcInputToUnix(draft.captured_at);

  if (!String(draft.scenario_id || "").trim()) {
    errors.scenario_id = "Enter a scenario ID.";
  }
  if (!String(draft.description || "").trim()) {
    errors.description = "Describe the declared policy change.";
  }
  if (!String(draft.reference || "").trim()) {
    errors.reference = "Enter the source or proposal reference for this declaration.";
  }
  if (!lifecyclePublicKey(draft.asset_mint)) {
    errors.asset_mint = "Enter a canonical 32-byte base58 mint key.";
  }
  if (effective === null) {
    errors.effective_at = "Enter a valid whole-second UTC effective time.";
  }
  if (captured === null) {
    errors.captured_at = "Enter a valid whole-second UTC declaration capture time.";
  }

  if (draft.has_successor) {
    if (!lifecyclePublicKey(draft.successor_mint)) {
      errors.successor_mint = "Enter a canonical 32-byte base58 successor mint key.";
    } else if (String(draft.successor_mint).trim() === String(draft.asset_mint).trim()) {
      errors.successor_mint = "The successor mint must differ from the current asset mint.";
    }
    if (!String(draft.successor_description || "").trim()) {
      errors.successor_description = "Describe the declared successor.";
    }
  }

  if (draft.has_deadline) {
    const deadline = lifecycleUtcInputToUnix(draft.deadline_at);
    if (deadline === null) {
      errors.deadline_at = "Enter a valid whole-second UTC deadline.";
    } else if (effective !== null && BigInt(deadline) <= BigInt(effective)) {
      errors.deadline_at = "The deadline must be after the effective time.";
    }
    if (!DEADLINE_STATUSES.has(draft.post_deadline_status)) {
      errors.post_deadline_status = "Choose a supported post-deadline status.";
    }
  }

  if (!draft.snapshot) {
    errors.snapshot = "Choose the immutable lifecycle snapshot JSON file.";
  }
  return errors;
}

export function buildLifecycleDocuments(draft) {
  const errors = validateLifecycleDraft(draft);
  if (Object.keys(errors).length > 0) {
    throw new Error("Lifecycle draft is incomplete.");
  }

  const effective = lifecycleUtcInputToUnix(draft.effective_at);
  const captured = lifecycleUtcInputToUnix(draft.captured_at);
  const before = String(BigInt(effective) - 1n);
  const supports = ["/policy/effective_at", "/policy/before", "/policy/after"];
  if (draft.has_deadline) supports.push("/policy/deadline");
  if (draft.has_successor) supports.push("/policy/successor");

  const scenario = {
    schema_version: 1,
    scenario_type: "lifecycle_change",
    id: String(draft.scenario_id).trim(),
    scenario_version: "browser-prepared/v1",
    captured_at: lifecycleUnixToRfc3339(captured),
    change: {
      description: String(draft.description).trim(),
    },
    policy: {
      asset_mint: String(draft.asset_mint).trim(),
      effective_at: lifecycleUnixToRfc3339(effective),
      before: "Active",
      after: "TransitionRequired",
      deadline: draft.has_deadline
        ? {
            at: lifecycleUnixToRfc3339(lifecycleUtcInputToUnix(draft.deadline_at)),
            after: draft.post_deadline_status,
          }
        : null,
      successor: draft.has_successor
        ? {
            mint: String(draft.successor_mint).trim(),
            description: String(draft.successor_description).trim(),
          }
        : null,
    },
    sources: [
      {
        id: "browser-declaration",
        kind: "ScenarioAssumption",
        reference: String(draft.reference).trim(),
        description: "User-provided hypothetical lifecycle policy; not issuer verification.",
        captured_at: lifecycleUnixToRfc3339(captured),
        supports,
        artifact: null,
        content_sha256: null,
      },
    ],
  };

  const mappedSupports = supports.map((pointer) => {
    if (pointer === "/policy/effective_at") return "/activation/unix_timestamp";
    if (pointer === "/policy/before") return "/change/before";
    if (pointer === "/policy/after") return "/change/after";
    if (pointer === "/policy/deadline") return "/change/deadline";
    return "/change/destination";
  });
  const change = {
    kind: "lifecycle_change",
    asset: { mint: scenario.policy.asset_mint },
    destination: scenario.policy.successor
      ? { mint: scenario.policy.successor.mint }
      : null,
    eligibility: { kind: "unknown" },
    deadline: scenario.policy.deadline
      ? {
          unix_timestamp: lifecycleUtcInputToUnix(draft.deadline_at),
          after: scenario.policy.deadline.after,
        }
      : null,
    before: scenario.policy.before,
    after: scenario.policy.after,
    sources: [
      {
        id: scenario.sources[0].id,
        kind: scenario.sources[0].kind,
        reference: scenario.sources[0].reference,
        supports: mappedSupports,
      },
    ],
  };
  if (change.destination === null) delete change.destination;
  if (change.deadline === null) delete change.deadline;

  const changeSpec = {
    schema_version: 1,
    change,
    activation: { unix_timestamp: Number(effective) },
    metadata: {
      label: scenario.id,
      source: scenario.change.description,
    },
  };
  const analysisOptions = {
    before: lifecycleUnixToRfc3339(before),
    at: lifecycleUnixToRfc3339(effective),
  };

  return {
    scenario,
    changeSpec,
    analysisOptions,
    scenarioDocument: `${JSON.stringify(scenario, null, 2)}\n`,
    changeSpecDocument: `${JSON.stringify(changeSpec, null, 2)}\n`,
    analysisOptionsDocument: `${JSON.stringify(analysisOptions, null, 2)}\n`,
  };
}

export async function digestLifecycleFile(file) {
  const bytes = await file.arrayBuffer();
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("");
}

export function lifecycleAcceptanceMismatch(change, draft) {
  if (change?.kind !== "lifecycle_change") return "Server accepted a different change kind.";
  if (!/^[a-f0-9]{64}$/.test(String(change.change_spec_id || ""))) {
    return "Server did not return a canonical lifecycle proposal identity.";
  }
  if (change.label !== String(draft.scenario_id || "").trim()) {
    return "Server accepted a different lifecycle scenario label.";
  }
  if (change.asset_mint !== String(draft.asset_mint || "").trim()) {
    return "Server accepted a different asset mint.";
  }
  const expectedDestination = draft.has_successor ? String(draft.successor_mint || "").trim() : null;
  if ((change.destination_mint ?? null) !== expectedDestination) {
    return "Server accepted a different successor mint.";
  }
  return null;
}

export function lifecycleSubmissionError(error) {
  if (error?.kind === "accepted_identity") return { kind: "accepted_identity", message: error.message };
  if (error?.status === 401 || error?.status === 403) return { kind: "auth", message: "Sign in again to submit this analysis." };
  if (error?.status === 409) return { kind: "stale", message: `Project readiness changed before submission: ${error.message}` };
  if (error?.status === 400 || error?.status === 413) return { kind: "validation", message: `Eplyx rejected the prepared inputs: ${error.message}` };
  if (error?.status === 0) return { kind: "job_creation", message: "Eplyx could not be reached, so no accepted run is known." };
  return { kind: "job_creation", message: `Eplyx could not create the analysis run: ${error?.message || "Unknown error."}` };
}

export async function submitPreparedLifecycle({ projectId, draft, request }) {
  const documents = buildLifecycleDocuments(draft);
  const body = new FormData();
  body.append("change_spec", new Blob([documents.changeSpecDocument], { type: "application/json" }), "change.json");
  body.append("snapshot", draft.snapshot, draft.snapshot.name);
  body.append("scenario", new Blob([documents.scenarioDocument], { type: "application/json" }), "scenario.json");
  body.append("analysis_options", new Blob([documents.analysisOptionsDocument], { type: "application/json" }), "analysis-options.json");

  const accepted = await request(`/v1/projects/${encodeURIComponent(projectId)}/checks`, {
    method: "POST",
    body,
  });
  if (typeof accepted.run_id !== "string" || !accepted.run_id) {
    const error = new Error("The server accepted the proposal without returning a run identifier.");
    error.kind = "accepted_identity";
    throw error;
  }
  const mismatch = lifecycleAcceptanceMismatch(accepted.change, draft);
  if (mismatch) {
    const error = new Error(`${mismatch} The accepted run is ${accepted?.run_id ?? "unknown"}; do not rely on it.`);
    error.kind = "accepted_identity";
    throw error;
  }
  if (accepted.status_url !== `/v1/runs/${accepted.run_id}`) {
    const error = new Error("The server returned a status route that does not match the accepted run.");
    error.kind = "accepted_identity";
    throw error;
  }
  return {
    accepted,
    route: `/p/${encodeURIComponent(projectId)}/runs/${encodeURIComponent(accepted.run_id)}`,
  };
}
