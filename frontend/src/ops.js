// Operator view of the hosted run queue, rendered from `GET /v1/ops`.
//
// Infrastructure only. A worker failure is an `execution_error`: no verdict was
// obtained, and nothing here describes a candidate.

const escapeHTML = value => String(value ?? '').replace(/[&<>"']/g, character => ({
  '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
}[character]));

const count = value => Number.isInteger(value) && value >= 0;
const seconds = value => value === null || count(value);

/**
 * @param {unknown} value
 */
export function parseOps(value) {
  const invalid = () => { throw new Error('Eplyx returned an invalid operations response.'); };
  const ops = /** @type {any} */ (value);
  if (!ops || ops.schema_version !== 1 || !count(ops.window_seconds)) invalid();
  const { workers, queue, outcomes, wait, execution, retries } = ops;
  if (!workers || !count(workers.max_concurrent_runs) || !count(workers.busy)) invalid();
  if (!queue || !count(queue.queued) || !count(queue.running)
      || !seconds(queue.oldest_queued_age_seconds ?? null)
      || !seconds(queue.longest_running_seconds ?? null)) invalid();
  for (const key of ['created', 'passed', 'failed', 'completed', 'execution_error', 'queued', 'running']) {
    if (!outcomes || !count(outcomes[key])) invalid();
  }
  for (const distribution of [wait, execution]) {
    if (!distribution || !count(distribution.count)) invalid();
    for (const key of ['p50_seconds', 'p90_seconds', 'max_seconds']) {
      if (!seconds(distribution[key] ?? null)) invalid();
    }
  }
  for (const key of ['runs_retried', 'interrupted_attempts', 'finalized_on_recovery', 'exhausted']) {
    if (!retries || !count(retries[key])) invalid();
  }
  if (!Array.isArray(ops.recent_worker_failures) || !Array.isArray(ops.recoveries)) invalid();
  return ops;
}

/** Whole seconds as a short duration; absent is a dash, never zero. */
export function duration(value) {
  if (value === null || value === undefined) return '—';
  if (value < 60) return `${value}s`;
  if (value < 3600) return `${Math.floor(value / 60)}m ${value % 60}s`;
  return `${Math.floor(value / 3600)}h ${Math.floor((value % 3600) / 60)}m`;
}

const metric = (label, value, note = '') =>
  `<article><span>${escapeHTML(label)}</span><b>${escapeHTML(value)}</b>${note ? `<small>${escapeHTML(note)}</small>` : ''}</article>`;

export function opsHTML(ops) {
  const hours = Math.round(ops.window_seconds / 3600);
  const failures = ops.recent_worker_failures.map(failure => `<li>
      <code>${escapeHTML(failure.run_id)}</code>
      <span>${escapeHTML(failure.project_id)} · ${escapeHTML(failure.attempts)} attempt(s)</span>
      <small>${escapeHTML(failure.detail ?? 'No detail recorded.')}</small>
    </li>`).join('');
  const recovery = ops.recoveries[0];
  const recoveryLine = recovery
    ? `Last start: ${recovery.requeued.length} re-enqueued, ${recovery.finalized.length} finalized from a written report, ${recovery.failed.length} given up as execution_error.`
    : 'No startup recovery has been recorded on this volume.';
  return `<div class="ops" data-queued="${ops.queue.queued}" data-execution-errors="${ops.outcomes.execution_error}">
    <div class="ops-metrics">
      ${metric('Workers busy', `${ops.workers.busy} / ${ops.workers.max_concurrent_runs}`)}
      ${metric('Queued', ops.queue.queued, ops.queue.queued ? `oldest ${duration(ops.queue.oldest_queued_age_seconds)}` : '')}
      ${metric('Running', ops.queue.running, ops.queue.running ? `longest ${duration(ops.queue.longest_running_seconds)}` : '')}
      ${metric('Wait p50 / p90', `${duration(ops.wait.p50_seconds)} / ${duration(ops.wait.p90_seconds)}`, `${ops.wait.count} run(s)`)}
      ${metric('Execution p50 / p90', `${duration(ops.execution.p50_seconds)} / ${duration(ops.execution.p90_seconds)}`, `${ops.execution.count} run(s)`)}
      ${metric('Retried runs', ops.retries.runs_retried, `${ops.retries.interrupted_attempts} interrupted attempt(s)`)}
      ${metric('Worker failures', ops.outcomes.execution_error, ops.retries.exhausted ? `${ops.retries.exhausted} after the retry limit` : '')}
    </div>
    <p class="console-note">Last ${escapeHTML(hours)}h: ${escapeHTML(ops.outcomes.created)} run(s) created — ${escapeHTML(ops.outcomes.passed)} passed, ${escapeHTML(ops.outcomes.failed)} failed, ${escapeHTML(ops.outcomes.completed)} completed, ${escapeHTML(ops.outcomes.execution_error)} without a verdict. ${escapeHTML(recoveryLine)}</p>
    ${failures ? `<h3>Recent worker failures</h3><ul class="ops-failures">${failures}</ul><p class="console-note">An execution error is an infrastructure outcome. It says nothing about the candidate.</p>` : ''}
  </div>`;
}
