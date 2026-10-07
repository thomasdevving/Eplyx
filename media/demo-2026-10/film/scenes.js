// The storyboard: the problem, what Eplyx is, how it works, who it is for, and
// one scenario per person. Every terminal and browser pixel comes from a
// recording of the real binaries and services (see ../README.md); overlays
// only label, frame and point at what those recordings show.
import { h, svg, put, seg, ease, keys, pw, clamp, lerp, splitWords, riseWords } from './core.js';
import { titleBlock, termWindow, webWindow, plan, enter, badge, pop, note, slide, pills, staggerIn, termCam, mark, scenario } from './kit.js';
import { Footage } from './components.js';

const CH = {
  web: [1, 'Public website'], cli: [2, 'Local CLI'], dash: [3, 'Local dashboard'], gate: [4, 'CI gate, offline'],
  hosted: [5, 'CI gate, hosted'], cloud: [6, 'Cloud workspace'], ops: [7, 'Operator console'], api: [8, 'HTTP API'],
};
const UC = {
  upgrade: [1, 'Program upgrade'], declare: [2, 'Declared changes'], gov: [3, 'Governance binding'], mig: [4, 'Token migration'],
  life: [5, 'Lifecycle change'], cur: [6, 'Current state'], fee: [7, 'Fee change'], inter: [8, 'Upgrade and fee together'],
  hist: [9, 'Historical research'], share: [10, 'Share and manage'],
};
const live = (root, x, y, text = 'LIVE ON SOLANA MAINNET') => { const el = h('div', { class: 'abs live', style: { left: x + 'px', top: y + 'px' } }, h('i'), text); root.append(el); return el; };
const typing = at => at.map(a => ({ t: a.typeStart, type: 'typing', dur: a.runAt - a.typeStart })).filter(c => c.dur > .05);
const ICON = {
  code: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M8 7l-5 5 5 5M16 7l5 5-5 5M13.5 4l-3 16"/></svg>',
  shield: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M12 3l8 3v6c0 4.5-3.4 8.3-8 9-4.6-.7-8-4.5-8-9V6z"/><path d="M8.5 12l2.5 2.5 4.5-5"/></svg>',
  sign: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M4 17c2-3 3.5-6 5-6s.5 5 2 5 2.5-3 4-3 1 2 3 2"/><path d="M3 21h18"/></svg>',
  coins: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5"><ellipse cx="9" cy="7" rx="6" ry="3"/><path d="M3 7v5c0 1.7 2.7 3 6 3s6-1.3 6-3V7"/><path d="M9 15v2c0 1.7 2.7 3 6 3s6-1.3 6-3v-5c0-1.7-2.7-3-6-3"/></svg>',
};

