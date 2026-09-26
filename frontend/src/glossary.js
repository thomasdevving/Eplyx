import labels from './glossary.json' with { type:'json' };
export const statusLabel = value => labels[value] ?? String(value ?? '').replace(/_/g, ' ').replace(/([a-z])([A-Z])/g, '$1 $2');

// Exact presentation-only wording corrections; downloaded analytical bytes stay unchanged.
export const evidenceText = value => value === "A rehearsal that succeeds for tested states does not show that every possible holder or state is safe." ? labels[value] : value;
