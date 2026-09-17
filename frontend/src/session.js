// Where the backend is, and what credential this tab holds.
//
// The API base URL is configuration, not something a person types into the
// product. `runtime-config.js` is written at build time from EPLYX_API_URL and
// loaded before the app, so the same bundle can be served against a local
// server or a deployed one without editing a component.
//
// The credential is the operator token described in docs/phase-10-hosted-ci.md:
// this console is the operator's, and project tokens are issued here for
// pipelines rather than used here. It lives in sessionStorage so a reload keeps
// working and closing the tab does not.

const DEFAULT_LOCAL_API = 'http://127.0.0.1:8891';
const KEY = 'eplyx-operator-token';

export const API_BASE = resolveApiBase();

function resolveApiBase() {
  const configured = globalThis.EPLYX_API_URL;
  if (typeof configured === 'string' && configured.trim()) return configured.trim().replace(/\/$/, '');
  // Unconfigured and served from a developer's machine: the local server. Served
  // from anywhere else, the API is assumed to sit behind the same origin, which
  // is wrong loudly rather than pointing at somebody's laptop quietly.
  const host = globalThis.location?.hostname ?? '';
  if (host === 'localhost' || host === '127.0.0.1') return DEFAULT_LOCAL_API;
  return '';
}

export function operatorToken() {
  try {
    return sessionStorage.getItem(KEY) || '';
  } catch {
    return '';
  }
}

export function setOperatorToken(token) {
  try {
    if (token) sessionStorage.setItem(KEY, token);
    else sessionStorage.removeItem(KEY);
  } catch {
    /* The console still works for this page view without storage. */
  }
}

export function isConnected() {
  return Boolean(operatorToken());
}

/** An API error that carries what the server actually said. */
export class ApiError extends Error {
  constructor(status, body) {
    super(body?.error || `Request failed (${status})`);
    this.status = status;
    this.body = body;
  }
}

/**
 * One request to the hosted API.
 *
 * Transport failure, authentication and a refused operation stay three
 * different things all the way to the caller: a page that renders them
 * identically cannot tell someone which of the three to fix.
 */
export async function api(path, { method = 'GET', body, headers = {} } = {}) {
  let response;
  try {
    response = await fetch(`${API_BASE}${path}`, {
      method,
      headers: { Authorization: `Bearer ${operatorToken()}`, ...headers },
      body,
    });
  } catch (error) {
    throw new ApiError(0, { error: 'Could not reach Eplyx.' });
  }
  const payload = response.status === 204 ? {} : await response.json().catch(() => ({}));
  if (!response.ok) throw new ApiError(response.status, payload);
  return payload;
}

export const json = value => ({
  body: JSON.stringify(value),
  headers: { 'Content-Type': 'application/json' },
});

export const short = (hash, length = 10) =>
  typeof hash === 'string' && hash.length > length ? `${hash.slice(0, length)}…` : hash ?? '—';

export function when(seconds) {
  if (!seconds) return '—';
  const date = new Date(seconds * 1000);
  return date.toLocaleString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  });
}
