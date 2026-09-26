import { statusLabel, evidenceText } from './glossary.js';
// Presentation helpers for the local dashboard. Statuses arrive from engine
// artifacts through the local API; nothing here decides or recalculates one.
import { esc as escapeHTML } from './format.js';
export const esc = value => escapeHTML(evidenceText(value));

// Tone is visual semantics only. Unsupported is a technical boundary, not a
// failure, and NotTested is unknown, so neither shares the critical color.
const TONES = {
 Pass:'pos', Proven:'pos', Satisfied:'pos', Ready:'pos', Valid:'pos', Complete:'pos', Recorded:'pos', Verified:'pos',
 Warn:'warn', Incomplete:'warn', Indeterminate:'warn', PartiallyResolved:'warn', Unfinished:'warn',
 Block:'crit', Blocked:'crit', Failed:'crit', Violated:'crit', Unreadable:'crit', IdentityMismatch:'crit',
 Unsupported:'neutral', NotApplicable:'neutral', CompleteForQuery:'neutral', CompleteSyntheticFixture:'neutral',
 NotTested:'unknown',
};
const ICONS = { pos:'✓', warn:'!', crit:'✕', neutral:'–', unknown:'?' };
export const tone = status => TONES[status] || 'unknown';
export const GATE = { Pass:statusLabel('Pass'), Warn:statusLabel('Warn'), Block:statusLabel('Block') };
export const GATE_SHORT = GATE;
export const words = statusLabel;
export const sentence = value => { const text = words(value); return text.charAt(0).toUpperCase() + text.slice(1); };

export function pill(status, text, size = '') {
 const t = tone(status);
 return `<span class="pill pill--${t}${size ? ` pill--${size}` : ''}" data-status="${esc(status ?? 'Unknown')}"><i aria-hidden="true">${ICONS[t]}</i>${esc(text ?? (status ? words(status) : 'Not recorded'))}</span>`;
}
export const gatePill = (outcome, size = '') => pill(outcome, (size === 'lg' ? GATE : GATE_SHORT)[outcome] || 'Not evaluated', size);

export const isRaw = value => /^\d+$/.test(String(value ?? ''));
export const raw = value => value === null || value === undefined || value === '' ? '—' : isRaw(value) ? BigInt(value).toLocaleString('en-US') : esc(value);
export const count = value => Number.isFinite(Number(value)) && value !== null ? Number(value).toLocaleString('en-US') : '—';
export const short = (value, n = 8) => value ? `${esc(String(value).slice(0, n))}…` : '—';
export const addr = value => value ? esc(value.length > 12 ? `${value.slice(0, 4)}…${value.slice(-4)}` : value) : '—';

export const copy = (text, label = 'Copy') => `<button type="button" class="copy" data-copy="${esc(text)}" aria-label="${esc(label)} ${esc(text)}">${esc(label)}</button>`;

// Overview shows a short identity; Technical shows the full value. Both are the
// same saved value; the toggle only changes presentation.
export function ident(value, kind = 'hash') {
 if (!value) return '<span class="muted">not recorded</span>';
 const shown = kind === 'addr' ? addr(value) : short(value, kind === 'commit' ? 7 : 8);
 return `<span class="ident"><code class="ov-only" title="${esc(value)}">${shown}</code><code class="tech-only">${esc(value)}</code>${copy(value)}</span>`;
}

export function ago(timestamp) {
 const time = Date.parse(timestamp);
 if (!Number.isFinite(time)) return 'time not recorded';
 const seconds = Math.round((Date.now() - time) / 1000);
 const units = [[86400 * 365, 'year'], [86400 * 30, 'month'], [86400, 'day'], [3600, 'hour'], [60, 'minute']];
 for (const [size, unit] of units) if (Math.abs(seconds) >= size) { const n = Math.round(seconds / size); return `${n} ${unit}${n === 1 ? '' : 's'} ago`; }
 return 'just now';
}
export const when = timestamp => { const time = Date.parse(timestamp); return Number.isFinite(time) ? new Date(time).toLocaleString('en-GB', { dateStyle:'medium', timeStyle:'short' }) : '—'; };
export const utc = timestamp => { const time = Date.parse(timestamp); return Number.isFinite(time) ? `${new Date(time).toISOString().slice(0, 16).replace('T', ' ')} UTC` : '—'; };
export const exact = timestamp => timestamp ? `<time class="tech-only" datetime="${esc(timestamp)}">${esc(timestamp)}</time>` : '';

// Values are trusted HTML built by these helpers; callers escape free text.
export const kv = rows => `<dl class="kv">${rows.filter(Boolean).map(([key, value, cls = '']) => `<div class="${cls}"><dt>${esc(key)}</dt><dd>${value}</dd></div>`).join('')}</dl>`;

