# Eplyx product demo film (October 2026)

**[output/eplyx-demo.mp4](output/eplyx-demo.mp4)** · 2:59 · 1920 × 1080 · 30 fps · H.264 + AAC stereo · −14 LUFS

A three-minute film of the live product for YouTube, Loom or Vimeo. It explains
the problem, what Eplyx is and how it works, who it is for, and then follows one
person per scenario: who they are, what situation they are in, what Eplyx does
for them and what they get back. Together the scenarios cover all eight channels
and all ten use cases. Every terminal and browser image is a recording of the
real binaries and services at commit `b53734b`, except the hosted-RPC access step
and the Squads verification, which were recorded on 12 October 2026 with a CLI
built from `d7326cc` (main's password-gated hosted Solana RPC, `323b9c5`)
against the live eplyx.dev.

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
| 1:12 | Protocol developer | Every pull request, checked automatically | CI gate, hosted | Program upgrade |
| 1:21 | Every developer | No RPC to set up: one password unlocks the Eplyx hosted RPC | Local CLI | |
| 1:27 | Multisig signer | You are asked to approve a program upgrade (**live**, through the hosted RPC) | Local CLI | Governance binding |
| 1:36 | Security team | Something changed on mainnet: what ran, and can you replay it (**live**) | Local CLI | Historical research |
| 1:46 | Token issuer | You are moving holders to a new token | Local CLI | Token migration |
| 1:56 | Token issuer | Review the rehearsal in the local dashboard | Local dashboard | Token migration |
| 2:03 | Risk team | A deadline moves: what does it mean for holders | Local CLI | Lifecycle change |
| 2:09 | Holder or market maker | Can this account still move today (**live**) | Cloud workspace | Current state |
| 2:19 | Protocol team | You want to raise the deposit fee to 1% | Local CLI | Fee change |
| 2:25 | Protocol team | And new code ships at the same time | Local CLI | Upgrade and fee together |
| 2:31 | The whole team | Share local results in one workspace | Cloud workspace | Share and manage |
| 2:38 | Your tooling | Everything is also an API | HTTP API | Share and manage |
| 2:43 | | What a result claims, and what it does not | | |
| 2:49 | | Before an on-chain change reaches users, simulate its real consequences first | | |

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
1:12 Every pull request, checked
1:21 No RPC to set up, one password
1:27 Approving a Squads upgrade (live)
1:36 Replaying mainnet history (live)
1:46 Rehearsing a token migration
1:56 The local dashboard
2:03 A lifecycle change
2:09 Can this account still move? (live)
2:19 A fee change
2:25 Code and fee together
2:31 Sharing results
2:38 The API
2:43 What a result claims
2:49 The vision
```

## What is real

Every terminal and browser pixel is a recording of the actual product. The motion
graphics frame, label and point at those recordings; they never stand in for one.

- **Terminals** are real interactive `bash` sessions in a pseudo-terminal
  (`source/pty_record.py`). Each command was typed into the shell, executed by the
  release binaries built from `b53734b` (`d7326cc` for the `access` and
  `governance` sessions), and every output byte and exit code was
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

### Hosted RPC and access password (12 October 2026)

- `eplyx access` (CLI built from `d7326cc`, which carries main's hosted RPC
  from `323b9c5`) was run against its default server, the live
  `https://eplyx.dev`, with no `EPLYX_URL` set. The provider behind the
  deployed relay is the operator's and is not named here. The recorder types
  the password into the hidden prompt from its own `EPLYX_ACCESS_PASSWORD`
  (`answer.env` in `source/sessions.mjs`); the recorded shell never receives
  that variable, `eplyx access` turns echo off, and the recorder refuses to keep
  a cast that contains the password. Every cast, session file and film cast was
  scanned for it afterwards with zero hits.

### Live mainnet reads

- 12 October 2026: `governance squads verify` on the committed G1.1 witness
  (Squads #1 of `8fJvcw…`) with no `--rpc-url` and no `SOLANA_RPC_URL`, read
  through the Eplyx hosted RPC on eplyx.dev using the access saved by the
  `access` session: `matched` at slot 455,986,845 (finalized), proposal status
  Active. A match is a statement at that slot.
- 7 October 2026: `versions upgrades/resolve` and `historical acquire` of the
  real PYUSD TransferChecked `3omP6i…` through the archive endpoint the
  repository's own demo script uses (`solana-mainnet.g.alchemy.com/v2/docs-demo`),
  configured as the session's own `SOLANA_RPC_URL`. These reads need exact-slot
  account history. On 12 October 2026 the provider behind eplyx.dev refused
  `versions resolve` at slot 427,147,035 (JSON-RPC `-32099`), so this scene was
  not re-recorded through the hosted RPC; its caption says so. The
  real Token-2022 upgrade replays with no behavioural difference; the last
  replay uses the constructed `fixture_token2022_v2.so` regression.
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
   The `access` and `governance` sessions use the live eplyx.dev: run
   `sessions.mjs access governance` with `EPLYX_ACCESS_PASSWORD` set in the
   recorder's environment and no `~/.config/eplyx/access.json` in the demo
   home. `EPLYX_RPC_BIN` overrides the CLI build they use (default: this
   checkout's `target/release`).
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
