# Eplyx — product demo film (October 2026)

**[output/eplyx-demo.mp4](output/eplyx-demo.mp4)** · 2:58 · 1920 × 1080 · 30 fps · H.264 + AAC stereo · −14 LUFS

A three-minute walkthrough of the live product: all eight channels and all ten
use cases, built from recordings of the real binaries and services at commit
`b53734b`. It is meant for YouTube, Loom or Vimeo uploads (under three minutes).

## Chapters

| Time | Scene | Channel | Use case |
| --- | --- | --- | --- |
| 0:00 | The homepage intro and hero (recorded), the promise | 01 Public website | — |
| 0:11 | One engine, eight ways in, ten questions | all | all |
| 0:16 | `/start` question chooser, `/runs/demo` saved report | 01 Public website | — |
| 0:25 | Same transaction, same state, two builds | 02 Local CLI | 01 Program upgrade |
| 0:29 | `bundle verify`, control `ci check` (exit 0), regression (exit 1) | 02 Local CLI | 01 Program upgrade |
| 0:40 | `expected-changes.toml` → expected 2, still `undeclarable_change` | 02 Local CLI | 02 Declared changes |
| 0:48 | Exit codes 0–5 as the gate, the repo's own workflow | 04 CI gate · offline | 01 Program upgrade |
| 0:53 | `eplyx-server admin` + operator console setup checklist | 07 Operator console | 10 Share & manage |
| 1:01 | `eplyx-submit.sh` → HTTP 202 → exit 1, hosted report, PR comment | 05 CI gate · hosted | 01 Program upgrade |
| 1:11 | **Live** `governance squads verify` on a real Squads V4 proposal | 02 Local CLI | 03 Governance binding |
| 1:19 | **Live** `versions upgrades/resolve`, `historical acquire`, replay | 02 Local CLI | 09 Historical research |
| 1:29 | `doctor`, `migration analyse/search/reproduce/gate/plan` | 02 Local CLI | 04 Token migration |
| 1:39 | `eplyx dashboard`: failed run, stress matrix, run comparison | 03 Local dashboard | 04 Token migration |
| 1:48 | `lifecycle analyse` + its dashboard page | 02 Local CLI · 03 | 05 Lifecycle change |
| 1:55 | **Live** PYUSD observation, offline Transfer and candidate checks | 06 Cloud workspace | 06 Current state |
| 2:05 | `parameter analyse`: 1% SOL deposit fee on a real DepositSol | 02 Local CLI | 07 Fee parameter |
| 2:13 | `interaction analyse`: does the code change the fee effect? | 02 Local CLI | 08 Upgrade × fee |
| 2:19 | `login` (browser device approval) · `link` · `sync` → workspace runs | 06 Cloud workspace | 10 Share & manage |
| 2:28 | `curl` `/setup` and `/runs` | 08 HTTP API | 10 Share & manage |
| 2:33 | Evidence pipeline; the synthetic 141-fixture method demo | 02 Local CLI | — |
| 2:40 | Five roles and their routes | — | — |
| 2:47 | Boundaries | — | — |
| 2:53 | End card | — | — |

YouTube chapter list (paste into the description):

```text
0:00 Eplyx — know what changes
0:16 Public website
0:25 Program upgrade check (CLI)
0:40 Declared changes
0:48 Offline CI gate
0:53 Operator console
1:01 Hosted CI gate + PR comment
1:11 Squads governance binding (live)
1:19 Historical research (live)
1:29 Token migration rehearsal
1:39 Local dashboard
1:48 Lifecycle change
1:55 Current state in the cloud workspace (live)
2:05 Fee parameter change
2:13 Upgrade × fee interaction
2:19 Sync to the workspace
2:28 HTTP API
2:33 Building the evidence
2:40 Who uses what
2:47 Boundaries
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

`film/index.html` is the editable composition; open it through `render.mjs`'s
server layout (film at `/`, clips at `/clips/`) to scrub with the player.