export function panel({ id = '', eyebrow = '', title, body, actions = '', cls = '' }) {
 return `<section class="panel ${cls}"${id ? ` id="${esc(id)}"` : ''}><header class="panel__head">${eyebrow ? `<span class="eyebrow">${esc(eyebrow)}</span>` : ''}<h2>${esc(title.replace(/[.?]$/, ""))}.</h2>${actions ? `<div class="panel__actions">${actions}</div>` : ''}</header><div class="panel__body">${body}</div></section>`;
}

export const empty = (text, command = '') => `<div class="empty"><p>${esc(text)}</p>${command ? commandLine(command) : ''}</div>`;
export const commandLine = (command, label = '') => `<div class="command">${label ? `<span class="command__label">${esc(label)}</span>` : ''}<code>${esc(command)}</code>${copy(command)}</div>`;

export function tile({ label, value, sub = '', status = '', href = '' }) {
 const body = `<span class="tile__label">${esc(label)}</span><strong class="tile__value">${value}</strong>${sub ? `<span class="tile__sub">${sub}</span>` : ''}`;
 const cls = `tile${status ? ` tile--${tone(status)}` : ''}`;
 return href ? `<a class="${cls}" href="${esc(href)}" data-link>${body}</a>` : `<div class="${cls}">${body}</div>`;
}

// A proportional status meter; every segment is labeled with its count, so
// color never carries meaning alone.
export function meter(parts, total) {
 const sum = total || parts.reduce((n, [, value]) => n + value, 0);
 if (!sum) return '<div class="meter meter--empty"><span>No selected cases</span></div>';
 return `<div class="meter" role="img" aria-label="${esc(parts.filter(([, v]) => v).map(([s, v]) => `${v} ${words(s)}`).join(', '))}">${parts.filter(([, v]) => v).map(([status, value]) => `<span class="meter__seg meter__seg--${tone(status)}" style="flex:${value}" title="${esc(`${value} ${words(status)}`)}"></span>`).join('')}</div><ul class="meter__legend">${parts.filter(([, v]) => v).map(([status, value]) => `<li>${pill(status, `${value} ${words(status)}`)}</li>`).join('')}</ul>`;
}

export const INVARIANT_NAMES = {
 conversion_output_matches:'Conversion output matches',
 no_selected_case_failed:'No selected case failed',
 required_path_available:'Required path available',
 authority_model_supported:'Authority model supported',
 no_positive_balance_stranded:'No positive balance stranded',
 candidate_binary_matches_package:'Candidate binary matches the package',
 mint_identity_matches_spec:'Mints match the specification',
 required_authorities_match:'Mint authorities match the specification',
 migration_arithmetic_matches_spec:'Migration arithmetic matches the specification',
 no_unintended_account_changes:'No unintended account changes',
 failed_migrations_roll_back:'Failed migrations roll back',
 supply_reconciles:'Supply reconciles',
 reserve_never_underflows:'Reserve never underflows',
 reserve_covers_eligible_holders:'Reserve covers every eligible holder',
 window_rules_hold:'Window rules hold',
 extension_semantics_honoured:'Token-2022 extension semantics honoured',
 no_unsupported_execution_succeeds:'No unsupported migration succeeds',
 no_stress_case_deviates:'No stress case deviates',
 all_positive_holders_migrate:'Every positive holder can migrate',
};
export const invariantName = type => INVARIANT_NAMES[type] || sentence(type);
export const SCOPES = {
 ExactCandidateConversion:'The configured source account and amount, executed once against the coherent final capture.',
 ExactOfficialTransition:'An issuer-defined official transition for the exact configured scope. Eplyx never tests this path.',
 SelectedStressCases:'The exact production accounts frozen for stress testing in this run. Other accounts were not tested.',
 SelectedAuthorityCases:'The bounded set of non-wallet accounts whose recorded authority was inspected in this run.',
 ObservedPositiveBalancePopulation:'Every positive-balance token account observed in this run’s population capture.',
 VmLoaderInput:'The exact executable bytes loaded into the local VM for this run.',
 CapturedMintState:'The source and destination mints as captured or built for this run.',
 AllExecutedMigrations:'Every migration this run executed: the population rehearsal and the frozen stress cases.',
 SequentialPopulationRehearsal:'The sequential rehearsal of the planned migratable holders over one bank and one shared reserve.',
 FrozenStressMatrix:'The stress cases frozen before execution in this run. Other states were not tested.',
 EnumeratedPositiveBalancePopulation:'Every positive-balance holder in the enumerated population of this run’s state.',
};
export const scopeText = scope => SCOPES[scope] || sentence(scope);
