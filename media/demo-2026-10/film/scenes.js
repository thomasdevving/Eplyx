// The storyboard. Every terminal and browser pixel comes from a recording made
// against the real binaries and services (see ../README.md). Overlays only
// label, frame and point at what those recordings show.
import { h, svg, put, seg, ease, keys, pw, clamp, lerp, splitWords, riseWords, fmt } from './core.js';
import { titleBlock, termWindow, webWindow, plan, enter, badge, pop, note, slide, pills, staggerIn, termCam, mark } from './kit.js';
import { Footage, windowFrame } from './components.js';

export const chapters = ['Open', 'Website', 'Upgrades', 'Hosted', 'Governance & history', 'Token transitions', 'Parameters', 'Share & integrate', 'Close'];
const CH = {
  web: [1, 'Public website'], cli: [2, 'Local CLI'], dash: [3, 'Local dashboard'], gate: [4, 'CI gate · offline'],
  hosted: [5, 'CI gate · hosted'], cloud: [6, 'Cloud workspace'], ops: [7, 'Operator console'], api: [8, 'HTTP API'],
};
const UC = {
  upgrade: [1, 'Program upgrade'], declare: [2, 'Declared changes'], gov: [3, 'Governance binding'], mig: [4, 'Token migration'],
  life: [5, 'Lifecycle change'], cur: [6, 'Current state'], fee: [7, 'Fee parameter'], inter: [8, 'Upgrade × fee'],
  hist: [9, 'Historical research'], share: [10, 'Share & manage'],
};
const live = (root, x, y, text = 'LIVE · SOLANA MAINNET') => { const el = h('div', { class: 'abs live', style: { left: x + 'px', top: y + 'px' } }, h('i'), text); root.append(el); return el; };
const tag = (root, x, y, text) => { const el = h('div', { class: 'abs srctag', style: { left: x + 'px', top: y + 'px' } }, text); root.append(el); return el; };
const typing = (at, steps) => at.map((a, i) => ({ t: a.typeStart, type: 'typing', dur: a.runAt - a.typeStart })).filter(c => c.dur > .05);

