// Pure browser-side preparation for one deliberately narrow Token Migration V1
// shape. This module maps form concepts to the existing ChangeSpec wire shape;
// it does not quote, evaluate, normalize or decide whether a migration works.

export const LEGACY_TOKEN_PROGRAM = 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA';
export const TOKEN_2022_PROGRAM = 'TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb';
export const U64_MAX = 18_446_744_073_709_551_615n;

const BASE58 = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
const UINT = /^(0|[1-9][0-9]*)$/;

export function canEnterPreparedMigration(capability) {
  return capability?.kind === 'token_migration' && capability?.can_submit === true;
}

/** Decode only far enough to establish canonical Solana public-key syntax. */
export function isPublicKey(value) {
  if (typeof value !== 'string' || !value || [...value].some(character => !BASE58.includes(character))) return false;
  const bytes = [0];
  for (const character of value) {
    let carry = BASE58.indexOf(character);
    for (let index = 0; index < bytes.length; index += 1) {
      carry += bytes[index] * 58;
      bytes[index] = carry & 0xff;
      carry >>= 8;
    }
    while (carry > 0) {
      bytes.push(carry & 0xff);
      carry >>= 8;
    }
  }
  for (let index = 0; index < value.length - 1 && value[index] === '1'; index += 1) bytes.push(0);
  return bytes.length === 32;
}

export function canonicalU64(value, { positive = false } = {}) {
  const text = String(value ?? '');
  if (!UINT.test(text)) return null;
  const parsed = BigInt(text);
  if (parsed > U64_MAX || (positive && parsed === 0n)) return null;
  return text;
}

/**
 * A datetime-local field in this flow is explicitly labelled UTC. Parse its
 * components as UTC instead of allowing Date to apply the browser timezone.
 */
export function utcInputToUnix(value) {
  const match = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})(?::(\d{2}))?$/.exec(String(value ?? ''));
  if (!match) return null;
  const [, year, month, day, hour, minute, second = '00'] = match;
  const parts = [year, month, day, hour, minute, second].map(Number);
  const milliseconds = Date.UTC(parts[0], parts[1] - 1, parts[2], parts[3], parts[4], parts[5]);
  if (!Number.isFinite(milliseconds)) return null;
  const roundTrip = new Date(milliseconds).toISOString().slice(0, 19);
  const expected = `${year}-${month}-${day}T${hour}:${minute}:${second}`;
  if (roundTrip !== expected) return null;
  const seconds = milliseconds / 1000;
  return Number.isSafeInteger(seconds) && seconds >= 0 ? String(seconds) : null;
}

export function unixToUtc(value) {
  const seconds = Number(value);
  return Number.isSafeInteger(seconds) ? new Date(seconds * 1000).toISOString().replace('.000Z', 'Z') : '';
}

export function validateMigrationDraft(draft, candidate = {}) {
  const errors = {};
  const requiredKey = (name, label) => {
    if (!String(draft[name] ?? '').trim()) errors[name] = `${label} is required.`;
    else if (!isPublicKey(String(draft[name]).trim())) errors[name] = `${label} must be a canonical Solana public key.`;
  };
  requiredKey('source_mint', 'Source token');
  requiredKey('destination_mint', 'Replacement token');
  requiredKey('mechanism_program_id', 'Mechanism program');
  if (!errors.source_mint && !errors.destination_mint && draft.source_mint.trim() === draft.destination_mint.trim()) {
    errors.destination_mint = 'Replacement token must differ from the source token.';
  }

  for (const [name, label] of [['source_decimals', 'Source decimals'], ['destination_decimals', 'Replacement decimals']]) {
    const value = canonicalU64(draft[name]);
    if (value === null || BigInt(value) > 255n) errors[name] = `${label} must be a whole number from 0 to 255.`;
  }
  for (const [name, label, positive] of [
    ['numerator', 'Ratio numerator', true],
    ['denominator', 'Ratio denominator', true],
    ['minimum_output_raw', 'Minimum output', true],
    ['reserve_raw', 'Proposed reserve', false],
  ]) {
    if (canonicalU64(draft[name], { positive }) === null) {
      errors[name] = `${label} must be a ${positive ? 'positive ' : ''}canonical raw integer no larger than u64.`;
    }
  }
  if (!['floor', 'ceiling'].includes(draft.rounding)) errors.rounding = 'Choose a supported rounding behavior.';
  if (!['none', 'source_bps'].includes(draft.fee_kind)) errors.fee_kind = 'Choose a supported fee behavior.';
  if (draft.fee_kind === 'source_bps') {
    const fee = canonicalU64(draft.fee_bps);
    if (fee === null || BigInt(fee) > 10_000n) errors.fee_bps = 'Source fee must be a whole number from 0 to 10,000 basis points.';
  }
  if (![LEGACY_TOKEN_PROGRAM, TOKEN_2022_PROGRAM].includes(draft.source_token_program)) errors.source_token_program = 'Choose a supported source token program.';
  if (![LEGACY_TOKEN_PROGRAM, TOKEN_2022_PROGRAM].includes(draft.destination_token_program)) errors.destination_token_program = 'Choose a supported replacement token program.';

  const activation = utcInputToUnix(draft.activation_utc);
  const deadline = utcInputToUnix(draft.deadline_utc);
  if (activation === null) errors.activation_utc = 'Enter a complete effective date and time in UTC.';
  if (deadline === null) errors.deadline_utc = 'Enter a complete deadline in UTC.';
  if (activation !== null && deadline !== null && BigInt(deadline) <= BigInt(activation)) {
    errors.deadline_utc = 'Migration deadline must follow the effective time.';
  }

  if (!candidate.sha256 || !/^[0-9a-f]{64}$/.test(candidate.sha256)) errors.candidate = 'Choose the exact compiled migration mechanism.';
  if (!Number.isSafeInteger(candidate.len) || candidate.len <= 0) errors.candidate = 'The migration mechanism file is empty or too large to identify safely.';
  if (!draft.state_input) errors.state_input = 'Choose the prepared state descriptor.';
  if (!draft.state_artifact) errors.state_artifact = 'Choose the prepared state artifact.';
  return errors;
}

