// Weekly update, week of 4 October 2026: one minute, same film runtime and kit
// as the product demo (media/demo-2026-10/film). Served at /x/ by render.mjs,
// so the shared modules are one level up.
import { lerp, seg, ease, h, put, pw, keys, splitWords, riseWords } from '../core.js';
import { titleBlock, termWindow, webWindow, plan, enter, note, slide, termCam, mark, scenario } from '../kit.js';

// The music follows these scenes (see source/score.py).
export const music = { groove: 'live', break: 'outro', end: 'outro', clap: 'access' };

const typing = at => at.map(a => ({ t: a.typeStart, type: 'typing', dur: a.runAt - a.typeStart })).filter(c => c.dur > .05);
const n = (i, label) => [i, label];

// Fact rows for this film only.
const css = document.createElement('style');
css.textContent = `
.wfact { position: absolute; width: 800px; padding: 18px 0 18px; border-top: 1px solid var(--line2); }
.wfact b { display: block; margin-bottom: 6px; font: 650 25px/1.15 var(--head); letter-spacing: -.02em; color: #fff; }
.wfact span { display: block; font: 400 20px/1.4 var(--body); color: rgba(236,230,247,.78); }
.wnum { position: absolute; font: 650 15px/1 var(--body); letter-spacing: .22em; text-transform: uppercase; color: var(--lilac); }
.wagenda { position: absolute; width: 600px; }
.wagenda div { display: flex; gap: 16px; align-items: baseline; padding: 13px 0; border-top: 1px solid var(--line); font: 500 22px/1.25 var(--body); color: rgba(236,230,247,.86); }
.wagenda b { width: 26px; font: 700 15px/1 var(--mono); color: var(--lilac); }
`;
document.head.append(css);

function facts(root, x, y, rows, { gap = 112, w = 800 } = {}) {
  return rows.map(([k, v], i) => { const el = h('div', { class: 'wfact', style: { left: x + 'px', top: y + i * gap + 'px', width: w + 'px' } }, h('b', {}, k), h('span', {}, v)); root.append(el); return el; });
}

const UPDATES = ['eplyx.dev is live', 'No RPC to set up: one password', 'Rehearse the rollout order', 'Every suite on every pull request', 'Sturdier storage and code'];