export const scenes = [
  // ── 0 ── Open: the real homepage intro, then the promise ──────────────────
  { id: 'intro', dur: 11, chapter: 0, hud: false, bg: 'ink', sweep: false,
    async build(root) {
      const W0 = 1920, H0 = 1200;
      const frame = windowFrame({ kind: 'browser', url: 'eplyx · Know what changes', w: W0, h: H0 + 44 });
      const foot = new Footage({ clip: 'hero-full', w: W0, h: H0 }); await foot.load();
      frame.body.append(foot.root); frame.root.style.left = '0px'; frame.root.style.top = '0px'; frame.root.style.transformOrigin = '0 0';
      root.append(frame.root);
      const tb = titleBlock(root, { x: 110, y: 250, w: 760, kicker: 'Onchain change intelligence for Solana', title: 'Know what changes.\n*See who is affected.', size: 'md',
        sub: 'Eplyx runs real SBF bytecode against captured state — deterministically and offline — and reports what a change does to real users and funds.' });
      const nos = pills(root, { x: 110, y: 760, items: [{ text: 'Never signs' }, { text: 'Never sends a transaction' }, { text: 'Never says “safe”' }] });
      const ct = lt => Math.min(10.95, lt * .96 + .05);
      return {
        prepare: lt => foot.prepare(ct(lt)),
        update(lt) {
          foot.update(ct(lt), { pointer: false });
          const p = seg(lt, 5.3, 6.6, ease.inOutExpo);
          const s = lerp(1, .5, p), x = lerp(0, 900, p), y = lerp(-104 - 60, 255, p);
          frame.root.style.transform = `translate(${x}px,${y}px) scale(${s})`;
          frame.root.style.borderRadius = lerp(0, 30, p) + 'px';
          frame.root.style.boxShadow = p > 0 ? '' : 'none';
          tb.update(lt, { at: 6.3 });
          staggerIn(nos, lt, 8.6, { step: .22 });
        },
        cues: () => [{ t: 5.3, type: 'whoosh' }, { t: 8.6, type: 'tick' }, { t: 8.82, type: 'tick' }, { t: 9.04, type: 'tick' }],
      };
    } },

  // ── 0 ── Eight channels around one engine ─────────────────────────────────
  { id: 'channels', dur: 5.5, chapter: 0, bg: 'violet',
    async build(root) {
      const tb = titleBlock(root, { x: 0, y: 118, w: 1920, title: 'One engine. *Eight ways in.', size: 'sm', align: 'center' });
      const cx = 960, cy = 590;
      const lines = svg('svg', { width: 1920, height: 1080, class: 'abs', style: 'left:0;top:0' });
      root.append(lines);
      const core = h('div', { class: 'abs core', html: window.markSVG('core__mark'), style: { left: cx - 110 + 'px', top: cy - 110 + 'px' } });
      root.append(core);
      const items = [
        ['01', 'Public website', 'demo report · guides'], ['02', 'Local CLI', 'every analysis, offline'], ['03', 'Local dashboard', 'read-only, loopback'],
        ['04', 'CI gate · offline', 'exit codes 0–5'], ['05', 'CI gate · hosted', 'submit + PR comment'], ['06', 'Cloud workspace', 'forms · history · sync'],
        ['07', 'Operator console', 'bundles · tokens · ops'], ['08', 'HTTP API', '/checks · /runs · /setup'],
      ];
      const cards = items.map(([n, t, s], i) => {
        const a = -Math.PI / 2 + i * Math.PI / 4, x = cx + Math.cos(a) * 660, y = cy + Math.sin(a) * 300;
        const el = h('div', { class: 'abs chcard', style: { left: x - 150 + 'px', top: y - 42 + 'px' } }, h('b', {}, n), h('div', {}, h('strong', {}, t), h('span', {}, s)));
        root.append(el);
        const ln = svg('line', { x1: cx, y1: cy, x2: x, y2: y, stroke: 'url(#lg)', 'stroke-width': 1.5 });
        lines.append(ln);
        const len = Math.hypot(x - cx, y - cy); ln.style.strokeDasharray = len; ln.style.strokeDashoffset = len;
        return { el, ln, len };
      });
      lines.prepend(svg('defs', {}, svg('linearGradient', { id: 'lg' }, svg('stop', { offset: '0', 'stop-color': '#c4b5fd', 'stop-opacity': '.1' }), svg('stop', { offset: '1', 'stop-color': '#c4b5fd', 'stop-opacity': '.7' }))));
      const ten = h('div', { class: 'abs tenrow' }); root.append(ten);
      const qs = Object.values(UC).map(([n, t]) => { const e = h('span', {}, h('b', {}, String(n).padStart(2, '0')), t); ten.append(e); return e; });
      return {
        update(lt) {
          tb.update(lt, { at: .05 });
          const cp = seg(lt, .1, .9, ease.outBack); put(core, { s: lerp(.4, 1, cp) * (1 + .02 * Math.sin(lt * 2)), o: clamp(cp * 1.5) });
          cards.forEach((c, i) => {
            const t0 = .5 + i * .12;
            c.ln.style.strokeDashoffset = c.len * (1 - seg(lt, t0, t0 + .5, ease.out));
            pop(c.el, lt, t0 + .2, { from: .7 });
          });
          staggerIn(qs, lt, 2.9, { step: .07, dy: 14 });
        },
        cues: () => [{ t: .1, type: 'boom' }, ...items.map((_, i) => ({ t: .7 + i * .12, type: 'tick' }))],
      };
    } },

  // ── 1 ── Channel 1: the public website ───────────────────────────────────
  { id: 'website', dur: 8.5, chapter: 1, channel: CH.web, bg: 'violet', caption: 'Recorded from the local public site · no account · nothing executes',
    async build(root) {
      const A = await webWindow(root, { clip: 'start', x: 80, y: 150, w: 1240, url: '127.0.0.1:4173/start' });
      const B = await webWindow(root, { clip: 'demo', x: 80, y: 150, w: 1240, url: '127.0.0.1:4173/runs/demo' });
      const tb = titleBlock(root, { x: 1380, y: 210, w: 470, kicker: 'No account needed', title: 'Start with\n*your question.', size: 'sm',
        sub: 'Six questions, each with what to bring and what you get back. A saved regression report opens without installing anything.' });
      const ca = lt => pw(lt, [[0, 1.6], [5.0, 10.9]]), cb = lt => pw(lt, [[4.8, .9], [8.5, 7.7]]);
      return {
        async prepare(lt) { await A.foot.prepare(ca(lt)); if (lt > 4.4) await B.foot.prepare(cb(lt)); },
        update(lt) {
          enter(A.win, lt, 0, { out: 4.8 });
          A.foot.update(ca(lt), { cam: { z: 1.08, cx: 640, cy: 470 } });
          enter(B.win, lt, 4.75, { dy: 80 });
          B.foot.update(cb(lt), { cam: keys(lt, [[4.8, { z: 1, cx: 720, cy: 450 }], [8.5, { z: 1.25, cx: 640, cy: 420 }]]), pointer: false });
          tb.update(lt, { at: .4 });
        },
        cues: () => [{ t: .1, type: 'whoosh' }, { t: 4.75, type: 'whoosh' }],
      };
    } },

  // ── 2 ── How a check works ────────────────────────────────────────────────
  { id: 'concept', dur: 4.5, chapter: 2, channel: CH.cli, usecase: UC.upgrade, bg: 'ink',
    async build(root) {
      const tb = titleBlock(root, { x: 0, y: 130, w: 1920, title: 'Same transaction. Same state. *Two builds.', size: 'sm', align: 'center' });
      const tx = h('div', { class: 'abs dnode dnode--tx', style: { left: '150px', top: '470px' } }, h('small', {}, 'Mainnet transaction · slot 447,850,493'), h('strong', {}, 'DepositSol'), h('code', {}, '313DzT…3UH5F'));
      const v1 = h('div', { class: 'abs dnode', style: { left: '700px', top: '330px' } }, h('small', {}, 'V1 · the binary that ran'), h('strong', {}, 'Baseline'), h('code', {}, 'ec2dfefa…d7e1'));
      const v2 = h('div', { class: 'abs dnode dnode--cand', style: { left: '700px', top: '610px' } }, h('small', {}, 'V2 · your candidate'), h('strong', {}, 'Candidate'), h('code', {}, '3193eabd…b099'));
      const df = h('div', { class: 'abs dnode dnode--diff', style: { left: '1250px', top: '470px' } }, h('small', {}, 'Compare'), h('strong', {}, 'State · balances · CPIs'), h('code', {}, '→ named economic findings'));
      root.append(tx, v1, v2, df);
      const s = svg('svg', { width: 1920, height: 1080, class: 'abs', style: 'left:0;top:0' }); root.prepend(s);
      const path = (d) => { const p = svg('path', { d, fill: 'none', stroke: '#a78bfa', 'stroke-width': 2, 'stroke-opacity': .7 }); s.append(p); const L = p.getTotalLength(); p.style.strokeDasharray = L; p.style.strokeDashoffset = L; return { p, L }; };
      const P = [path('M560 540 C 630 540 620 400 700 400'), path('M560 540 C 630 540 620 680 700 680'), path('M1110 400 C 1180 400 1170 540 1250 540'), path('M1110 680 C 1180 680 1170 540 1250 540')];
      const dots = P.map(() => { const d = svg('circle', { r: 6, fill: '#e7ddff' }); s.append(d); return d; });
      return {
        update(lt) {
          tb.update(lt, { at: .05 });
          pop(tx, lt, .25); pop(v1, lt, .9); pop(v2, lt, 1.0); pop(df, lt, 2.1);
          P.forEach((q, i) => { const a = i < 2 ? .6 : 1.7; q.p.style.strokeDashoffset = q.L * (1 - seg(lt, a, a + .5, ease.out)); const k = ((lt - a) * .7) % 1; const pt = q.p.getPointAtLength(q.L * clamp(k)); dots[i].setAttribute('cx', pt.x); dots[i].setAttribute('cy', pt.y); dots[i].style.opacity = lt > a + .5 ? .9 : 0; });
        },
        cues: () => [{ t: .25, type: 'tick' }, { t: .9, type: 'tick' }, { t: 1.0, type: 'tick' }, { t: 2.1, type: 'tick' }],
      };
    } },

  // ── 2 ── UC1: the upgrade gate, locally ───────────────────────────────────
  { id: 'upgrade', dur: 11, chapter: 2, channel: CH.cli, usecase: UC.upgrade, bg: 'violet', caption: 'Real PTY recording · committed SPL Stake Pool bundle · candidate = constructed regression fixture, not an upstream release',
    async build(root) {
      const T = await termWindow(root, { cast: 'upgrade', x: 690, y: 140, w: 1150, cols: 100, rows: 30, title: 'eplyx — ~/stake-pool-program' });
      const tb = titleBlock(root, { x: 96, y: 200, w: 560, kicker: 'Program upgrade', title: 'Does my build\n*change real behaviour?', size: 'md',
        sub: 'Ten validated mainnet observations, the pinned baseline that ran them, and your candidate — replayed offline.' });
      const P = plan(T.term, .3, [{ type: .9, out: .45, hold: 1.2 }, { type: .95, out: .25, hold: 1.4 }, { type: .95, out: .3, hold: 3.2 }]);
      const ok = badge(root, { x: 96, y: 720, label: 'GATE: PASSED', sub: 'control · exit 0', tone: 'green' });
      const bad = badge(root, { x: 96, y: 720, label: 'GATE: FAILED', sub: 'candidate · exit 1', tone: 'red' });
      const mk1 = mark(T.term, T.term.overlay, 'now_reverts', { tone: 'red' }), mk2 = mark(T.term, T.term.overlay, 'pool_tokens_received/decreased', { tone: 'red' });
      const a = P.at;
      return {
        prepare: lt => T.term.prepare(P.map(lt)),
        update(lt) {
          enter(T.win, lt, 0);
          tb.update(lt, { at: .2 });
          T.term.update({ cam: termCam(T.term, lt, [
            { a: a[0].outEnd, b: a[0].holdEnd - .2, text: 'records:', z: 1.45 },
            { a: a[1].outEnd, b: a[1].holdEnd - .1, text: 'GATE: PASSED', z: 1.55, dy: -60 },
            { a: a[2].outEnd + .1, b: 99, text: 'FINDINGS', z: 1.5, dy: 120 },
          ]) });
          pop(ok, lt, a[1].outEnd + .1, { out: a[1].holdEnd });
          pop(bad, lt, a[2].outEnd + .5);
          mk1(seg(lt, a[2].outEnd + .9, a[2].outEnd + 1.3)); mk2(seg(lt, a[2].outEnd + 1.2, a[2].outEnd + 1.6));
        },
        cues: () => [...typing(a), { t: a[1].outEnd + .1, type: 'pass' }, { t: a[2].outEnd + .5, type: 'fail' }],
      };
    } },

  // ── 2 ── UC2: declared, bounded expectations ─────────────────────────────
  { id: 'expectations', dur: 7.5, chapter: 2, channel: CH.cli, usecase: UC.declare, bg: 'violet', caption: 'Declarations: docs/pilot/expected-changes.bounded.toml · real recording',
    async build(root) {
      const T = await termWindow(root, { cast: 'expectations', x: 80, y: 140, w: 1100, cols: 100, rows: 30, title: 'eplyx — ~/stake-pool-program' });
      const tb = titleBlock(root, { x: 1250, y: 190, w: 600, kicker: 'expected-changes.toml', title: 'Declare intended\n*changes narrowly.', size: 'sm',
        sub: 'Named, bounded findings become expected. A change nothing can name still fails the gate.' });
      const chips = pills(root, { x: 1250, y: 620, wrap: 600, items: [{ text: 'expected', tone: 'green' }, { text: 'unexpected', tone: 'red' }, { text: 'exceeded', tone: 'orange' }, { text: 'stale' }, { text: 'unevaluable' }] });
      const P = plan(T.term, .25, [{ type: .8, out: .3, hold: 1.5 }, { type: 1.0, out: .3, hold: 3 }]);
      const a = P.at;
      const m1 = mark(T.term, T.term.overlay, 'expected                       2', { tone: 'green' });
      const m2 = mark(T.term, T.term.overlay, 'GATE: FAILED (undeclarable_change)', { tone: 'red' });
      const bad = badge(root, { x: 1250, y: 760, label: 'undeclarable_change', sub: 'still exit 1', tone: 'red' });
      return {
        prepare: lt => T.term.prepare(P.map(lt)),
        update(lt) {
          enter(T.win, lt, 0); tb.update(lt, { at: .2 }); staggerIn(chips, lt, .9, { step: .09 });
          T.term.update({ cam: termCam(T.term, lt, [{ a: a[0].outEnd, b: a[0].holdEnd - .1, text: 'max_delta_bps', z: 1.35 }, { a: a[1].outEnd, b: 99, text: 'RESULTS', z: 1.4, dy: 130 }]) });
          m1(seg(lt, a[1].outEnd + .6, a[1].outEnd + 1)); m2(seg(lt, a[1].outEnd + 1.3, a[1].outEnd + 1.7));
          pop(bad, lt, a[1].outEnd + 1.6);
          chips.forEach((c, i) => { c.style.outline = (i === 0 && lt > a[1].outEnd + .6) ? '2px solid #6fe3b4' : 'none'; });
        },
        cues: () => [...typing(a), { t: a[1].outEnd + 1.6, type: 'fail' }],
      };
    } },

  // ── 2 ── Channel 4: the offline CI gate ──────────────────────────────────
  { id: 'gate', dur: 5, chapter: 2, channel: CH.gate, usecase: UC.upgrade, bg: 'ink', caption: 'Excerpt: .github/workflows/reproducibility.yml in this repository',
    async build(root) {
      const tb = titleBlock(root, { x: 96, y: 170, w: 760, kicker: 'CI gate · offline', title: 'No service. No endpoint.\n*The exit code is the gate.', size: 'sm' });
      const codes = [['0', 'passed', 'green'], ['1', 'undeclared or out-of-bounds change', 'red'], ['2', 'configuration or fidelity error'], ['3', 'stale declaration', 'orange'], ['4', 'bundle / baseline incompatible'], ['5', 'unevaluable declaration']];
      const tiles = codes.map(([c, t, tone], i) => { const el = h('div', { class: `abs codetile ${tone ? 'codetile--' + tone : ''}`, style: { left: 96 + (i % 3) * 262 + 'px', top: 500 + Math.floor(i / 3) * 180 + 'px' } }, h('b', {}, c), h('span', {}, t)); root.append(el); return el; });
      const yaml = h('pre', { class: 'abs yaml', html: `<i># .github/workflows/reproducibility.yml (excerpt)</i>
<b>- name:</b> Run expected regression
  <b>run:</b> |
    target/debug/eplyx ci check \\
      --bundle deploy/bundle \\
      --candidate artifacts/fixture_stake_pool_v2.so \\
      --format json --out regression.json
    code=$?
    echo "regression exit code: $code (expected 1)"
    test "$code" -eq 1` });
      root.append(yaml);
      return {
        update(lt) { tb.update(lt, { at: .05 }); tiles.forEach((el, i) => pop(el, lt, .5 + i * .1)); enter(yaml, lt, .4, { dy: 40, rx: 6 }); },
        cues: () => codes.map((_, i) => ({ t: .5 + i * .1, type: 'tick' })),
      };
    } },

  // ── 3 ── Channel 7: the operator pins the baseline ───────────────────────
  { id: 'operator', dur: 8.5, chapter: 3, channel: CH.ops, usecase: UC.share, bg: 'violet', caption: 'eplyx-server admin on the service volume · operator console at /projects',
    async build(root) {
      const T = await termWindow(root, { cast: 'operator', x: 70, y: 300, w: 930, cols: 100, rows: 21, title: 'operator — eplyx-server admin' });
      const B = await webWindow(root, { clip: 'console', x: 1040, y: 230, w: 820, url: '127.0.0.1:4173/projects' });
      const tb = titleBlock(root, { x: 80, y: 120, w: 1000, kicker: 'Operator console', title: 'The operator pins the baseline.\n*A CI token can never move it.', size: 'sm' });
      const P = plan(T.term, .35, [{ skip: true }, { type: .7, out: .3, hold: .8 }, { type: .7, out: .25, hold: .8 }, { type: .7, out: .2, hold: 1.4 }]);
      const ct = lt => pw(lt, [[0, 2.4], [1.5, 3.7], [4.2, 6.1], [8.5, 8.25]]);
      return {
        async prepare(lt) { await T.term.prepare(P.map(lt)); await B.foot.prepare(ct(lt)); },
        update(lt) {
          tb.update(lt, { at: 0 }); enter(T.win, lt, .1); enter(B.win, lt, .9, { dy: 70 });
          T.term.update(); B.foot.update(ct(lt), { cam: keys(lt, [[0, { z: 1, cx: 720, cy: 450 }], [5.6, { z: 1, cx: 720, cy: 450 }], [7.2, { z: 1.45, cx: 520, cy: 520 }]]) });
        },
        cues: () => typing(P.at),
      };
    } },

  // ── 3 ── Channel 5: the hosted gate from CI ──────────────────────────────
  { id: 'hosted', dur: 9.5, chapter: 3, channel: CH.hosted, usecase: UC.upgrade, bg: 'violet', caption: 'scripts/eplyx-submit.sh → local eplyx-server → the same engine · PR comment body composed locally, nothing posted to GitHub',
    async build(root) {
      const T = await termWindow(root, { cast: 'hosted', x: 70, y: 330, w: 900, cols: 100, rows: 18, title: 'GitHub Actions step — eplyx-submit.sh' });
      const B = await webWindow(root, { clip: 'hosted', x: 1010, y: 150, w: 850, url: '127.0.0.1:4390/p/stake-pool-upgrades/runs/run_01M4B1V5…' });
      const tb = titleBlock(root, { x: 80, y: 120, w: 920, kicker: 'CI gate · hosted', title: 'Upload the build.\n*Get the same verdict.', size: 'sm' });
      const P = plan(T.term, .3, [{ type: .5, out: .1, hold: .2 }, { type: 1.1, out: 1.0, hold: 3.5 }]);
      const ct = lt => pw(lt, [[0, 3.0], [2.6, 3.62], [5.6, 8.5], [9.5, 11.6]]);
      const bad = badge(root, { x: 560, y: 800, label: 'status failed · exit 1', sub: 'HTTP 202 → worker → verdict', tone: 'red' });
      // The pull-request comment body, composed by scripts/eplyx-pr-comment.py's
      // own compose() from the summary the submit client wrote (not posted).
      const pr = h('div', { class: 'abs prcard', style: { left: '70px', top: '520px' } });
      pr.innerHTML = `<header><svg class="prcard__icon" width="18" height="18" viewBox="0 0 16 16"><path fill="currentColor" d="M2 2.5A1.5 1.5 0 0 1 3.5 1h9A1.5 1.5 0 0 1 14 2.5v7a1.5 1.5 0 0 1-1.5 1.5H8.2l-3.1 2.6c-.5.4-1.1 0-1.1-.5V11h-.5A1.5 1.5 0 0 1 2 9.5z"/></svg> Pull request comment <em>composed by scripts/eplyx-pr-comment.py</em></header>
        <p><strong>Unexpected change.</strong> Replaying production transactions against this candidate produced semantic changes that <code>expected-changes.toml</code> does not declare, or that exceed their declared bounds.</p>
        <h5>Eplyx — Analytical regression</h5>
        <table><tr><td>Candidate</td><td><code>3193eabd9fe2e479…b099</code></td></tr><tr><td>Bundle</td><td><code>5e5b67ac13e4f6b8…285f</code></td></tr><tr><td>Records</td><td><code>10</code></td></tr><tr><td>Exit code</td><td><code>1</code></td></tr><tr><td>Run status</td><td><code>failed</code></td></tr></table>`;
      root.append(pr);
      return {
        async prepare(lt) { await T.term.prepare(P.map(lt)); await B.foot.prepare(ct(lt)); },
        update(lt) {
          tb.update(lt, { at: 0 }); enter(T.win, lt, .1); enter(B.win, lt, 1.6, { dy: 70 });
          T.term.update(); B.foot.update(ct(lt));
          pop(bad, lt, P.at[1].outEnd + .2, { out: 5.5 });
          enter(pr, lt, 5.6, { dy: 50, rx: 6 });
        },
        cues: () => [...typing(P.at), { t: P.at[1].outEnd + .2, type: 'fail' }, { t: 5.6, type: 'whoosh' }],
      };
    } },

  // ── 4 ── UC3: Squads governance binding, live ────────────────────────────
  { id: 'governance', dur: 8, chapter: 4, channel: CH.cli, usecase: UC.gov, bg: 'ink', caption: 'Live read of a real Squads V4 proposal via the public mainnet RPC · read-only · never signs',
    async build(root) {
      const T = await termWindow(root, { cast: 'governance', x: 70, y: 150, w: 1100, cols: 100, rows: 30, title: 'eplyx — ~/squads' });
      const tb = titleBlock(root, { x: 1240, y: 200, w: 620, kicker: 'Squads V4 · before you sign', title: 'Does the proposal\n*hold the bytes we analysed?', size: 'sm' });
      const lv = live(root, 1240, 160);
      const eq = h('div', { class: 'abs eqcard', style: { left: '1240px', top: '520px' } },
        h('div', {}, h('small', {}, 'Analysed candidate'), h('code', {}, 'b10c1c93…0761d')), h('b', {}, '='), h('div', {}, h('small', {}, 'Squads buffer ELF'), h('code', {}, 'b10c1c93…0761d')));
      root.append(eq);
      const n = note(root, { x: 1240, y: 720, w: 600, title: 'A match is a statement at a slot.', body: 'The buffer authority is the vault, so the bytes can only change through another vault transaction. Re-verify immediately before approving.' });
      const P = plan(T.term, .3, [{ type: 1.2, out: .5, hold: 5 }]);
      const m1 = mark(T.term, T.term.overlay, 'Result:     matched', { tone: 'green' });
      const m2 = mark(T.term, T.term.overlay, 'Observed:', { tone: 'violet', span: 40 });
      return {
        prepare: lt => T.term.prepare(P.map(lt)),
        update(lt) {
          enter(T.win, lt, 0); tb.update(lt, { at: .2 }); slide(lv, lt, .3, { dx: 0, dy: -10 });
          const a = P.at[0];
          T.term.update({ cam: termCam(T.term, lt, [{ a: a.outEnd + .1, b: 99, text: 'Result:', z: 1.35, dy: 110, dx: 300 }]) });
          m1(seg(lt, a.outEnd + .5, a.outEnd + .9)); m2(seg(lt, a.outEnd + .8, a.outEnd + 1.2));
          pop(eq, lt, a.outEnd + 1.2); slide(n, lt, a.outEnd + 1.8, { dx: 30 });
        },
        cues: () => [...typing(P.at), { t: P.at[0].outEnd + .5, type: 'pass' }],
      };
    } },

  // ── 4 ── UC9: historical research, live archive ──────────────────────────
  { id: 'history', dur: 10.5, chapter: 4, channel: CH.cli, usecase: UC.hist, bg: 'ink', caption: 'Live archive RPC · real PYUSD TransferChecked · real Token-2022 upgrade · the last replay uses a constructed regression fixture',
    async build(root) {
      const T = await termWindow(root, { cast: 'history', x: 70, y: 300, w: 1100, cols: 100, rows: 26, title: 'eplyx — ~/history' });
      const tb = titleBlock(root, { x: 80, y: 115, w: 1100, kicker: 'Historical research', title: 'Which version ran at slot X —\n*and does a replay reproduce it exactly?', size: 'sm' });
      const lv = live(root, 1240, 300, 'LIVE · ARCHIVE RPC');
      // slot timeline
      const tl = h('div', { class: 'abs timeline', style: { left: '1240px', top: '360px', width: '600px' } });
      tl.innerHTML = `<div class="tl__axis"></div><div class="tl__mk tl__mk--tx" style="left:71.4%"><b>427,146,982</b><span>PYUSD transfer</span></div><div class="tl__mk tl__mk--up" style="left:71.5%"><b>427,147,035</b><span>Token-2022 upgrade</span></div><div class="tl__end tl__end--a">420,000,000</div><div class="tl__end tl__end--b">430,000,000</div>`;
      root.append(tl);
      const steps = ['versions upgrades — bisect deployment slots', 'historical acquire — exact state at S−1 and S', 'versions resolve — the binary live after', 'compare — real V1 vs real V2', 'compare — real V1 vs a regression'].map((s, i) => { const el = h('div', { class: 'abs step', style: { left: '1240px', top: 560 + i * 62 + 'px' } }, h('b', {}, String(i + 1)), s); root.append(el); return el; });
      const P = plan(T.term, .3, [{ type: .9, out: .35, hold: .9 }, { type: 1.0, out: .35, hold: 1.0 }, { type: .8, out: .3, hold: .9 }, { type: .7, out: .25, hold: 1.3 }, { type: .8, out: .25, hold: 2.0 }]);
      const a = P.at;
      const m1 = mark(T.term, T.term.overlay, 'fidelity Matched', { tone: 'green' });
      const m2 = mark(T.term, T.term.overlay, 'delta -0.001000', { tone: 'red' });
      return {
        prepare: lt => T.term.prepare(P.map(lt)),
        update(lt) {
          enter(T.win, lt, 0); tb.update(lt, { at: .1 }); slide(lv, lt, .2, { dx: 0, dy: -10 });
          put(tl, { o: seg(lt, .4, .9) });
          tl.querySelector('.tl__mk--up').style.opacity = seg(lt, a[0].outEnd, a[0].outEnd + .3);
          tl.querySelector('.tl__mk--tx').style.opacity = seg(lt, a[1].outEnd, a[1].outEnd + .3);
          steps.forEach((el, i) => { slide(el, lt, .5 + i * .08, { dx: 20 }); el.classList.toggle('step--done', lt > a[i].outEnd); el.classList.toggle('step--on', lt > a[i].typeStart && lt <= a[i].outEnd); });
          T.term.update({ cam: termCam(T.term, lt, [{ a: a[3].outEnd, b: a[3].holdEnd, text: 'fidelity Matched', z: 1.3, dx: 200 }, { a: a[4].outEnd, b: 99, text: 'delta -0.001000', z: 1.5, dx: 200, dy: -40 }]) });
          m1(seg(lt, a[3].outEnd + .3, a[3].outEnd + .6) * (1 - seg(lt, a[4].typeStart, a[4].typeStart + .2))); m2(seg(lt, a[4].outEnd + .5, a[4].outEnd + .9));
        },
        cues: () => [...typing(a), { t: a[4].outEnd + .5, type: 'fail' }],
      };
    } },

  // ── 5 ── UC4: rehearse a token migration ─────────────────────────────────
  { id: 'migration', dur: 10, chapter: 5, channel: CH.cli, usecase: UC.mig, bg: 'violet', caption: 'Synthetic fixture world (examples/migrations/minimal) · reference build, then a known deadline-defect build',
    async build(root) {
      const T = await termWindow(root, { cast: 'migration', x: 700, y: 140, w: 1140, cols: 100, rows: 30, title: 'eplyx — ~/token-migration' });
      const tb = titleBlock(root, { x: 96, y: 170, w: 560, kicker: 'Token migration', title: 'Can it account for\n*every holder and fund?', size: 'sm' });
      const axes = ['mechanism', 'funding', 'population', 'reconciliation'].map((k, i) => { const el = h('div', { class: 'abs axis', style: { left: '96px', top: 470 + i * 78 + 'px' } }, h('span', {}, k), h('b', {}, '')); root.append(el); return el; });
      const P = plan(T.term, .25, [{ type: .35, out: .2, hold: .5 }, { type: .45, out: .2, hold: 1.1 }, { type: .5, out: .1, hold: .25 }, { type: .45, out: .25, hold: 1.0 }, { type: .6, out: .2, hold: .7 }, { type: .5, out: .05, hold: .1 }, { type: .5, out: .2, hold: 1.0 }, { type: .5, out: .15, hold: 1.2 }]);
      const a = P.at;
      const cx = badge(root, { x: 96, y: 800, label: 'Counterexample found', sub: 'derived · reproduced', tone: 'red' });
      const st = (el, text, tone) => { const b = el.querySelector('b'); if (b.textContent !== text) b.textContent = text; el.className = 'abs axis axis--' + tone; };
      return {
        prepare: lt => T.term.prepare(P.map(lt)),
        update(lt) {
          enter(T.win, lt, 0); tb.update(lt, { at: .1 });
          axes.forEach((el, i) => slide(el, lt, .6 + i * .08, { dx: -20 }));
          const after = lt > a[6].outEnd, done = lt > a[1].outEnd;
          const vals = !done ? [['—', ''], ['—', ''], ['—', ''], ['—', '']] : after ? [['Blocked', 'red'], ['Ready', 'green'], ['Incomplete', 'orange'], ['Ready', 'green']] : [['Ready', 'green'], ['Ready', 'green'], ['Incomplete', 'orange'], ['Ready', 'green']];
          axes.forEach((el, i) => st(el, vals[i][0], vals[i][1]));
          pop(cx, lt, a[3].outEnd + .2);
          T.term.update({ cam: termCam(T.term, lt, [{ a: a[1].outEnd, b: a[1].holdEnd, text: 'mechanism:', z: 1.4 }, { a: a[3].outEnd, b: a[4].holdEnd, text: 'UnexpectedSuccess', z: 1.4 }, { a: a[6].outEnd, b: a[6].holdEnd + .2, text: 'mechanism: Blocked', z: 1.35 }, { a: a[7].outEnd, b: 99, text: 'UNSIGNED', z: 1.45 }]) });
        },
        cues: () => [...typing(a), { t: a[3].outEnd + .2, type: 'fail' }],
      };
    } },

  // ── 5 ── Channel 3: the local dashboard ──────────────────────────────────
  { id: 'dashboard', dur: 8.5, chapter: 5, channel: CH.dash, usecase: UC.mig, bg: 'violet', caption: 'eplyx dashboard on loopback over the saved .eplyx/ records · executes nothing',
    async build(root) {
      const A = await webWindow(root, { clip: 'dashboard', x: 180, y: 140, w: 1340, url: '127.0.0.1:4185/runs/run_…c43467360081' });
      const B = await webWindow(root, { clip: 'dashboard2', x: 180, y: 140, w: 1340, url: '127.0.0.1:4185/compare' });
      const n1 = note(root, { x: 1430, y: 640, w: 420, kicker: 'Stress matrix', title: '19 of 20 cases as specified', body: 'At the deadline, rejection was required — the defect build migrated anyway.' });
      const n2 = note(root, { x: 1430, y: 640, w: 420, kicker: 'Compare runs', title: 'Ready → Blocked', body: 'Reference vs defect build: 20/20 → 19/20 stress cases, 1 → 4 invariants violated.' });
      const ca = lt => pw(lt, [[0, 5.0], [.6, 6.1], [4.4, 12.4]]), cb = lt => pw(lt, [[4.25, 9.0], [5.7, 11.25], [5.72, 12.29], [8.5, 15.5]]);
      return {
        async prepare(lt) { await A.foot.prepare(ca(lt)); if (lt > 4.0) await B.foot.prepare(cb(lt)); },
        update(lt) {
          enter(A.win, lt, 0, { out: 4.3 }); enter(B.win, lt, 4.25, { dy: 70 });
          A.foot.update(ca(lt), { cam: keys(lt, [[0, { z: 1, cx: 720, cy: 450 }], [1.6, { z: 1, cx: 720, cy: 450 }], [2.8, { z: 1.4, cx: 740, cy: 520 }], [4.2, { z: 1.4, cx: 740, cy: 520 }]]) });
          B.foot.update(cb(lt), { cam: keys(lt, [[4.5, { z: 1, cx: 720, cy: 450 }], [6.2, { z: 1, cx: 720, cy: 450 }], [7.2, { z: 1.35, cx: 830, cy: 400 }]]) });
          slide(n1, lt, 2.4, { dx: 30, out: 4.2 }); slide(n2, lt, 6.6, { dx: 30 });
        },
        cues: () => [{ t: 0, type: 'whoosh' }, { t: 4.25, type: 'whoosh' }],
      };
    } },

  // ── 5 ── UC5: a lifecycle policy change ──────────────────────────────────
  { id: 'lifecycle', dur: 7, chapter: 5, channel: CH.cli, usecase: UC.life, bg: 'ink', caption: 'Synthetic snapshot and scenario (engine/examples/dashboard_records) · hypothetical policy times',
    async build(root) {
      const T = await termWindow(root, { cast: 'lifecycle', x: 70, y: 300, w: 950, cols: 100, rows: 25, title: 'eplyx — ~/lifecycle' });
      const B = await webWindow(root, { clip: 'dashboard3', x: 1060, y: 250, w: 800, url: '127.0.0.1:4185/runs/run_…2b1efdfb2e8b' });
      const tb = titleBlock(root, { x: 80, y: 115, w: 940, kicker: 'Lifecycle change', title: 'A deadline moves.\n*What does it mean for holders?', size: 'sm' });
      const chips = pills(root, { x: 1060, y: 175, wrap: 800, items: [{ text: 'PreEvent' }, { text: 'Ready', tone: 'green' }, { text: 'Blocked', tone: 'red' }, { text: 'Incomplete', tone: 'orange' }] });
      const P = plan(T.term, .3, [{ type: .7, out: .15, hold: .3 }, { type: .9, out: .3, hold: 3.6 }]);
      const m = mark(T.term, T.term.overlay, 'Active', { tone: 'violet', span: 0 });
      const m2 = mark(T.term, T.term.overlay, 'StaleExposure: 1', { tone: 'orange' });
      const ct = lt => pw(lt, [[0, 1.2], [7, 7.6]]);
      return {
        async prepare(lt) { await T.term.prepare(P.map(lt)); await B.foot.prepare(ct(lt)); },
        update(lt) {
          tb.update(lt, { at: 0 }); enter(T.win, lt, .1); enter(B.win, lt, 1.2, { dy: 70 }); staggerIn(chips, lt, .5, { step: .08 });
          T.term.update({ cam: termCam(T.term, lt, [{ a: P.at[1].outEnd, b: 99, text: 'Before:', z: 1.3, dy: 120, dx: 250 }]) });
          m2(seg(lt, P.at[1].outEnd + .6, P.at[1].outEnd + 1)); B.foot.update(ct(lt), { pointer: false });
        },
        cues: () => typing(P.at),
      };
    } },

  // ── 5 ── UC6 + Channel 6: current state, live, in the workspace ─────────
  { id: 'current', dur: 10.5, chapter: 5, channel: CH.cloud, usecase: UC.cur, bg: 'violet', caption: 'Live mainnet observation (public RPC) of PYUSD for a public owner · the checks run offline in the local VM',
    async build(root) {
      const A = await webWindow(root, { clip: 'current', x: 80, y: 150, w: 1300, url: '127.0.0.1:4390/p/token-transitions/analyse' });
      const B = await webWindow(root, { clip: 'current2', x: 80, y: 150, w: 1300, url: '127.0.0.1:4390/p/token-transitions/analyse?observation=obs_0c8d…' });
      const lv = live(root, 1430, 160);
      const tb = titleBlock(root, { x: 1430, y: 210, w: 440, kicker: 'Current state', title: 'Can this exact\n*account move now?', size: 'sm' });
      const steps = [['Observe', 'PYUSD · a public owner · read-only'], ['Transfer 1 PYUSD', 'Verified in the local VM'], ['Candidate migration', 'Exact checks passed'], ['Funds moved', 'No — nothing is signed or sent']]
        .map(([t, s], i) => { const el = h('div', { class: 'abs step step--wide', style: { left: '1430px', top: 520 + i * 92 + 'px' } }, h('b', {}, String(i + 1)), h('div', {}, h('strong', {}, t), h('span', {}, s))); root.append(el); return el; });
      const ca = lt => pw(lt, [[0, 4.6], [1.6, 9.0], [2.4, 11.3], [3.6, 13.5], [4.6, 20.4], [5.4, 24.2], [6.1, 27.2], [7.0, 29.5]]);
      const cb = lt => pw(lt, [[6.8, 11.6], [8.0, 15.9], [10.5, 19.2]]);
      return {
        async prepare(lt) { await A.foot.prepare(ca(lt)); if (lt > 6.4) await B.foot.prepare(cb(lt)); },
        update(lt) {
          enter(A.win, lt, 0, { out: 6.8 }); enter(B.win, lt, 6.75, { dy: 70 });
          A.foot.update(ca(lt), { cam: keys(lt, [[0, { z: 1.15, cx: 640, cy: 420 }], [4.4, { z: 1.15, cx: 640, cy: 420 }], [5.4, { z: 1.3, cx: 560, cy: 420 }]]) });
          B.foot.update(cb(lt), { cam: { z: 1.3, cx: 560, cy: 420 } });
          slide(lv, lt, .2, { dx: 0, dy: -10 }); tb.update(lt, { at: .2 });
          const on = [2.4, 6.1, 8.2, 9.0];
          steps.forEach((el, i) => { slide(el, lt, .8 + i * .1, { dx: 20 }); el.classList.toggle('step--done', lt > on[i]); });
        },
        cues: () => [{ t: 2.4, type: 'pass' }, { t: 6.1, type: 'pass' }, { t: 8.2, type: 'pass' }],
      };
    } },

  // ── 6 ── UC7: a fee parameter change ─────────────────────────────────────
  { id: 'fee', dur: 7.5, chapter: 6, channel: CH.cli, usecase: UC.fee, bg: 'ink', caption: 'One retained mainnet DepositSol (record 151010f7…) · SOL deposit fee 0 → 1% · manager signer assumed in simulation',
    async build(root) {
      const T = await termWindow(root, { cast: 'fee', x: 70, y: 220, w: 1000, cols: 100, rows: 26, title: 'eplyx — ~/stake-pool-program' });
      const tb = titleBlock(root, { x: 80, y: 105, w: 1300, kicker: 'Fee parameter', title: 'What does a 1% deposit fee *do to a real deposit?', size: 'sm' });
      const bars = h('div', { class: 'abs bars', style: { left: '1130px', top: '300px' } });
      bars.innerHTML = `<div class="bar"><span>Depositor receives · fee 0%</span><i><em></em></i><b>760,985,008</b></div><div class="bar bar--b"><span>Depositor receives · fee 1%</span><i><em></em></i><b>753,375,157</b></div><div class="bar bar--c"><span>Manager fee account</span><i><em></em></i><b>+7,609,851</b></div><p>raw pool tokens · <strong>reproduced: true</strong></p>`;
      root.append(bars);
      const P = plan(T.term, .3, [{ type: 1.0, out: .2, hold: .6 }, { type: .6, out: .2, hold: 1.4 }, { type: .8, out: .2, hold: 1.6 }]);
      const m = mark(T.term, T.term.overlay, '"delta_raw": "-7609851"', { tone: 'red' });
      return {
        prepare: lt => T.term.prepare(P.map(lt)),
        update(lt) {
          enter(T.win, lt, 0); tb.update(lt, { at: .05 });
          put(bars, { o: seg(lt, .4, .9) });
          const k = seg(lt, P.at[1].outEnd, P.at[1].outEnd + 1.1, ease.inOut);
          const ems = bars.querySelectorAll('em'); ems[0].style.width = 100 * k + '%'; ems[1].style.width = 99 * k + '%'; ems[2].style.width = 1 * k + '%';
          bars.querySelectorAll('.bar').forEach((b, i) => b.style.opacity = seg(lt, P.at[1].outEnd + i * .25, P.at[1].outEnd + .4 + i * .25));
          bars.querySelector('p').style.opacity = seg(lt, P.at[2].outEnd, P.at[2].outEnd + .4);
          m(seg(lt, P.at[1].outEnd + .4, P.at[1].outEnd + .8));
          T.term.update();
        },
        cues: () => typing(P.at),
      };
    } },

  // ── 6 ── UC8: does the code change the fee's effect? ─────────────────────
  { id: 'interaction', dur: 6.5, chapter: 6, channel: CH.cli, usecase: UC.inter, bg: 'ink', caption: 'Upgrade candidate = constructed Step 10B fixture (a664f74b…) · same retained DepositSol',
    async build(root) {
      const T = await termWindow(root, { cast: 'interaction', x: 70, y: 250, w: 1000, cols: 100, rows: 17, title: 'eplyx — ~/stake-pool-program' });
      const tb = titleBlock(root, { x: 80, y: 105, w: 1400, kicker: 'Upgrade × fee', title: 'Does the new code change *the fee’s effect?', size: 'sm' });
      const grid = h('div', { class: 'abs matrix', style: { left: '1130px', top: '280px' } });
      grid.innerHTML = `<div></div><div class="mh">fee 0%</div><div class="mh">fee 1%</div><div class="mh">fee effect</div>
        <div class="mh">V1 code</div><div class="mc">baseline</div><div class="mc">−7,609,851</div><div class="mc mc--e">−7,609,851</div>
        <div class="mh">V2 code</div><div class="mc">0 change</div><div class="mc">−7,609,851</div><div class="mc mc--e">−7,609,851</div>
        <div></div><div></div><div class="mh">interaction</div><div class="mc mc--z">0</div>`;
      root.append(grid);
      const P = plan(T.term, .3, [{ type: 2.0, out: .4, hold: 3.2 }]);
      const m = mark(T.term, T.term.overlay, 'no_measured_interaction', { tone: 'green' });
      return {
        prepare: lt => T.term.prepare(P.map(lt)),
        update(lt) {
          enter(T.win, lt, 0); tb.update(lt, { at: .05 });
          [...grid.children].forEach((c, i) => put(c, { o: seg(lt, P.at[0].outEnd + i * .05, P.at[0].outEnd + .3 + i * .05), y: (1 - seg(lt, P.at[0].outEnd + i * .05, P.at[0].outEnd + .4 + i * .05, ease.outQuint)) * 10 }));
          m(seg(lt, P.at[0].outEnd + .3, P.at[0].outEnd + .7)); T.term.update({ cam: termCam(T.term, lt, [{ a: P.at[0].outEnd + .2, b: 99, text: 'no_measured_interaction', z: 1.35, dy: 40 }]) });
        },
        cues: () => typing(P.at),
      };
    } },

  // ── 7 ── UC10 + Channel 6: share local runs ──────────────────────────────
  { id: 'share', dur: 8.5, chapter: 7, channel: CH.cloud, usecase: UC.share, bg: 'violet', caption: 'Real device-code sign-in approved in the browser · sync uploads saved records; viewing them never re-executes',
    async build(root) {
      const T = await termWindow(root, { cast: 'sync', x: 70, y: 280, w: 920, cols: 100, rows: 22, title: 'eplyx — ~/token-migration' });
      const A = await webWindow(root, { clip: 'device', x: 1030, y: 230, w: 830, url: '127.0.0.1:4390/device?code=…' });
      const B = await webWindow(root, { clip: 'workspace', x: 1030, y: 230, w: 830, url: '127.0.0.1:4390/p/token-transitions/runs' });
      const tb = titleBlock(root, { x: 80, y: 120, w: 1200, kicker: 'Share & manage', title: 'login · link · sync — *one workspace history.', size: 'sm' });
      const P = plan(T.term, .3, [{ type: .7, out: .3, hold: 1.6 }, { type: .7, out: .2, hold: .5 }, { type: .5, out: .9, hold: 2.2 }]);
      const ca = lt => pw(lt, [[.5, .9], [2.4, 4.08]]), cb = lt => pw(lt, [[4.3, 1.1], [5.3, 1.1], [5.31, 6.4], [8.5, 8.8]]);
      return {
        async prepare(lt) { await T.term.prepare(P.map(lt)); await A.foot.prepare(ca(lt)); if (lt > 4) await B.foot.prepare(cb(lt)); },
        update(lt) {
          tb.update(lt, { at: 0 }); enter(T.win, lt, .1); enter(A.win, lt, .6, { dy: 70, out: 4.3 }); enter(B.win, lt, 4.3, { dy: 70 });
          A.foot.update(ca(lt), { cam: { z: 1.35, cx: 720, cy: 300 } }); B.foot.update(cb(lt)); T.term.update();
        },
        cues: () => typing(P.at),
      };
    } },

  // ── 7 ── Channel 8: the HTTP API ─────────────────────────────────────────
  { id: 'api', dur: 5.5, chapter: 7, channel: CH.api, usecase: UC.share, bg: 'ink', caption: 'curl with the project token against the local service',
    async build(root) {
      const T = await termWindow(root, { cast: 'api', x: 70, y: 230, w: 1100, cols: 100, rows: 22, title: 'eplyx — curl' });
      const tb = titleBlock(root, { x: 80, y: 110, w: 1400, kicker: 'HTTP API', title: 'Everything the UI does, *over HTTP.', size: 'sm' });
      const eps = ['POST /checks', 'GET /runs/{id}', 'GET /setup', 'GET /capabilities', 'POST /governance/squads/verify', 'POST /governance/squads/attest', 'GET /v1/ops'];
      const chips = eps.map((e, i) => { const el = h('div', { class: 'abs endpoint', style: { left: '1240px', top: 250 + i * 88 + 'px' } }, e); root.append(el); return el; });
      const P = plan(T.term, .25, [{ type: .9, out: .2, hold: 1.0 }, { type: .9, out: .2, hold: 1.6 }]);
      return {
        prepare: lt => T.term.prepare(P.map(lt)),
        update(lt) { enter(T.win, lt, 0); tb.update(lt, { at: 0 }); chips.forEach((c, i) => slide(c, lt, .4 + i * .09, { dx: 30 })); T.term.update(); },
        cues: () => typing(P.at),
      };
    } },

  // ── 7 ── The evidence layer and the method demo ─────────────────────────
  { id: 'evidence', dur: 6.5, chapter: 7, channel: CH.cli, bg: 'violet', caption: 'Synthetic fixture-lending corpus: a method demo, not a production use case',
    async build(root) {
      const tb = titleBlock(root, { x: 80, y: 115, w: 1700, kicker: 'Building the evidence', title: 'Bundles are built rarely, *reviewed, and pinned.', size: 'sm' });
      const stages = ['ingest · discover', 'historical acquire  S−1 / S', 'screen same-slot writers', 'versions resolve', 'corpus select', 'bundle build', 'bundle verify'];
      const row = stages.map((s, i) => { const el = h('div', { class: 'abs stage', style: { left: 80 + i * 252 + 'px', top: '300px' } }, h('b', {}, String(i + 1).padStart(2, '0')), s); root.append(el); return el; });
      const T = await termWindow(root, { cast: 'synthetic', x: 80, y: 430, w: 1000, cols: 100, rows: 22, title: 'eplyx — ~/eplyx' });
      const stats = [['141', 'fixtures'], ['89', 'outcome-identical'], ['52', 'changed'], ['11', 'critical'], ['$6,182,370', 'collateral represented']];
      const st = stats.map(([n, l], i) => { const el = h('div', { class: 'abs stat', style: { left: '1140px', top: 450 + i * 100 + 'px' } }, h('b', {}, n), h('span', {}, l)); root.append(el); return el; });
      const P = plan(T.term, 1.4, [{ type: .7, out: .3, hold: 3.5 }]);
      return {
        prepare: lt => T.term.prepare(P.map(lt)),
        update(lt) { tb.update(lt, { at: 0 }); row.forEach((el, i) => pop(el, lt, .3 + i * .1)); enter(T.win, lt, 1.0); st.forEach((el, i) => slide(el, lt, P.at[0].outEnd + i * .12, { dx: 30 })); T.term.update(); },
        cues: () => [...stages.map((_, i) => ({ t: .3 + i * .1, type: 'tick' })), ...typing(P.at)],
      };
    } },

  // ── 8 ── Who uses what ───────────────────────────────────────────────────
  { id: 'personas', dur: 7, chapter: 8, bg: 'ink',
    async build(root) {
      const tb = titleBlock(root, { x: 80, y: 115, w: 1700, kicker: 'Who uses what', title: 'Five roles. *One source of evidence.', size: 'sm' });
      const rows = [['Protocol developer', ['CLI', 'ci check', 'hosted gate in the PR']], ['Auditor · security reviewer', ['CLI analyses', 'historical research', 'reproducible reports']], ['Multisig signer', ['governance verify before signing', 'attest after execution']], ['Token issuer · risk team', ['migration', 'lifecycle', 'current state', 'fee analysis']], ['Service operator', ['bundles', 'projects & tokens', 'ops', 'backup & restore']]];
      const els = rows.map(([who, route], i) => { const el = h('div', { class: 'abs persona', style: { left: '80px', top: 290 + i * 128 + 'px' } }, h('strong', {}, who), h('div', {}, ...route.map((r, j) => [j ? h('i', {}, '→') : null, h('span', {}, r)]))); root.append(el); return el; });
      return { update(lt) { tb.update(lt, { at: 0 }); els.forEach((el, i) => slide(el, lt, .35 + i * .14, { dx: -40 })); }, cues: () => rows.map((_, i) => ({ t: .35 + i * .14, type: 'tick' })) };
    } },

  // ── 8 ── Boundaries ──────────────────────────────────────────────────────
  { id: 'boundaries', dur: 6, chapter: 8, bg: 'ink',
    async build(root) {
      const tb = titleBlock(root, { x: 80, y: 115, w: 1700, kicker: 'Boundaries', title: 'Coverage is explicit. *A pass is not a safety claim.', size: 'sm' });
      const items = [['Semantic adapters', 'SPL Stake Pool DepositSol / WithdrawSol · Token-2022 transfers · a narrow Kamino slice · a bounded Memo path'],
        ['no_semantic_coverage', 'means “not looked at” — never “nothing changed”'], ['Not replayed yet', 'address lookup tables · account creation/closure · general CPI'],
        ['A pass', 'holds within the corpus it replayed'], ['Not built', 'GitHub App · AI analysis · fiat valuation']];
      const els = items.map(([k, v], i) => { const el = h('div', { class: 'abs bound', style: { left: '80px', top: 300 + i * 120 + 'px' } }, h('b', {}, k), h('span', {}, v)); root.append(el); return el; });
      return { update(lt) { tb.update(lt, { at: 0 }); els.forEach((el, i) => slide(el, lt, .4 + i * .14, { dx: -30 })); } };
    } },

  // ── 8 ── End card ────────────────────────────────────────────────────────
  { id: 'end', dur: 5, chapter: 8, hud: false, bg: 'violet',
    async build(root) {
      const mk = h('div', { class: 'abs endmark', html: window.markSVG('endmark__svg') }); root.append(mk);
      const word = h('div', { class: 'abs endword' }, 'Eplyx'); root.append(word);
      const tb = titleBlock(root, { x: 0, y: 640, w: 1920, title: 'Know what changes. *See who is affected.', size: 'sm', align: 'center' });
      const sub = h('div', { class: 'abs endsub' }, 'Deterministic · offline · read-only  —  github.com/thomasdevving/Eplyx'); root.append(sub);
      return {
        update(lt) {
          const c = mk.querySelector('.mk-c'), w = mk.querySelector('.mk-w');
          put(c, { x: lerp(-60, 0, seg(lt, .1, .9, ease.outQuint)), y: lerp(-40, 0, seg(lt, .1, .9, ease.outQuint)), o: seg(lt, .1, .5) });
          put(w, { x: lerp(60, 0, seg(lt, .25, 1.05, ease.outQuint)), y: lerp(40, 0, seg(lt, .25, 1.05, ease.outQuint)), o: seg(lt, .25, .65) });
          put(mk, { s: lerp(.9, 1, seg(lt, 0, 1.2, ease.out)) * (1 + .01 * Math.sin(lt * 1.5)) });
          put(word, { y: (1 - seg(lt, .7, 1.4, ease.outQuint)) * 30, o: seg(lt, .7, 1.2) });
          tb.update(lt, { at: 1.3 }); put(sub, { o: seg(lt, 2.2, 2.8), y: (1 - seg(lt, 2.2, 2.9, ease.outQuint)) * 12 });
        },
        cues: () => [{ t: .1, type: 'boom' }],
      };
    } },
];
