# Eplyx product demo film (October 2026)

**[output/eplyx-demo.mp4](output/eplyx-demo.mp4)** · 2:57 · 1920 × 1080 · 30 fps · H.264 + AAC stereo · −14 LUFS

A three-minute film of the live product for YouTube, Loom or Vimeo. It explains
the problem, what Eplyx is and how it works, who it is for, and then follows one
person per scenario: who they are, what situation they are in, what Eplyx does
for them and what they get back. Together the scenarios cover all eight channels
and all ten use cases. Every terminal and browser image is a recording of the
real binaries and services at commit `b53734b`.

## Story

| Time | Who | Situation | Channel | Use case |
| --- | --- | --- | --- | --- |
| 0:00 | | You know what you changed in the code, but not what it will do to real users | | |
| 0:10 | | How Eplyx works: a simulation rebuilt from real mainnet data, old and new side by side | | |
| 0:26 | | Who it is for | | |
| 0:32 | Anyone | Start with your question on the website, or open a saved report | Public website | |
| 0:39 | Protocol developer | You are about to ship a new build | Local CLI | Program upgrade |
| 0:53 | Protocol developer | Some changes are on purpose | Local CLI | Declared changes |
| 1:00 | Protocol developer | In CI, the exit code is the gate | CI gate, offline | Program upgrade |
| 1:05 | Platform team | Set it up once; CI can never move the baseline | Operator console | Share and manage |
| 1:13 | Protocol developer | Every pull request, checked automatically | CI gate, hosted | Program upgrade |
| 1:23 | Multisig signer | You are asked to approve a program upgrade (**live**) | Local CLI | Governance binding |
| 1:32 | Security team | Something changed on mainnet: what ran, and can you replay it (**live**) | Local CLI | Historical research |
| 1:42 | Token issuer | You are moving holders to a new token | Local CLI | Token migration |
| 1:52 | Token issuer | Review the rehearsal in the local dashboard | Local dashboard | Token migration |
| 1:59 | Risk team | A deadline moves: what does it mean for holders | Local CLI | Lifecycle change |
| 2:05 | Holder or market maker | Can this account still move today (**live**) | Cloud workspace | Current state |
| 2:15 | Protocol team | You want to raise the deposit fee to 1% | Local CLI | Fee change |
| 2:21 | Protocol team | And new code ships at the same time | Local CLI | Upgrade and fee together |
| 2:27 | The whole team | Share local results in one workspace | Cloud workspace | Share and manage |
| 2:34 | Your tooling | Everything is also an API | HTTP API | Share and manage |
| 2:39 | | What a result claims, and what it does not | | |
| 2:46 | | Before an on-chain change reaches users, simulate its real consequences first | | |

YouTube chapter list (paste into the description):

```text
0:00 The problem
0:10 How Eplyx works
0:26 Who it is for
0:32 Try it in the browser
0:39 Checking a new program build
0:53 Declaring intended changes
1:00 The CI gate
1:05 Setting it up once
1:13 Every pull request, checked
1:23 Approving a Squads upgrade (live)
1:32 Replaying mainnet history (live)
1:42 Rehearsing a token migration
1:52 The local dashboard
1:59 A lifecycle change
2:05 Can this account still move? (live)
2:15 A fee change
2:21 Code and fee together
2:27 Sharing results
2:34 The API
2:39 What a result claims
2:46 The vision
```

## What is real

Every terminal and browser pixel is a recording of the actual product. The motion
graphics frame, label and point at those recordings; they never stand in for one.

- **Terminals** are real interactive `bash` sessions in a pseudo-terminal
  (`source/pty_record.py`). Each command was typed into the shell, executed by the
  release binaries built from `b53734b`, and every output byte and exit code was
  stored as an asciicast (`assets/casts/*.cast`). The film replays those bytes
  through xterm.js. Two edits are applied and nothing else: idle gaps longer than
  0.45 s are shortened (`source/build-casts.mjs`), and typing/output are played
  faster than real time. The prompt's red `✗ 1` is bash's own exit status.
- **Browser footage** is the Chrome DevTools screencast of the unmodified local
  services (`source/recorder.mjs`, `source/capture-web.mjs`): the public site on
  `:4173`, `eplyx-server` with its embedded cloud workspace on `:4390` (scratch
  Postgres, loopback only), and `eplyx dashboard` on `:4185`. The pointer is drawn
  by the film from the logged pointer events of each recording. Long waits
  (page loads, queued jobs) are cut.
