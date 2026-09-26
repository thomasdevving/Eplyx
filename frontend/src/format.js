import { statusLabel } from './glossary.js';
export const esc = value => String(value ?? 'Unknown').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
export const number = value => BigInt(value).toLocaleString('en-US');
export const label = statusLabel;
const badgeTone = status => ['Proven','Satisfied','Supported','Ready','Accepted'].includes(status) ? 'evidence-badge--proven'
 : ['Failed','Blocked','Contradicted'].includes(status) ? 'evidence-badge--failed'
 : ['NotTested','Incomplete','IncompleteEvidence','NotEstablished','Indeterminate','NotAccepted'].includes(status) ? 'evidence-badge--open' : '';
export const badge = status => `<span class="evidence-badge ${badgeTone(status)}">${esc(label(status))}</span>`;