export function buildMigrationChangeSpec(draft, candidate) {
  const errors = validateMigrationDraft(draft, candidate);
  if (Object.keys(errors).length) {
    const error = new Error('The proposal has structural errors.');
    error.fields = errors;
    throw error;
  }
  const activation = utcInputToUnix(draft.activation_utc);
  const deadline = utcInputToUnix(draft.deadline_utc);
  const fee = draft.fee_kind === 'source_bps'
    ? { kind: 'source_bps', bps: Number(draft.fee_bps) }
    : { kind: 'none' };
  return {
    schema_version: 1,
    change: {
      kind: 'token_migration',
      source: {
        mint: draft.source_mint.trim(),
        token_program: draft.source_token_program,
        decimals: Number(draft.source_decimals),
      },
      destination: {
        mint: draft.destination_mint.trim(),
        token_program: draft.destination_token_program,
        decimals: Number(draft.destination_decimals),
      },
      conversion: {
        ratio_basis: 'ui',
        numerator: draft.numerator,
        denominator: draft.denominator,
        rounding: draft.rounding,
        fee,
        minimum_output_raw: draft.minimum_output_raw,
      },
      eligibility: {
        amount_policy: 'full_balance',
        minimum_source_balance_raw: '1',
        holder_authorization: ['owner'],
        owner_authority_classes: ['wallet', 'multisig'],
        excluded_accounts: [],
      },
      source_disposition: { kind: 'burn' },
      destination_funding: {
        kind: 'reserve_transfer',
        reserve: { kind: 'proposed', funded_raw: draft.reserve_raw },
      },
      authorities: {
        migration_authority: { kind: 'program_derived' },
        expected: {},
        fee_payer: { kind: 'relayer' },
      },
      deadline: { kind: 'unix_timestamp', value: deadline },
      mechanism: {
        program_id: draft.mechanism_program_id.trim(),
        artifact: { sha256: candidate.sha256, len: candidate.len },
      },
    },
    activation: { unix_timestamp: Number(activation) },
  };
}

export function proposalDocument(spec) {
  return `${JSON.stringify(spec, null, 2)}\n`;
}

export async function digestFile(file) {
  const digest = await crypto.subtle.digest('SHA-256', await file.arrayBuffer());
  return Array.from(new Uint8Array(digest), byte => byte.toString(16).padStart(2, '0')).join('');
}

export function acceptedMigrationMismatch(accepted, spec) {
  const change = accepted?.change;
  if (!change || change.kind !== 'token_migration') return 'The server accepted a run but did not identify it as this token migration.';
  if (change.source_mint !== spec.change.source.mint || change.destination_mint !== spec.change.destination.mint) {
    return 'The server recorded different source or replacement token identities.';
  }
  if (change.candidate_sha256 !== spec.change.mechanism.artifact.sha256) return 'The server recorded different mechanism bytes.';
  if (String(change.candidate_len) !== String(spec.change.mechanism.artifact.len)) return 'The server recorded a different mechanism byte length.';
  return null;
}

export async function submitPreparedMigration({ projectId, spec, files, request }) {
  const body = new FormData();
  body.append('change_spec', new Blob([proposalDocument(spec)], { type: 'application/json' }), 'change.json');
  body.append('candidate', files.candidate, files.candidate.name || 'migration.so');
  body.append('state_input', files.state_input, files.state_input.name || 'state.json');
  body.append('state_artifact', files.state_artifact, files.state_artifact.name || 'state-artifact.json');
  body.append('analysis_options', new Blob([JSON.stringify({ policy: 'block-only' })], { type: 'application/json' }), 'analysis-options.json');
  const accepted = await request(`/v1/projects/${encodeURIComponent(projectId)}/checks`, { method: 'POST', body });
  const mismatch = acceptedMigrationMismatch(accepted, spec);
  if (mismatch) {
    const error = new Error(`${mismatch} The accepted run is ${accepted?.run_id ?? 'unknown'}; do not rely on it.`);
    error.kind = 'accepted_identity';
    throw error;
  }
  if (typeof accepted.run_id !== 'string' || !accepted.run_id) {
    const error = new Error('The server accepted the proposal without returning a run identifier.');
    error.kind = 'accepted_identity';
    throw error;
  }
  return { accepted, route: `/p/${encodeURIComponent(projectId)}/runs/${encodeURIComponent(accepted.run_id)}` };
}

export function describeMigrationSubmissionError(error) {
  if (error?.kind === 'accepted_identity') return { kind: 'accepted_identity', message: error.message };
  if (error?.status === 409) return { kind: 'readiness', message: `Project availability changed before submission: ${error.message}` };
  if (error?.status === 400 || error?.status === 413) return { kind: 'server_input', message: `Eplyx rejected the prepared inputs: ${error.message}` };
  if (error?.status === 401 || error?.status === 403) return { kind: 'authentication', message: 'Your project session no longer authorizes this submission.' };
  if (error?.status === 0) return { kind: 'job_creation', message: 'Eplyx could not be reached, so no accepted run is known.' };
  return { kind: 'job_creation', message: `Eplyx could not create the analysis run: ${error?.message ?? 'Unknown error.'}` };
}