- **The homepage intro** was captured deterministically under virtual time
  (`source/deterministic.mjs`) because its WebGL scene renders too slowly in a
  software-GL container to record in real time; the animation itself is the
  site's own.
- **SBF programs** come from the repository's `Regression` CI run on `b53734b`
  (artefact `sbf-artifacts`, zip SHA-256 `5f51cfd3…`). The pinned hashes match:
  `fixture_stake_pool_v2.so` `3193eabd…`, `fixture_stake_pool_config_v2.so`
  `a664f74b…`, `eplyx_token_migration.so` `e5db6948…`.

### Live mainnet reads (7 October 2026)

- `governance squads verify` on the committed G1.1 witness (Squads #1 of
  `8fJvcw…`) via `api.mainnet-beta.solana.com`: `matched` at slot 454,211,406
  (finalized). A match is a statement at that slot.
- `versions upgrades/resolve` and `historical acquire` of the real PYUSD
  TransferChecked `3omP6i…` through the archive endpoint the repository's own
  demo script uses (`solana-mainnet.g.alchemy.com/v2/docs-demo`). The real
  Token-2022 upgrade replays with no behavioural difference; the last replay
  uses the constructed `fixture_token2022_v2.so` regression.
- Hosted current-state observation of PYUSD for one public owner (the largest
  holder account at capture time) via the public RPC. The Transfer and candidate
  checks then ran offline in the local VM. Nothing was signed or sent; signing
  possession is not established.

### Labelled as synthetic or constructed

- Regression candidates (`fixture_stake_pool_v2`, `fixture_token2022_v2`,
  `fixture_stake_pool_config_v2`, the deadline-defect migration build) are
  locally constructed counterexamples, not upstream releases.
- The migration world is `examples/migrations/minimal` (a synthetic fixture).
- The lifecycle snapshot/scenario come from `engine/examples/dashboard_records`
  (synthetic, hypothetical policy times).
- `eplyx compare` over the 141-fixture lending corpus is the method demo.
- "Hosted" means a local `eplyx-server`, not a production deployment. The
  pull-request comment shown is the body `scripts/eplyx-pr-comment.py`'s own
  `compose()` produced from the submit client's summary
  (`assets/pr-comment.md`); nothing was posted to GitHub.

### Editorial

Titles, callouts, highlights, camera moves, transitions, HUD and the end card are
editorial. The score and sound design are synthesized by `source/score.py` (no
samples); typing clicks are placed on the recorded typing intervals.

## Re-make it

1. Build `eplyx`, `eplyx-server` and `examples/dashboard_records` (release) and
   place the CI `sbf-artifacts` in `artifacts/`.
2. Start a scratch Postgres on `127.0.0.1:54329`, then `eplyx-server` and the
   public frontend (`EPLYX_API_URL=http://127.0.0.1:4390 PORT=4173 node
   frontend/dev-server.mjs`). The film's run used
   `EPLYX_OBSERVATION_RPC_URL`/`EPLYX_GOVERNANCE_RPC_URL` set to the public RPC.
3. `source/setup-home.sh /home/demo`, `node source/seed-workspace.mjs`, then
   record in this order: `node source/sessions.mjs operator`, `capture-web.mjs
   console`, `sessions.mjs upgrade expectations hosted api`, the browser clips,
   `sessions.mjs ops migration lifecycle dashboard`, `record-sync.sh`, and the
   remaining sessions. Credentials stay in `/srv/eplyx-demo`, outside the
   repository; session files keep tokens masked.
4. `node source/build-casts.mjs`, `node source/render.mjs --stills 30,60` to
   review, `python3 source/score.py`, then
   `EPLYX_AUDIO=output/score.wav node source/render.mjs` (writes the master `output/eplyx-demo-video.mp4`; the committed `output/eplyx-demo.mp4` is its upload encode).

To re-render from this directory alone (no services, no recordings), restore the
browser footage first: `node source/pack-clips.mjs unpack /tmp/eplyx-clips` and
then `EPLYX_CLIPS=/tmp/eplyx-clips node source/render.mjs …`. `assets/web/` keeps
every recorded frame at its original timestamp (H.264 4:4:4, CRF 14) with the
pointer log; `pack-clips.mjs pack` rebuilds it from raw captures.

`output/eplyx-demo.mp4` is a two-pass 3.9 Mbit/s encode of the CRF 16 master
(SSIM 0.998 against the master), kept under GitHub's 100 MB file limit.

`film/index.html` is the editable composition; open it through `render.mjs`'s
server layout (film at `/`, clips at `/clips/`) to scrub with the player.
