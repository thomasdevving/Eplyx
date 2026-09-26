// Views of saved lifecycle/current outputs. No status is inferred from missing
// fields and no path inherits another path's execution evidence.
import { esc, words, pill, tile, panel, kv, ident, raw, count, when, empty } from './ui.js';
import { BASE } from './env.js';
const NAMES = { lifecycle_change:'Lifecycle analysis', current_observation:'Current observation', current_path:'Current path check', program_upgrade:'Program upgrade' };
const name = run => NAMES[run.kind] ?? 'Analytical record';
export const analyticalCells = run => `<span class="tag">${esc(name(run))}</span>`;
export function analyticalTiles(run) {
 return `<div class="tiles tiles--3">${tile({ label:'Analysis', value:esc(name(run)) })}${tile({ label:'Record', value:pill(run.state) })}${tile({ label:'Execution', value:run.kind === 'current_observation' || run.kind === 'lifecycle_change' ? pill('NotTested') : pill(run.status), sub:'Only the recorded scope applies.' })}</div>`;
}
const entries = value => Object.entries(value ?? {}).map(([key, value]) => [words(key), typeof value === 'object' ? `<code>${esc(JSON.stringify(value))}</code>` : esc(words(value))]);
const technicalJSON = value => { const text=JSON.stringify(value,null,2); return text.length<=65536?esc(text):`${esc(text.slice(0,65536))}\n… Preview limited to 64 KiB. Open the complete report artifact for every field.`; };
const limits = values => `<ul class="limits">${(values ?? []).map(v => `<li>${esc(v)}</li>`).join('')}</ul>`;
function lifecycle(report) {
 const impact=report.impact ?? report, summary=impact.summary ?? {};
 const entities=impact.entities ?? [], protocols=impact.protocol_observations ?? [];
 const rows=(items, protocol=false) => `<div class="table-wrap"><table class="table"><thead><tr><th>${protocol?'Protocol observation':'Entity'}</th><th>Before</th><th>After</th><th>Classification</th><th>Execution</th><th class="tech-only">Public raw balance</th></tr></thead><tbody>${items.slice(0,100).map(e => `<tr><td>${ident(e.entity_id ?? e.token_account,'addr')}</td><td>${esc(words(e.pre_lifecycle_status))}</td><td>${esc(words(e.post_lifecycle_status))}</td><td>${esc(words(e.impact_classification))}</td><td>${pill(e.execution_status)}</td><td class="tech-only">${raw(e.balance?.raw)}</td></tr>`).join('')}</tbody></table></div>${items.length>100?`<p>Showing 100 of ${count(items.length)} records. The complete output is in the report.</p>`:''}`;
 return `${panel({ id:'policy',title:'Declared policy',body:`<p>Lifecycle times change the interpretation of this pinned snapshot. They do not capture new chain state or establish an issuer transition.</p>${kv([['Before',esc(when(impact.before?.evaluated_at))],['After',esc(when(impact.after?.evaluated_at))],['Before status',esc(words(impact.before?.lifecycle_status))],['After status',esc(words(impact.after?.lifecycle_status))],['Policy basis',pill('OperatorSupplied')]])}` })}
 ${panel({ id:'consequence',title:'Entity consequences',body:`${kv([['Entities evaluated',count(summary.entities_evaluated)],['Positive public balances',count(summary.positive_balance_entities)],['Changed economic meaning',count(summary.economic_meaning_changed_entities)],['Affected public balance (raw)',raw(summary.phase2_affected_public_raw_exposure),'tech-only']])}${rows(entities)}` })}
 ${panel({ id:'protocols',title:'Protocol observations',body:`<p>These observations refer to the same entities. Their balances are not added to entity totals.</p>${protocols.length?rows(protocols,true):empty('No protocol observations recorded.')}` })}
 ${panel({ id:'limits',title:'Scope and limitations',body:limits([...(report.limitations??[]),...(impact.limitations??[])]) })}`;
}
function current(report) {
 const observation=report.kind==='current-inspection', wallet=report.wallet_observation;
 const paths=report.paths ?? (report.path ? [{path:report.path,status:report.status,reason:report.reason}] : []);
 const checks=(report.accounts??[]).flatMap(a=>(a.checks??[]).map(c=>({...c,source:a.source})));
 return `${panel({ id:'observation',title:observation?'Current observation':'Recorded check',body:`${kv([
 ['Kind',esc(words(report.kind ?? 'Declared check'))],['Mint',ident(report.asset?.mint ?? report.mint,'addr')],['Source account',ident(report.source,'addr')],['Public owner',ident(wallet?.public_owner ?? report.owner,'addr')],['Acquisition completed',esc(when(report.acquisition?.completed_at))],['Execution performed',report.execution_performed===true?'Yes, in the local VM':report.execution_performed===false?'No':'Not recorded'],['Signing possession',report.signer_possession_known===true?'Recorded by engine':'Cannot be judged'],['Scope',esc(report.scope ?? 'Only the selected mint and recorded accounts. Refresh starts untested.')],
 ])}${report.inspection?kv(entries(report.inspection)):''}${report.discovery?kv(entries(report.discovery)):''}` })}
 ${panel({ id:'paths',title:'Paths remain separate',body:`<p>Transfer, market exit, withdrawal, redemption and official transition keep separate evidence. A token transfer does not establish a market exit.</p>${paths.length?`<div class="table-wrap"><table class="table"><thead><tr><th>Path</th><th>Recorded status</th><th>Reason</th></tr></thead><tbody>${paths.map(p=>`<tr><td>${esc(words(p.path))}</td><td>${pill(p.status)}</td><td>${esc(p.reason??'')}</td></tr>`).join('')}</tbody></table></div>`:empty('No path result recorded.')}${checks.length?`<h3>Available checks.</h3>${checks.map(c=>`<p>${ident(c.source,'addr')} · ${esc(words(c.path))} · ${pill(c.status)} ${esc(c.reason??'')}</p>`).join('')}<p>Availability is not execution proof.</p>`:''}` })}
 ${wallet?panel({ id:'wallet',title:'Owner and selected mint',body:`<p>A public owner address does not prove control of its keys.</p>${kv(entries({status:wallet.status,...wallet.summary}))}<div class="table-wrap"><table class="table"><thead><tr><th>Token account</th><th>Public balance</th><th>State</th></tr></thead><tbody>${(wallet.token_accounts??[]).slice(0,100).map(a=>`<tr><td>${ident(a.address,'addr')}</td><td>${raw(a.raw_balance ?? a.amount_raw)}</td><td>${esc(words(a.state ?? a.status))}</td></tr>`).join('')}</tbody></table></div>` }):''}
 ${panel({id:'limits',title:'Scope and limitations',body:limits(report.limitations ?? [report.scope ?? 'No other account, amount, recipient, position, range, bank or path inherits this result.'])})}`;
}
export function analyticalDetail(detail,{evidenceBlock}={}) {
 const report=detail.analysis?.report ?? {};
 const content=detail.state!=='Complete'?empty(detail.problems?.join('; ')||'This run has not finished.'):detail.kind==='lifecycle_change'?lifecycle(report):current(report);
 return {title:name(detail),crumbs:[['Runs','/runs'],[`#${detail.number??'?'}`]],html:`<div class="page-head"><div><span class="eyebrow">Run #${esc(detail.number??'?')}</span><h1>${esc(name(detail))}.</h1><p class="muted">${esc(detail.claim_basis??'Saved engine output.')}</p></div></div>${analyticalTiles(detail)}<nav class="subnav" aria-label="Run sections"><a href="#${detail.kind==='lifecycle_change'?'policy':'observation'}">Input</a><a href="#${detail.kind==='lifecycle_change'?'consequence':'paths'}">Results</a><a href="#limits">Limitations</a><a href="#evidence">Evidence</a></nav>${content}${panel({id:'evidence',title:'Evidence',body:evidenceBlock?evidenceBlock(detail):kv(entries(detail.evidence?.hashes))})}${panel({cls:'tech-only',title:'Recorded JSON',body:`<pre>${technicalJSON(report)}</pre>`})}`};
}
export function analyticalOverview(detail,project,helpers) {
 const view=analyticalDetail(detail,helpers);
 return {title:'Overview',html:`${view.html}${panel({title:'Recent runs',body:helpers.runsTable(project.recent_runs??[])})}`};
}
export function analyticalCompare(c,{fieldTable}) {
 return {title:'Compare',html:`<div class="page-head"><h1>Compare analytical records.</h1><p>${esc(c.causality)}</p></div><nav class="subnav"><a href="${BASE}/runs/${esc(c.left.id)}" data-link>Run A</a><a href="${BASE}/runs/${esc(c.right.id)}" data-link>Run B</a></nav>${panel({title:'Input differences',body:fieldTable(c.inputs)})}${panel({title:'Recorded result differences',body:fieldTable(c.results)})}`};
}