export const scenes = [
  // ── The problem ─────────────────────────────────────────────────────────
  { id: 'hook', dur: 10, hud: false, bg: 'ink', sweep: false,
    async build(root) {
      const l1 = h('div', { class: 'abs hook', style: { top: '380px' } }); const w1 = splitWords(l1, 'You know what you changed in the code.', 'w');
      const l2 = h('div', { class: 'abs hook', style: { top: '480px' } }); const w2 = splitWords(l2, 'But do you know what it will do to real users?', 'w');
      w2.forEach(s => s.classList.add('grad'));
      root.append(l1, l2);
      const foot = new Footage({ clip: 'hero-full', w: 1920, h: 1200 }); await foot.load();
      foot.root.style.position = 'absolute'; foot.root.style.top = '-60px'; root.append(foot.root);
      const ct = lt => clamp(1.4 + (lt - 4.3) * 1.0, 1.4, 10.9);
      return {
        prepare: lt => foot.prepare(ct(lt)),
        update(lt) {
          riseWords(w1, lt, .3, { stagger: .07, out: 3.9 }); riseWords(w2, lt, 1.6, { stagger: .07, out: 4.0 });
          const p = seg(lt, 4.2, 5.2, ease.inOut);
          put(foot.root, { o: p, s: lerp(1.06, 1, seg(lt, 4.2, 10, ease.out)), origin: '50% 50%' });
          foot.update(ct(lt), { pointer: false });
        },
        cues: () => [{ t: 4.4, type: 'boom' }],
      };
    } },

  // ── How it works ────────────────────────────────────────────────────────
  { id: 'how', dur: 16, bg: 'ink', caption: 'Example from the film: the SPL Stake Pool program, 10 retained mainnet transactions and a constructed regression build',
    async build(root) {
      const tb = titleBlock(root, { x: 110, y: 120, w: 1700, kicker: 'How Eplyx works', title: 'A simulation of your protocol, *rebuilt from real mainnet data.', size: 'sm' });
      const colA = h('div', { class: 'hw-col', style: { left: '110px' } });
      colA.innerHTML = `<div class="hw-head"><b>1</b>Real on-chain data</div>` + [['Transactions', '10 real mainnet transactions'], ['Accounts', 'exact state before and after'], ['Programs', 'the binary that actually ran'], ['Dependencies', 'pinned to the version live at that slot']].map(([a, b]) => `<div class="hw-row">${a}<span>${b}</span></div>`).join('');
      const colB = h('div', { class: 'hw-col', style: { left: '700px' } });
      colB.innerHTML = `<div class="hw-head"><b>2</b>Rebuilt in isolated Solana VMs</div>
        <div class="hw-vm"><small>Today</small><strong>The program that ran on mainnet</strong><code>ec2dfefa…d7e1</code><div class="tx"><i>DepositSol</i><i>WithdrawSol ×9</i></div></div>
        <div class="hw-vm hw-vm--b"><small>Proposed</small><strong>Your change</strong><code>3193eabd…b099</code><div class="tx"><i>DepositSol</i><i>WithdrawSol ×9</i></div></div>
        <div class="hw-head" style="margin-top:6px"><b>3</b>The same user actions, replayed in both</div>`;
      const colC = h('div', { class: 'hw-col', style: { left: '1330px' } });
      colC.innerHTML = `<div class="hw-head"><b>4</b>What actually changes</div>
        <div class="hw-res">Withdrawals that used to succeed <em>now revert</em><span>9 of 9 replayed withdrawals</span></div>
        <div class="hw-res">The depositor receives <em>1 bp fewer</em> pool tokens<span>measured on the real deposit</span></div>
        <div class="hw-res">Anything else that differs<span>named, or flagged as unexplained</span></div>`;
      const a1 = h('div', { class: 'hw-arrow', style: { left: '610px' } }), a2 = h('div', { class: 'hw-arrow', style: { left: '1245px' } });
      const foot = h('div', { class: 'abs hw-foot', html: '<b>5 · Reproducible.</b> Every input is pinned by hash, so anyone can rerun the analysis offline and get the same result.' });
      root.append(colA, a1, colB, a2, colC, foot);
      const rowsA = [...colA.children], vms = [...colB.querySelectorAll('.hw-vm')], txs = [...colB.querySelectorAll('.tx i')], head3 = colB.lastElementChild, headB = colB.firstElementChild, rowsC = [...colC.children];
      return {
        update(lt) {
          tb.update(lt, { at: .1 });
          rowsA.forEach((el, i) => slide(el, lt, .8 + i * .22, { dx: 0, dy: 14 }));
          put(a1, { o: seg(lt, 3.4, 3.8), sx: seg(lt, 3.4, 3.9, ease.out), origin: '0 50%' });
          slide(headB, lt, 3.7, { dx: 0, dy: 12 }); vms.forEach((el, i) => slide(el, lt, 4.0 + i * .3, { dx: 0, dy: 18 }));
          slide(head3, lt, 6.4, { dx: 0, dy: 10 });
          txs.forEach((el, i) => { const t0 = 6.8 + (i % 2) * .25; put(el, { o: seg(lt, t0, t0 + .3), x: (1 - seg(lt, t0, t0 + .6, ease.outQuint)) * -30 }); });
          put(a2, { o: seg(lt, 8.8, 9.2), sx: seg(lt, 8.8, 9.3, ease.out), origin: '0 50%' });
          rowsC.forEach((el, i) => slide(el, lt, 9.2 + i * .55, { dx: 0, dy: 14 }));
          put(foot, { o: seg(lt, 12.4, 13), y: (1 - seg(lt, 12.4, 13.2, ease.outQuint)) * 12 });
        },
        cues: () => [.8, 1.02, 1.24, 1.46, 4.0, 4.3, 6.8, 7.05, 9.2, 9.75, 10.3].map(t => ({ t, type: 'tick' })).concat([{ t: 12.4, type: 'pass' }]),
      };
    } },

  // ── Who it is for ───────────────────────────────────────────────────────
  { id: 'who', dur: 6, bg: 'violet',
    async build(root) {
      const tb = titleBlock(root, { x: 110, y: 130, w: 1700, kicker: 'Who it is for', title: 'For the people who ship, secure *and approve on-chain changes.', size: 'sm' });
      const people = [
        ['code', 'Protocol developers', 'Check a new build or a parameter change against real usage, on your machine or on every pull request.', 'upgrade checks · fee changes · CI'],
        ['shield', 'Security and audit teams', 'See exactly what changed, then reproduce any finding offline from the same pinned evidence.', 'historical replay · reproducible reports'],
        ['sign', 'Governance signers', 'Before you approve an upgrade, confirm the proposal holds the exact code that was analysed.', 'Squads proposal binding'],
        ['coins', 'Token issuers and risk teams', 'Rehearse a migration or a policy change against real holders before anything moves.', 'migration · lifecycle · current state'],
      ];
      const cols = people.map(([icon, who, what, routes], i) => { const el = h('div', { class: 'who', style: { left: 110 + i * 430 + 'px' }, html: `${ICON[icon]}<strong>${who}</strong><p>${what}</p><em>${routes}</em>` }); root.append(el); return el; });
      return { update(lt) { tb.update(lt, { at: 0 }); cols.forEach((el, i) => slide(el, lt, .6 + i * .18, { dx: 0, dy: 24 })); }, cues: () => people.map((_, i) => ({ t: .6 + i * .18, type: 'tick' })) };
    } },

  // ── Channel 1: the public website ───────────────────────────────────────
  { id: 'website', dur: 7, channel: CH.web, bg: 'violet', caption: 'Recorded from the local public site · no account · nothing executes in the browser',
    async build(root) {
      const A = await webWindow(root, { clip: 'start', x: 80, y: 150, w: 1240, url: '127.0.0.1:4173/start' });
      const B = await webWindow(root, { clip: 'demo', x: 80, y: 150, w: 1240, url: '127.0.0.1:4173/runs/demo' });
      const tb = titleBlock(root, { x: 1390, y: 220, w: 460, kicker: 'Try it in the browser', title: 'Start with\n*your question.', size: 'sm',
        sub: 'Pick what you are checking and see what to bring. Or open a saved report. No account, nothing to install.' });
      const ca = lt => pw(lt, [[0, 1.6], [4.2, 10.9]]), cb = lt => pw(lt, [[4.0, .9], [7, 7.0]]);
      return {
        async prepare(lt) { await A.foot.prepare(ca(lt)); if (lt > 3.6) await B.foot.prepare(cb(lt)); },
        update(lt) {
          enter(A.win, lt, 0, { out: 4.0 }); A.foot.update(ca(lt), { cam: { z: 1.08, cx: 640, cy: 470 } });
          enter(B.win, lt, 3.95, { dy: 80 }); B.foot.update(cb(lt), { cam: keys(lt, [[4, { z: 1, cx: 720, cy: 450 }], [7, { z: 1.22, cx: 640, cy: 420 }]]), pointer: false });
          tb.update(lt, { at: .3 });
        },
        cues: () => [{ t: .1, type: 'whoosh' }, { t: 3.95, type: 'whoosh' }],
      };
    } },

  // ── Scenario: a developer about to ship a new build ────────────────────
  { id: 'upgrade', dur: 14, channel: CH.cli, usecase: UC.upgrade, bg: 'violet', caption: 'Real terminal recording · the candidate is a constructed regression build, not an upstream release',
    async build(root) {
      const T = await termWindow(root, { cast: 'upgrade', x: 700, y: 140, w: 1150, cols: 100, rows: 30, title: 'eplyx · ~/stake-pool-program' });
      const sc = scenario(root, { x: 96, y: 170, w: 560, persona: 'Protocol developer', title: 'You are about to ship\n*a new build.', size: 'md',
        does: 'Replays 10 real mainnet transactions against the program that ran them, and against your build. Offline, on your machine.', doesY: 420 });
      const P = plan(T.term, .4, [{ type: .9, out: .45, hold: 1.4 }, { type: .95, out: .25, hold: 1.7 }, { type: .95, out: .3, hold: 4.6 }]);
      const ok = badge(root, { x: 96, y: 650, label: 'Same build: passed', sub: 'control · exit 0', tone: 'green' });
      const bad = badge(root, { x: 96, y: 650, label: 'Your build: failed', sub: 'exit 1 · blocks the merge', tone: 'red' });
      const res = note(root, { x: 96, y: 790, w: 560, title: '9 of 9 real withdrawals now revert.', body: 'The one real deposit returns 1 bp fewer pool tokens.' });
      const mk1 = mark(T.term, T.term.overlay, 'now_reverts', { tone: 'red' }), mk2 = mark(T.term, T.term.overlay, 'pool_tokens_received/decreased', { tone: 'red' });
      const a = P.at;
      return {
        prepare: lt => T.term.prepare(P.map(lt)),
        update(lt) {
          enter(T.win, lt, 0); sc.update(lt, { at: .15 });
          T.term.update({ cam: termCam(T.term, lt, [
            { a: a[0].outEnd, b: a[0].holdEnd - .2, text: 'records:', z: 1.45 },
            { a: a[1].outEnd, b: a[1].holdEnd - .1, text: 'GATE: PASSED', z: 1.55, dy: -60 },
            { a: a[2].outEnd + .1, b: 99, text: 'FINDINGS', z: 1.5, dy: 120 },
          ]) });
          pop(ok, lt, a[1].outEnd + .1, { out: a[1].holdEnd });
          pop(bad, lt, a[2].outEnd + .5);
          mk1(seg(lt, a[2].outEnd + .9, a[2].outEnd + 1.3)); mk2(seg(lt, a[2].outEnd + 1.2, a[2].outEnd + 1.6));
          slide(res, lt, a[2].outEnd + 1.5, { dx: 0, dy: 16 });
        },
        cues: () => [...typing(a), { t: a[1].outEnd + .1, type: 'pass' }, { t: a[2].outEnd + .5, type: 'fail' }],
      };
    } },

  // ── Scenario: intended changes ──────────────────────────────────────────
  { id: 'expectations', dur: 7, channel: CH.cli, usecase: UC.declare, bg: 'violet', caption: 'Declarations from docs/pilot/expected-changes.bounded.toml · real terminal recording',
    async build(root) {
      const T = await termWindow(root, { cast: 'expectations', x: 80, y: 140, w: 1100, cols: 100, rows: 30, title: 'eplyx · ~/stake-pool-program' });
      const sc = scenario(root, { x: 1250, y: 170, w: 600, persona: 'Protocol developer', title: 'Some changes\n*are on purpose.', size: 'sm',
        does: 'You declare them, narrowly, in expected-changes.toml. Eplyx checks each one stays within its bounds, and still fails on any change it cannot explain.', doesY: 360 });
      const chips = pills(root, { x: 1250, y: 600, wrap: 600, items: [{ text: 'expected', tone: 'green' }, { text: 'unexpected', tone: 'red' }, { text: 'exceeded', tone: 'orange' }, { text: 'stale' }, { text: 'unevaluable' }] });
      const P = plan(T.term, .25, [{ type: .75, out: .3, hold: 1.4 }, { type: .95, out: .3, hold: 2.6 }]);
      const a = P.at;
      const m1 = mark(T.term, T.term.overlay, 'expected                       2', { tone: 'green' });
      const m2 = mark(T.term, T.term.overlay, 'GATE: FAILED (undeclarable_change)', { tone: 'red' });
      const bad = badge(root, { x: 1250, y: 730, label: 'Still failed', sub: 'a change nothing can name', tone: 'red' });
      return {
        prepare: lt => T.term.prepare(P.map(lt)),
        update(lt) {
          enter(T.win, lt, 0); sc.update(lt, { at: .15 }); staggerIn(chips, lt, 1.0, { step: .08 });
          T.term.update({ cam: termCam(T.term, lt, [{ a: a[0].outEnd, b: a[0].holdEnd - .1, text: 'max_delta_bps', z: 1.3 }, { a: a[1].outEnd, b: 99, text: 'RESULTS', z: 1.35, dy: 130 }]) });
          m1(seg(lt, a[1].outEnd + .5, a[1].outEnd + .9)); m2(seg(lt, a[1].outEnd + 1.1, a[1].outEnd + 1.5));
          pop(bad, lt, a[1].outEnd + 1.4);
          chips.forEach((c, i) => { c.style.borderColor = (i === 0 && lt > a[1].outEnd + .5) ? 'rgba(111,227,180,.8)' : ''; });
        },
        cues: () => [...typing(a), { t: a[1].outEnd + 1.4, type: 'fail' }],
      };
    } },

  // ── Channel 4: the offline CI gate ─────────────────────────────────────
  { id: 'gate', dur: 5, channel: CH.gate, usecase: UC.upgrade, bg: 'ink', caption: 'Excerpt from this repository\'s .github/workflows/reproducibility.yml · no service and no endpoint needed',
    async build(root) {
      const tb = titleBlock(root, { x: 96, y: 170, w: 760, kicker: 'In your CI pipeline', title: 'The exit code\n*is the gate.', size: 'md' });
      const codes = [['0', 'Passed', 'green'], ['1', 'Unexpected or out-of-bounds change', 'red'], ['2', 'Could not evaluate'], ['3', 'Stale declaration', 'orange'], ['4', 'Incompatible baseline'], ['5', 'A declaration it cannot judge']];
      const tiles = codes.map(([c, t, tone], i) => { const el = h('div', { class: `abs codetile ${tone ? 'codetile--' + tone : ''}`, style: { left: 96 + (i % 3) * 258 + 'px', top: 500 + Math.floor(i / 3) * 172 + 'px' } }, h('b', {}, c), h('span', {}, t)); root.append(el); return el; });
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
        update(lt) { tb.update(lt, { at: .05 }); tiles.forEach((el, i) => pop(el, lt, .5 + i * .1, { from: .85 })); enter(yaml, lt, .4, { dy: 40, rx: 6 }); },
        cues: () => codes.map((_, i) => ({ t: .5 + i * .1, type: 'tick' })),
      };
    } },

  // ── Channel 7: the platform team sets it up once ───────────────────────
  { id: 'operator', dur: 8, channel: CH.ops, usecase: UC.share, bg: 'violet', caption: 'eplyx-server admin on the service volume · operator console at /projects',
    async build(root) {
      const T = await termWindow(root, { cast: 'operator', x: 70, y: 360, w: 930, cols: 100, rows: 21, title: 'operator · eplyx-server admin' });
      const B = await webWindow(root, { clip: 'console', x: 1040, y: 250, w: 820, url: '127.0.0.1:4173/projects' });
      const tag = h('div', { class: 'abs persona', style: { left: '80px', top: '118px' } }, h('b', {}, 'Platform team')); root.append(tag);
      const tb = titleBlock(root, { x: 80, y: 158, w: 940, title: 'Set it up once.\n*CI can never move the baseline.', size: 'sm',
        sub: 'Register the evidence, activate it, issue a CI token. That token can submit checks, never change what they are measured against.' });
      tb.el.querySelector('.lead').style.fontSize = '21px';
      const P = plan(T.term, .35, [{ skip: true }, { type: .65, out: .3, hold: .75 }, { type: .65, out: .25, hold: .75 }, { type: .65, out: .2, hold: 1.4 }]);
      const ct = lt => pw(lt, [[0, 2.4], [1.5, 3.7], [4.0, 6.1], [8, 8.25]]);
      return {
        async prepare(lt) { await T.term.prepare(P.map(lt)); await B.foot.prepare(ct(lt)); },
        update(lt) {
          put(tag, { o: seg(lt, 0, .4) }); tb.update(lt, { at: .05 }); enter(T.win, lt, .1); enter(B.win, lt, .9, { dy: 70 });
          T.term.update(); B.foot.update(ct(lt), { cam: keys(lt, [[0, { z: 1, cx: 720, cy: 450 }], [5.2, { z: 1, cx: 720, cy: 450 }], [6.7, { z: 1.45, cx: 520, cy: 520 }]]) });
        },
        cues: () => typing(P.at),
      };
    } },

  // ── Channel 5: every pull request, checked by the hosted service ────────
  { id: 'hosted', dur: 10, channel: CH.hosted, usecase: UC.upgrade, bg: 'violet', caption: 'scripts/eplyx-submit.sh against a local eplyx-server · PR comment composed by scripts/eplyx-pr-comment.py, nothing posted',
    async build(root) {
      const T = await termWindow(root, { cast: 'hosted', x: 70, y: 330, w: 900, cols: 100, rows: 18, title: 'GitHub Actions step · eplyx-submit.sh' });
      const B = await webWindow(root, { clip: 'hosted', x: 1010, y: 150, w: 850, url: '127.0.0.1:4390/p/stake-pool-upgrades/runs/run_01M4B1V5…' });
      const tag = h('div', { class: 'abs persona', style: { left: '80px', top: '118px' } }, h('b', {}, 'Protocol developer')); root.append(tag);
      const tb = titleBlock(root, { x: 80, y: 158, w: 920, title: 'Every pull request,\n*checked automatically.', size: 'sm' });
      const P = plan(T.term, .3, [{ type: .5, out: .1, hold: .2 }, { type: 1.1, out: 1.0, hold: 3.6 }]);
      const ct = lt => pw(lt, [[0, 3.0], [2.6, 3.62], [5.6, 8.5], [10, 11.6]]);
      const bad = badge(root, { x: 560, y: 820, label: 'failed · exit 1', sub: 'the job fails, the merge is blocked', tone: 'red' });
      const pr = h('div', { class: 'abs prcard', style: { left: '70px', top: '540px' } });
      pr.innerHTML = `<header><svg class="prcard__icon" width="18" height="18" viewBox="0 0 16 16"><path fill="currentColor" d="M2 2.5A1.5 1.5 0 0 1 3.5 1h9A1.5 1.5 0 0 1 14 2.5v7a1.5 1.5 0 0 1-1.5 1.5H8.2l-3.1 2.6c-.5.4-1.1 0-1.1-.5V11h-.5A1.5 1.5 0 0 1 2 9.5z"/></svg> Pull request comment <em>composed by scripts/eplyx-pr-comment.py</em></header>
        <p><strong>Unexpected change.</strong> Replaying production transactions against this candidate produced semantic changes that <code>expected-changes.toml</code> does not declare, or that exceed their declared bounds.</p>
        <table><tr><td>Candidate</td><td><code>3193eabd9fe2e479…b099</code></td></tr><tr><td>Records</td><td><code>10</code></td></tr><tr><td>Exit code</td><td><code>1</code></td></tr><tr><td>Run status</td><td><code>failed</code></td></tr></table>`;
      root.append(pr);
      return {
        async prepare(lt) { await T.term.prepare(P.map(lt)); await B.foot.prepare(ct(lt)); },
        update(lt) {
          put(tag, { o: seg(lt, 0, .4) }); tb.update(lt, { at: .05 }); enter(T.win, lt, .1); enter(B.win, lt, 1.6, { dy: 70 });
          T.term.update(); B.foot.update(ct(lt));
          pop(bad, lt, P.at[1].outEnd + .2, { out: 5.8 });
          enter(pr, lt, 5.9, { dy: 50, rx: 6 });
        },
        cues: () => [...typing(P.at), { t: P.at[1].outEnd + .2, type: 'fail' }, { t: 5.9, type: 'whoosh' }],
      };
    } },

  // ── Scenario: a multisig signer, live ──────────────────────────────────
  { id: 'governance', dur: 9, channel: CH.cli, usecase: UC.gov, bg: 'ink', caption: 'Live read of a real Squads V4 proposal through the public mainnet RPC · read-only · Eplyx never signs',
    async build(root) {
      const T = await termWindow(root, { cast: 'governance', x: 70, y: 150, w: 1100, cols: 100, rows: 30, title: 'eplyx · ~/squads' });
      const lv = live(root, 1240, 118);
      const sc = scenario(root, { x: 1240, y: 170, w: 620, persona: 'Multisig signer', title: 'You are asked to approve\n*a program upgrade.', size: 'sm',
        does: 'Reads the Squads proposal from mainnet and checks that its upgrade buffer holds exactly the code that was analysed.', doesY: 360 });
      const eq = h('div', { class: 'abs eqcard', style: { left: '1240px', top: '560px' } },
        h('div', {}, h('small', {}, 'Analysed build'), h('code', {}, 'b10c1c93…0761d')), h('b', {}, '='), h('div', {}, h('small', {}, 'Bytes in the proposal'), h('code', {}, 'b10c1c93…0761d')));
      root.append(eq);
      const n = note(root, { x: 1240, y: 700, w: 610, title: 'Matched at slot 454,211,406.', body: 'Only another vault transaction can change those bytes, so re-verify right before you approve.' });
      const P = plan(T.term, .3, [{ type: 1.2, out: .5, hold: 6 }]);
      const m1 = mark(T.term, T.term.overlay, 'Result:     matched', { tone: 'green' });
      const m2 = mark(T.term, T.term.overlay, 'Observed:', { tone: 'violet', span: 40 });
      return {
        prepare: lt => T.term.prepare(P.map(lt)),
        update(lt) {
          enter(T.win, lt, 0); sc.update(lt, { at: .15 }); slide(lv, lt, .2, { dx: 0, dy: -8 });
          const a = P.at[0];
          T.term.update({ cam: termCam(T.term, lt, [{ a: a.outEnd + .1, b: 99, text: 'Result:', z: 1.35, dy: 110, dx: 300 }]) });
          m1(seg(lt, a.outEnd + .5, a.outEnd + .9)); m2(seg(lt, a.outEnd + .8, a.outEnd + 1.2));
          pop(eq, lt, a.outEnd + 1.2, { from: .9 }); slide(n, lt, a.outEnd + 1.8, { dx: 0, dy: 14 });
        },
        cues: () => [...typing(P.at), { t: P.at[0].outEnd + .5, type: 'pass' }],
      };
    } },

  // ── Scenario: a security team investigating, live ──────────────────────
  { id: 'history', dur: 10, channel: CH.cli, usecase: UC.hist, bg: 'ink', caption: 'Live archive RPC · a real PYUSD TransferChecked and a real Token-2022 upgrade · the last replay uses a constructed regression build',
    async build(root) {
      const T = await termWindow(root, { cast: 'history', x: 70, y: 310, w: 1100, cols: 100, rows: 26, title: 'eplyx · ~/history' });
      const tag = h('div', { class: 'abs persona', style: { left: '80px', top: '118px' } }, h('b', {}, 'Security team')); root.append(tag);
      const tb = titleBlock(root, { x: 80, y: 158, w: 1100, title: 'Something changed on mainnet.\n*What ran, and can you replay it exactly?', size: 'sm' });
      const lv = live(root, 1240, 300, 'LIVE ON THE ARCHIVE');
      const tl = h('div', { class: 'abs timeline', style: { left: '1240px', top: '360px', width: '600px' } });
      tl.innerHTML = `<div class="tl__axis"></div><div class="tl__mk tl__mk--tx" style="left:71.4%"><b>427,146,982</b><span>PYUSD transfer</span></div><div class="tl__mk tl__mk--up" style="left:71.5%"><b>427,147,035</b><span>Token-2022 upgrade</span></div><div class="tl__end tl__end--a">420,000,000</div><div class="tl__end tl__end--b">430,000,000</div>`;
      root.append(tl);
      const steps = ['Find when the program was upgraded', 'Fetch the exact transaction and accounts', 'Fetch the binary that ran after it', 'Replay with the real upgrade: no change', 'Replay with a broken build: 0.001 PYUSD less'].map((s, i) => { const el = h('div', { class: 'abs step', style: { left: '1240px', top: 560 + i * 60 + 'px' } }, h('b', {}, String(i + 1)), s); root.append(el); return el; });
      const P = plan(T.term, .3, [{ type: .85, out: .35, hold: .85 }, { type: .95, out: .35, hold: .95 }, { type: .75, out: .3, hold: .8 }, { type: .65, out: .25, hold: 1.2 }, { type: .75, out: .25, hold: 1.7 }]);
      const a = P.at;
      const m1 = mark(T.term, T.term.overlay, 'fidelity Matched', { tone: 'green' });
      const m2 = mark(T.term, T.term.overlay, 'delta -0.001000', { tone: 'red' });
      return {
        prepare: lt => T.term.prepare(P.map(lt)),
        update(lt) {
          enter(T.win, lt, 0); put(tag, { o: seg(lt, 0, .4) }); tb.update(lt, { at: .05 }); slide(lv, lt, .2, { dx: 0, dy: -8 });
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

  // ── Scenario: a token issuer planning a migration ──────────────────────
  { id: 'migration', dur: 10, channel: CH.cli, usecase: UC.mig, bg: 'violet', caption: 'Synthetic fixture world (examples/migrations/minimal) · the reference build, then a known deadline-defect build',
    async build(root) {
      const T = await termWindow(root, { cast: 'migration', x: 700, y: 140, w: 1140, cols: 100, rows: 30, title: 'eplyx · ~/token-migration' });
      const sc = scenario(root, { x: 96, y: 150, w: 560, persona: 'Token issuer', title: 'You are moving holders\n*to a new token.', size: 'sm',
        does: 'Rehearses the migration in a local VM over every holder, then searches bounded edge cases for a counterexample.', doesY: 330 });
      const axes = ['mechanism', 'funding', 'population', 'reconciliation'].map((k, i) => { const el = h('div', { class: 'abs axis', style: { left: '96px', top: 530 + i * 68 + 'px' } }, h('span', {}, k), h('b', {}, '')); root.append(el); return el; });
      const P = plan(T.term, .25, [{ type: .35, out: .2, hold: .5 }, { type: .45, out: .2, hold: 1.1 }, { type: .5, out: .1, hold: .25 }, { type: .45, out: .25, hold: 1.0 }, { type: .6, out: .2, hold: .7 }, { type: .5, out: .05, hold: .1 }, { type: .5, out: .2, hold: 1.0 }, { type: .5, out: .15, hold: 1.2 }]);
      const a = P.at;
      const cx = badge(root, { x: 96, y: 820, label: 'Counterexample found', sub: 'at the deadline · reproduced · nothing signed', tone: 'red' });
      const st = (el, text, tone) => { const b = el.querySelector('b'); if (b.textContent !== text) b.textContent = text; el.className = 'abs axis axis--' + tone; };
      return {
        prepare: lt => T.term.prepare(P.map(lt)),
        update(lt) {
          enter(T.win, lt, 0); sc.update(lt, { at: .1 });
          axes.forEach((el, i) => slide(el, lt, .7 + i * .08, { dx: -20 }));
          const after = lt > a[6].outEnd, done = lt > a[1].outEnd;
          const vals = !done ? [['', ''], ['', ''], ['', ''], ['', '']] : after ? [['Blocked', 'red'], ['Ready', 'green'], ['Incomplete', 'orange'], ['Ready', 'green']] : [['Ready', 'green'], ['Ready', 'green'], ['Incomplete', 'orange'], ['Ready', 'green']];
          axes.forEach((el, i) => st(el, vals[i][0], vals[i][1]));
          pop(cx, lt, a[3].outEnd + .2);
          T.term.update({ cam: termCam(T.term, lt, [{ a: a[1].outEnd, b: a[1].holdEnd, text: 'mechanism:', z: 1.4 }, { a: a[3].outEnd, b: a[4].holdEnd, text: 'UnexpectedSuccess', z: 1.4 }, { a: a[6].outEnd, b: a[6].holdEnd + .2, text: 'mechanism: Blocked', z: 1.35 }, { a: a[7].outEnd, b: 99, text: 'UNSIGNED', z: 1.45 }]) });
        },
        cues: () => [...typing(a), { t: a[3].outEnd + .2, type: 'fail' }],
      };
    } },

  // ── Channel 3: review it in the local dashboard ────────────────────────
  { id: 'dashboard', dur: 7, channel: CH.dash, usecase: UC.mig, bg: 'violet', caption: 'eplyx dashboard on loopback over the saved .eplyx/ records · it executes nothing',
    async build(root) {
      const A = await webWindow(root, { clip: 'dashboard', x: 180, y: 140, w: 1340, url: '127.0.0.1:4185/runs/run_…c43467360081' });
      const B = await webWindow(root, { clip: 'dashboard2', x: 180, y: 140, w: 1340, url: '127.0.0.1:4185/compare' });
      const n1 = note(root, { x: 1430, y: 640, w: 420, kicker: 'Stress matrix', title: '19 of 20 cases as specified', body: 'At the deadline the build had to reject the migration. It migrated anyway.' });
      const n2 = note(root, { x: 1430, y: 640, w: 420, kicker: 'Compare two runs', title: 'Ready becomes Blocked', body: 'Reference build against the defect build: 20 of 20 stress cases become 19, and 1 broken rule becomes 4.' });
      const ca = lt => pw(lt, [[0, 5.0], [.6, 6.1], [3.6, 12.4]]), cb = lt => pw(lt, [[3.45, 9.0], [4.6, 11.25], [4.62, 12.29], [7, 15.0]]);
      return {
        async prepare(lt) { await A.foot.prepare(ca(lt)); if (lt > 3.1) await B.foot.prepare(cb(lt)); },
        update(lt) {
          enter(A.win, lt, 0, { out: 3.5 }); enter(B.win, lt, 3.45, { dy: 70 });
          A.foot.update(ca(lt), { cam: keys(lt, [[0, { z: 1, cx: 720, cy: 450 }], [1.2, { z: 1, cx: 720, cy: 450 }], [2.2, { z: 1.4, cx: 740, cy: 520 }], [3.5, { z: 1.4, cx: 740, cy: 520 }]]) });
          B.foot.update(cb(lt), { cam: keys(lt, [[3.5, { z: 1, cx: 720, cy: 450 }], [5.0, { z: 1, cx: 720, cy: 450 }], [5.9, { z: 1.35, cx: 830, cy: 400 }]]) });
          slide(n1, lt, 1.9, { dx: 30, out: 3.4 }); slide(n2, lt, 5.3, { dx: 30 });
        },
        cues: () => [{ t: 0, type: 'whoosh' }, { t: 3.45, type: 'whoosh' }],
      };
    } },

  // ── Scenario: a lifecycle policy change ────────────────────────────────
  { id: 'lifecycle', dur: 6, channel: CH.cli, usecase: UC.life, bg: 'ink', caption: 'Synthetic snapshot and scenario (engine/examples/dashboard_records) · hypothetical policy times',
    async build(root) {
      const T = await termWindow(root, { cast: 'lifecycle', x: 70, y: 300, w: 950, cols: 100, rows: 25, title: 'eplyx · ~/lifecycle' });
      const B = await webWindow(root, { clip: 'dashboard3', x: 1060, y: 250, w: 800, url: '127.0.0.1:4185/runs/run_…2b1efdfb2e8b' });
      const tag = h('div', { class: 'abs persona', style: { left: '80px', top: '118px' } }, h('b', {}, 'Risk team')); root.append(tag);
      const tb = titleBlock(root, { x: 80, y: 158, w: 940, title: 'A deadline moves.\n*What does it mean for holders?', size: 'sm' });
      const chips = pills(root, { x: 1060, y: 175, wrap: 800, items: [{ text: 'PreEvent' }, { text: 'Ready', tone: 'green' }, { text: 'Blocked', tone: 'red' }, { text: 'Incomplete', tone: 'orange' }] });
      const P = plan(T.term, .3, [{ type: .6, out: .15, hold: .3 }, { type: .8, out: .3, hold: 3.0 }]);
      const m2 = mark(T.term, T.term.overlay, 'StaleExposure: 1', { tone: 'orange' });
      const ct = lt => pw(lt, [[0, 1.2], [6, 7.2]]);
      return {
        async prepare(lt) { await T.term.prepare(P.map(lt)); await B.foot.prepare(ct(lt)); },
        update(lt) {
          put(tag, { o: seg(lt, 0, .4) }); tb.update(lt, { at: .05 }); enter(T.win, lt, .1); enter(B.win, lt, 1.0, { dy: 70 }); staggerIn(chips, lt, .5, { step: .08 });
          T.term.update({ cam: termCam(T.term, lt, [{ a: P.at[1].outEnd, b: 99, text: 'Before:', z: 1.3, dy: 120, dx: 250 }]) });
          m2(seg(lt, P.at[1].outEnd + .6, P.at[1].outEnd + 1)); B.foot.update(ct(lt), { pointer: false });
        },
        cues: () => typing(P.at),
      };
    } },

  // ── Scenario + Channel 6: current state, live, in the cloud workspace ──
  { id: 'current', dur: 10, channel: CH.cloud, usecase: UC.cur, bg: 'violet', caption: 'Live mainnet observation of PYUSD for one public owner (public RPC) · the checks run offline in the local VM',
    async build(root) {
      const A = await webWindow(root, { clip: 'current', x: 80, y: 150, w: 1300, url: '127.0.0.1:4390/p/token-transitions/analyse' });
      const B = await webWindow(root, { clip: 'current2', x: 80, y: 150, w: 1300, url: '127.0.0.1:4390/p/token-transitions/analyse?observation=obs_0c8d…' });
      const lv = live(root, 1430, 150);
      const tag = h('div', { class: 'abs persona', style: { left: '1430px', top: '205px' } }, h('b', {}, 'Holder or market maker')); root.append(tag);
      const tb = titleBlock(root, { x: 1430, y: 245, w: 440, title: 'Can this account\n*still move today?', size: 'sm' });
      const steps = [['Observe it, live', 'PYUSD for one public owner, read-only'], ['Transfer 1 PYUSD', 'executed offline: verified'], ['Migrate it to a new token', 'exact checks passed'], ['Funds moved', 'none. Nothing is signed or sent']]
        .map(([t, s], i) => { const el = h('div', { class: 'abs step step--wide', style: { left: '1430px', top: 520 + i * 88 + 'px' } }, h('b', {}, String(i + 1)), h('div', {}, h('strong', {}, t), h('span', {}, s))); root.append(el); return el; });
      const ca = lt => pw(lt, [[0, 4.6], [1.5, 9.0], [2.3, 11.3], [3.4, 13.5], [4.3, 20.4], [5.1, 24.2], [5.8, 27.2], [6.6, 29.5]]);
      const cb = lt => pw(lt, [[6.5, 11.6], [7.6, 15.9], [10, 19.2]]);
      return {
        async prepare(lt) { await A.foot.prepare(ca(lt)); if (lt > 6.1) await B.foot.prepare(cb(lt)); },
        update(lt) {
          enter(A.win, lt, 0, { out: 6.5 }); enter(B.win, lt, 6.45, { dy: 70 });
          A.foot.update(ca(lt), { cam: keys(lt, [[0, { z: 1.15, cx: 640, cy: 420 }], [4.1, { z: 1.15, cx: 640, cy: 420 }], [5.1, { z: 1.3, cx: 560, cy: 420 }]]) });
          B.foot.update(cb(lt), { cam: { z: 1.3, cx: 560, cy: 420 } });
          slide(lv, lt, .2, { dx: 0, dy: -8 }); put(tag, { o: seg(lt, .2, .6) }); tb.update(lt, { at: .25 });
          const on = [2.3, 5.8, 7.8, 8.6];
          steps.forEach((el, i) => { slide(el, lt, .8 + i * .1, { dx: 20 }); el.classList.toggle('step--done', lt > on[i]); });
        },
        cues: () => [{ t: 2.3, type: 'pass' }, { t: 5.8, type: 'pass' }, { t: 7.8, type: 'pass' }],
      };
    } },

  // ── Scenario: a fee change ──────────────────────────────────────────────
  { id: 'fee', dur: 6.5, channel: CH.cli, usecase: UC.fee, bg: 'ink', caption: 'One retained mainnet DepositSol (record 151010f7…) · SOL deposit fee 0 to 1% · manager signer assumed in simulation',
    async build(root) {
      const T = await termWindow(root, { cast: 'fee', x: 70, y: 300, w: 1000, cols: 100, rows: 24, title: 'eplyx · ~/stake-pool-program' });
      const tag = h('div', { class: 'abs persona', style: { left: '80px', top: '118px' } }, h('b', {}, 'Protocol team')); root.append(tag);
      const tb = titleBlock(root, { x: 80, y: 158, w: 1500, title: 'You want to raise the deposit fee to 1%.\n*What does a real deposit receive?', size: 'sm' });
      const bars = h('div', { class: 'abs bars', style: { left: '1130px', top: '330px' } });
      bars.innerHTML = `<div class="bar"><span>Depositor receives at a 0% fee</span><i><em></em></i><b>760,985,008</b></div><div class="bar bar--b"><span>Depositor receives at a 1% fee</span><i><em></em></i><b>753,375,157</b></div><div class="bar bar--c"><span>Manager fee account</span><i><em></em></i><b>+7,609,851</b></div><p>raw pool tokens · reproduced offline: <strong>true</strong></p>`;
      root.append(bars);
      const P = plan(T.term, .3, [{ type: .9, out: .2, hold: .5 }, { type: .55, out: .2, hold: 1.2 }, { type: .7, out: .2, hold: 1.2 }]);
      const m = mark(T.term, T.term.overlay, '"delta_raw": "-7609851"', { tone: 'red' });
      return {
        prepare: lt => T.term.prepare(P.map(lt)),
        update(lt) {
          enter(T.win, lt, 0); put(tag, { o: seg(lt, 0, .4) }); tb.update(lt, { at: .05 });
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

  // ── Scenario: code and fee together ────────────────────────────────────
  { id: 'interaction', dur: 5.5, channel: CH.cli, usecase: UC.inter, bg: 'ink', caption: 'Upgrade candidate is the constructed Step 10B fixture (a664f74b…) · the same retained DepositSol',
    async build(root) {
      const T = await termWindow(root, { cast: 'interaction', x: 70, y: 300, w: 1000, cols: 100, rows: 17, title: 'eplyx · ~/stake-pool-program' });
      const tag = h('div', { class: 'abs persona', style: { left: '80px', top: '118px' } }, h('b', {}, 'Protocol team')); root.append(tag);
      const tb = titleBlock(root, { x: 80, y: 158, w: 1500, title: 'And you are shipping new code at the same time.\n*Does it change what the fee does?', size: 'sm' });
      const grid = h('div', { class: 'abs matrix', style: { left: '1130px', top: '330px' } });
      grid.innerHTML = `<div></div><div class="mh">fee 0%</div><div class="mh">fee 1%</div><div class="mh">fee effect</div>
        <div class="mh">old code</div><div class="mc">baseline</div><div class="mc">−7,609,851</div><div class="mc mc--e">−7,609,851</div>
        <div class="mh">new code</div><div class="mc">0 change</div><div class="mc">−7,609,851</div><div class="mc mc--e">−7,609,851</div>
        <div></div><div></div><div class="mh">difference</div><div class="mc mc--z">0</div>`;
      root.append(grid);
      const res = note(root, { x: 1130, y: 700, w: 720, title: 'No interaction measured.', body: 'The fee has the same effect on both builds, for this real deposit.' });
      const P = plan(T.term, .3, [{ type: 1.5, out: .4, hold: 2.9 }]);
      const m = mark(T.term, T.term.overlay, 'no_measured_interaction', { tone: 'green' });
      return {
        prepare: lt => T.term.prepare(P.map(lt)),
        update(lt) {
          enter(T.win, lt, 0); put(tag, { o: seg(lt, 0, .4) }); tb.update(lt, { at: .05 });
          const o = P.at[0].outEnd;
          [...grid.children].forEach((c, i) => put(c, { o: seg(lt, o + i * .04, o + .3 + i * .04), y: (1 - seg(lt, o + i * .04, o + .4 + i * .04, ease.outQuint)) * 10 }));
          slide(res, lt, o + 1.0, { dx: 0, dy: 14 });
          m(seg(lt, o + .3, o + .7)); T.term.update({ cam: termCam(T.term, lt, [{ a: o + .2, b: 99, text: 'no_measured_interaction', z: 1.35, dy: 40 }]) });
        },
        cues: () => [...typing(P.at), { t: P.at[0].outEnd + 1.0, type: 'pass' }],
      };
    } },

  // ── Channel 6: share results with the team ─────────────────────────────
  { id: 'share', dur: 7, channel: CH.cloud, usecase: UC.share, bg: 'violet', caption: 'Real device-code sign-in approved in the browser · sync uploads saved results, viewing them never re-executes',
    async build(root) {
      const T = await termWindow(root, { cast: 'sync', x: 70, y: 300, w: 920, cols: 100, rows: 22, title: 'eplyx · ~/token-migration' });
      const A = await webWindow(root, { clip: 'device', x: 1030, y: 250, w: 830, url: '127.0.0.1:4390/device?code=…' });
      const B = await webWindow(root, { clip: 'workspace', x: 1030, y: 250, w: 830, url: '127.0.0.1:4390/p/token-transitions/runs' });
      const tag = h('div', { class: 'abs persona', style: { left: '80px', top: '118px' } }, h('b', {}, 'The whole team')); root.append(tag);
      const tb = titleBlock(root, { x: 80, y: 158, w: 1300, title: 'Share local results\n*in one workspace.', size: 'sm' });
      const P = plan(T.term, .3, [{ type: .6, out: .3, hold: 1.2 }, { type: .6, out: .2, hold: .4 }, { type: .5, out: .8, hold: 1.6 }]);
      const ca = lt => pw(lt, [[.5, .9], [2.1, 4.08]]), cb = lt => pw(lt, [[3.6, 1.1], [4.4, 1.1], [4.41, 6.4], [7, 8.8]]);
      return {
        async prepare(lt) { await T.term.prepare(P.map(lt)); await A.foot.prepare(ca(lt)); if (lt > 3.2) await B.foot.prepare(cb(lt)); },
        update(lt) {
          put(tag, { o: seg(lt, 0, .4) }); tb.update(lt, { at: .05 }); enter(T.win, lt, .1); enter(A.win, lt, .5, { dy: 70, out: 3.6 }); enter(B.win, lt, 3.6, { dy: 70 });
          A.foot.update(ca(lt), { cam: { z: 1.35, cx: 720, cy: 300 } }); B.foot.update(cb(lt)); T.term.update();
        },
        cues: () => typing(P.at),
      };
    } },

  // ── Channel 8: the HTTP API ─────────────────────────────────────────────
  { id: 'api', dur: 5, channel: CH.api, usecase: UC.share, bg: 'ink', caption: 'curl with the project token against the local service',
    async build(root) {
      const T = await termWindow(root, { cast: 'api', x: 70, y: 270, w: 1100, cols: 100, rows: 22, title: 'curl · the Eplyx API' });
      const tb = titleBlock(root, { x: 80, y: 130, w: 1400, kicker: 'For your own tooling', title: 'Everything is also *an API.', size: 'sm' });
      const eps = ['POST /checks', 'GET /runs/{id}', 'GET /setup', 'GET /capabilities', 'POST /governance/squads/verify', 'GET /v1/ops'];
      const chips = eps.map((e, i) => { const el = h('div', { class: 'abs endpoint', style: { left: '1240px', top: 290 + i * 84 + 'px' } }, e); root.append(el); return el; });
      const P = plan(T.term, .25, [{ type: .8, out: .2, hold: .9 }, { type: .8, out: .2, hold: 1.3 }]);
      return {
        prepare: lt => T.term.prepare(P.map(lt)),
        update(lt) { enter(T.win, lt, 0); tb.update(lt, { at: 0 }); chips.forEach((c, i) => slide(c, lt, .4 + i * .09, { dx: 30 })); T.term.update(); },
        cues: () => typing(P.at),
      };
    } },

  // ── Honest boundaries ───────────────────────────────────────────────────
  { id: 'boundaries', dur: 7, bg: 'ink',
    async build(root) {
      const tb = titleBlock(root, { x: 80, y: 130, w: 1700, kicker: 'Honest about coverage', title: 'What a result claims, *and what it does not.', size: 'sm' });
      const items = [['Deep coverage today', 'SPL Stake Pool deposits and withdrawals and Token-2022 transfers, plus narrow Kamino and Memo paths.'],
        ['Not looked at', 'is reported as not evaluated. Never as unchanged.'],
        ['A pass', 'holds for the transactions that were replayed. It is not a safety claim.'],
        ['Read-only', 'Eplyx never signs, never sends a transaction and never calls anything safe.']];
      const els = items.map(([k, v], i) => { const el = h('div', { class: 'abs bound', style: { left: '80px', top: 330 + i * 130 + 'px' } }, h('b', {}, k), h('span', {}, v)); root.append(el); return el; });
      return { update(lt) { tb.update(lt, { at: 0 }); els.forEach((el, i) => slide(el, lt, .45 + i * .16, { dx: 0, dy: 16 })); }, cues: () => items.map((_, i) => ({ t: .45 + i * .16, type: 'tick' })) };
    } },

  // ── Vision ──────────────────────────────────────────────────────────────
  { id: 'vision', dur: 6, hud: false, bg: 'violet',
    async build(root) {
      const l1 = h('div', { class: 'abs hook', style: { top: '400px' } }); const w1 = splitWords(l1, 'Before an on-chain change reaches users,', 'w');
      const l2 = h('div', { class: 'abs hook', style: { top: '492px' } }); const w2 = splitWords(l2, 'simulate its real consequences first.', 'w');
      w2.forEach(s => s.classList.add('grad'));
      root.append(l1, l2);
      return { update(lt) { riseWords(w1, lt, .3, { stagger: .07 }); riseWords(w2, lt, 1.2, { stagger: .07 }); }, cues: () => [{ t: .3, type: 'whoosh' }] };
    } },

  // ── End card ────────────────────────────────────────────────────────────
  { id: 'end', dur: 5, hud: false, bg: 'violet',
    async build(root) {
      const mk = h('div', { class: 'abs endmark', html: window.markSVG('endmark__svg') }); root.append(mk);
      const word = h('div', { class: 'abs endword' }, 'Eplyx'); root.append(word);
      const tb = titleBlock(root, { x: 0, y: 660, w: 1920, title: 'Know what changes. *See who is affected.', size: 'sm', align: 'center' });
      const sub = h('div', { class: 'abs endsub' }, 'Deterministic · offline · read-only   ·   github.com/thomasdevving/Eplyx'); root.append(sub);
      return {
        update(lt) {
          const c = mk.querySelector('.mk-c'), w = mk.querySelector('.mk-w');
          put(c, { x: lerp(-50, 0, seg(lt, .1, .9, ease.outQuint)), y: lerp(-30, 0, seg(lt, .1, .9, ease.outQuint)), o: seg(lt, .1, .5) });
          put(w, { x: lerp(50, 0, seg(lt, .25, 1.05, ease.outQuint)), y: lerp(30, 0, seg(lt, .25, 1.05, ease.outQuint)), o: seg(lt, .25, .65) });
          put(mk, { s: lerp(.92, 1, seg(lt, 0, 1.2, ease.out)) });
          put(word, { y: (1 - seg(lt, .7, 1.4, ease.outQuint)) * 24, o: seg(lt, .7, 1.2) });
          tb.update(lt, { at: 1.3 }); put(sub, { o: seg(lt, 2.2, 2.8), y: (1 - seg(lt, 2.2, 2.9, ease.outQuint)) * 10 });
        },
        cues: () => [{ t: .1, type: 'boom' }],
      };
    } },
];