export const scenes = [
  // ── The week at a glance: the real git log ──────────────────────────────
  { id: 'intro', dur: 5.5, hud: false, bg: 'ink',
    async build(root) {
      const tb = titleBlock(root, { x: 90, y: 150, w: 640, kicker: 'Weekly update · 4 to 11 October 2026', title: 'This week\nin *Eplyx.' });
      const ag = h('div', { class: 'wagenda', style: { left: '90px', top: '470px' } });
      const rows = UPDATES.map((u, i) => { const d = h('div', {}, h('b', {}, String(i + 1).padStart(2, '0')), u); ag.append(d); return d; });
      root.append(ag);
      const T = await termWindow(root, { cast: 'w-log', x: 790, y: 150, w: 1060, cols: 88, rows: 19, title: 'git · 16 commits on main' });
      const P = plan(T.term, .5, [{ type: .8, out: .5, hold: 9 }]);
      return {
        prepare: lt => T.term.prepare(P.map(lt)),
        update(lt) {
          tb.update(lt, { at: .1 }); enter(T.win, lt, .3, { dy: 50 }); T.term.update();
          rows.forEach((r, i) => slide(r, lt, 1.9 + i * .14, { dx: 0, dy: 14 }));
        },
        cues: () => [...typing(P.at), ...UPDATES.map((_, i) => ({ t: 1.9 + i * .14, type: 'tick' }))],
      };
    } },

  // ── 1. eplyx.dev is live ─────────────────────────────────────────────────
  { id: 'live', dur: 9, channel: n(1, 'eplyx.dev'), usecase: n(1, 'Update 1 of 5'), bg: 'violet',
    caption: 'Recorded from the live https://eplyx.dev on 12 October 2026 · site on Cloudflare Pages, workspace and API on the same domain',
    async build(root) {
      const B = await webWindow(root, { clip: 'live', x: 760, y: 150, w: 1090, url: 'eplyx.dev/cli' });
      const tb = titleBlock(root, { x: 90, y: 160, w: 620, kicker: 'Live', title: 'Eplyx is live\n*on eplyx.dev.' });
      const fs = facts(root, 90, 470, [
        ['Public site', 'served from Cloudflare Pages, with verified routing'],
        ['Workspace and API', 'on the same domain, eplyx.dev/workspace and /v1'],
        ['CLI guide', 'now starts with unlocking the hosted RPC'],
      ], { w: 620 });
      const at = label => B.foot.mark(label);
      const L = at('loaded'), I = at('install'), Tp = at('top'), Ws = at('workspace'), WL = at('workspace loaded'), E = B.foot.meta.frames.at(-1).t;
      const ct = lt => pw(lt, [[0, L - .4], [.6, L + .1], [1.2, I], [5.4, Tp - .7], [6.9, Ws], [7.4, WL], [9, E]]);
      return {
        async prepare(lt) { await B.foot.prepare(ct(lt)); },
        update(lt) {
          tb.update(lt, { at: 0 }); enter(B.win, lt, .2, { dy: 70 });
          B.foot.update(ct(lt), { cam: keys(lt, [[0, { z: 1, cx: 720, cy: 450 }], [1.9, { z: 1, cx: 720, cy: 450 }], [2.9, { z: 1.2, cx: 720, cy: 470 }], [5.2, { z: 1.2, cx: 720, cy: 470 }], [6.1, { z: 1, cx: 720, cy: 450 }]]) });
          B.setUrl(ct(lt) > Ws + .5 ? 'eplyx.dev/workspace' : 'eplyx.dev/cli');
          fs.forEach((el, i) => slide(el, lt, .9 + i * .16, { dx: 0, dy: 16 }));
        },
        cues: () => [{ t: .2, type: 'whoosh' }, ...fs.map((_, i) => ({ t: .9 + i * .16, type: 'tick' }))],
      };
    } },

  // ── 2. The hosted RPC: one password ────────────────────────────────────
  { id: 'access', dur: 8, channel: n(2, 'Local CLI'), usecase: n(2, 'Update 2 of 5'), bg: 'ink',
    caption: 'Real terminal recording against the live eplyx.dev, 12 October 2026 · the password is typed into a hidden prompt',
    async build(root) {
      const T = await termWindow(root, { cast: 'access', x: 70, y: 560, w: 1780, cols: 104, rows: 9, title: 'eplyx · ~' });
      const sc = scenario(root, { x: 90, y: 120, w: 880, persona: 'Every developer', title: 'No RPC to set up.\n*One password.', size: 'sm',
        does: 'Commands that read Solana now go through the Eplyx hosted RPC. Unlock it once with your access password; your own RPC still works and always wins.', doesY: 320 });
      const nt = note(root, { x: 1110, y: 250, w: 740, title: 'A read-only relay.', body: 'An allowlist of read methods only: no send, simulate or airdrop. The provider URL and key never leave the server; a wrong password waits one second.' });
      const P = plan(T.term, .4, [{ type: .6, out: 1.5, hold: 1.2 }, { type: .8, out: .4, hold: 3 }]);
      const a = P.at;
      const m1 = mark(T.term, T.term.overlay, 'Access granted.', { tone: 'green' });
      const m2 = mark(T.term, T.term.overlay, 'Unlocked:', { tone: 'violet' });
      return {
        prepare: lt => T.term.prepare(P.map(lt)),
        update(lt) {
          enter(T.win, lt, .1, { dy: 60 }); sc.update(lt, { at: 0 }); T.term.update();
          m1(seg(lt, a[0].outEnd + .3, a[0].outEnd + .7)); m2(seg(lt, a[1].outEnd + .2, a[1].outEnd + .6));
          slide(nt, lt, 1.2, { dx: 0, dy: 14 });
        },
        cues: () => [...typing(P.at), { t: a[0].outEnd + .3, type: 'pass' }],
      };
    } },

  // ── 3. Rollout rehearsal (Steps 16A and 16B) ────────────────────────────
  { id: 'rollout', dur: 14, channel: n(3, 'Local CLI'), usecase: n(3, 'Update 3 of 5'), bg: 'ink',
    caption: 'Real terminal recording · constructed rollout counterexample, not an upstream release · one retained mainnet DepositSol · signer assumptions explicit · nothing deployed',
    async build(root) {
      const T = await termWindow(root, { cast: 'w-rollout', x: 70, y: 380, w: 1780, cols: 120, rows: 17, title: 'eplyx rollout · ~/stake-pool-program' });
      const tb = titleBlock(root, { x: 90, y: 120, w: 900, kicker: 'New: rollout rehearsal', title: 'Rehearse the order,\n*not just the code.', size: 'sm' });
      const d = h('div', { class: 'does', style: { left: '1060px', top: '128px', width: '790px' } });
      d.append(h('strong', {}, 'What it does'));
      const p = h('span'); const dw = splitWords(p, 'Runs the loader’s real Upgrade inside the retained Stake Pool world, then tries both orders against a fee change. Step 16B adds ExtendProgram when a new build does not fit its ProgramData.', 'w'); d.append(p);
      root.append(d);
      const P = plan(T.term, .4, [{ type: 1.4, out: .5, hold: .5 }, { type: 1.2, out: .5, hold: 2.6 }, { type: .9, out: .4, hold: 4.6 }]);
      const a = P.at;
      const mR = mark(T.term, T.term.overlay, 'set_fee rejected', { tone: 'red' });
      const mV = mark(T.term, T.term.overlay, 'set_fee verified  →  upgrade', { tone: 'green' });
      const mS = mark(T.term, T.term.overlay, 'rollout_order_effect_observed', { tone: 'violet' });
      return {
        prepare: lt => T.term.prepare(P.map(lt)),
        update(lt) {
          tb.update(lt, { at: 0 }); enter(T.win, lt, .1, { dy: 60 });
          d.style.borderTopColor = `rgba(231,221,255,${(.22 * seg(lt, .5, 1.1)).toFixed(3)})`; d.firstChild.style.opacity = seg(lt, .55, .95); riseWords(dw, lt, .65, { stagger: .012, dur: .6 });
          T.term.update({ cam: termCam(T.term, lt, [{ a: a[1].outEnd + .2, b: a[2].typeStart, text: 'order_a', z: 1.22, dx: 120, dy: 20 }]) });
          mR(seg(lt, a[1].outEnd + .5, a[1].outEnd + .9)); mV(seg(lt, a[1].outEnd + 1.1, a[1].outEnd + 1.5)); mS(seg(lt, a[2].outEnd + .3, a[2].outEnd + .7));
        },
        cues: () => [...typing(a), { t: a[1].outEnd + .5, type: 'fail' }, { t: a[1].outEnd + 1.1, type: 'pass' }],
      };
    } },

  // ── 4. CI and operations ────────────────────────────────────────────────
  { id: 'ci', dur: 10.5, channel: n(4, 'CI and operations'), usecase: n(4, 'Update 4 of 5'), bg: 'violet',
    caption: 'Test counts from the commit record (6361faa) · setup and queue views recorded against a local eplyx-server on 7 October 2026',
    async build(root) {
      const tb = titleBlock(root, { x: 90, y: 130, w: 820, kicker: 'Regression CI and operations', title: 'Every pull request,\n*every suite.', size: 'sm' });
      const fs = facts(root, 90, 380, [
        ['GitHub CI', 'every documented suite on each pull request and push to main'],
        ['Public tier', '1,103 tests and clippy on a clean checkout'],
        ['make regression', '1,489 tests and all four browser suites'],
        ['Operations', 'one PR comment per project, a queue view, joint backup and restore'],
      ], { w: 660, gap: 118 });
      const A = await termWindow(root, { cast: 'api', x: 830, y: 150, w: 1030, cols: 100, rows: 11, title: 'Guided first check · GET /setup' });
      const O = await termWindow(root, { cast: 'ops', x: 830, y: 590, w: 1030, cols: 100, rows: 9, title: 'Queue view · eplyx-server admin ops' });
      const PA = plan(A.term, .5, [{ type: 1, out: .4, hold: 9 }]);
      const PO = plan(O.term, 3.4, [{ skip: true }, { type: 1, out: .3, hold: 9 }]);
      return {
        async prepare(lt) { await A.term.prepare(PA.map(lt)); await O.term.prepare(PO.map(lt)); },
        update(lt) {
          tb.update(lt, { at: 0 }); enter(A.win, lt, .2, { dy: 60 }); enter(O.win, lt, 2.9, { dy: 60 });
          A.term.update(); O.term.update();
          fs.forEach((el, i) => slide(el, lt, .8 + i * .2, { dx: 0, dy: 16 }));
        },
        cues: () => [...typing(PA.at), ...typing(PO.at), ...fs.map((_, i) => ({ t: .8 + i * .2, type: 'tick' }))],
      };
    } },

  // ── 5. Hardening and the module split ──────────────────────────────────
  { id: 'hardening', dur: 8, channel: n(5, 'Engine and server'), usecase: n(5, 'Update 5 of 5'), bg: 'ink',
    caption: 'Commits 38ea36c and 9c61e32 · the split moves code only: no route, output, exit code or help text changed',
    async build(root) {
      const tb = titleBlock(root, { x: 90, y: 130, w: 760, kicker: 'Hardening and refactor', title: 'Sturdier\n*underneath.', size: 'sm' });
      const fs = facts(root, 90, 400, [
        ['Write-once storage', 'records are published with fsync and a no-clobber rename'],
        ['Sign-in', 'a token revocation can no longer be overwritten by a stale snapshot'],
        ['Module split', '--help for all 89 command paths byte-identical before and after'],
      ], { w: 660, gap: 122 });
      const T = await termWindow(root, { cast: 'w-modules', x: 830, y: 330, w: 1030, cols: 100, rows: 10, title: 'git · ~/eplyx' });
      const P = plan(T.term, .3, [{ type: .6, out: .2, hold: 1.4 }, { type: 1.1, out: .3, hold: 4 }]);
      const m = mark(T.term, T.term.overlay, '4239 deletions(-)', { tone: 'violet' });
      return {
        prepare: lt => T.term.prepare(P.map(lt)),
        update(lt) {
          tb.update(lt, { at: 0 }); enter(T.win, lt, .1, { dy: 60 }); T.term.update();
          fs.forEach((el, i) => slide(el, lt, .7 + i * .2, { dx: 0, dy: 16 }));
          m(seg(lt, P.at[1].outEnd + .3, P.at[1].outEnd + .7));
        },
        cues: () => [...typing(P.at), ...fs.map((_, i) => ({ t: .7 + i * .2, type: 'tick' }))],
      };
    } },

  // ── Outro ───────────────────────────────────────────────────────────────
  { id: 'outro', dur: 5, hud: false, bg: 'violet',
    async build(root) {
      const mk = h('div', { class: 'abs endmark', html: window.markSVG('endmark__svg') }); root.append(mk);
      const word = h('div', { class: 'abs endword' }, 'Eplyx'); root.append(word);
      const tb = titleBlock(root, { x: 0, y: 660, w: 1920, title: 'Know what changes. *See who is affected.', size: 'sm', align: 'center' });
      const sub = h('div', { class: 'abs endsub' }, 'eplyx.dev   ·   github.com/thomasdevving/Eplyx'); root.append(sub);
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
