# Eplyx weekly update: 4 to 11 October 2026

**[output/eplyx-weekly-2026-10-04.mp4](output/eplyx-weekly-2026-10-04.mp4)** · 1:00 · 1920 × 1080 · 30 fps · H.264 + AAC stereo

A one-minute summary of what landed on `main` in the week starting Sunday
4 October 2026: 16 commits (`38ea36c` to `ff297cc`, merged to `main` as
`323b9c5`). It uses the same film runtime, kit, design and score generator as
the product demo in [`../demo-2026-10`](../demo-2026-10/README.md); every
terminal and browser image is a recording of the real product.

## Story

| Time | Update | Commits | Evidence on screen |
| --- | --- | --- | --- |
| 0:00 | The week at a glance | all 16 | `git log --oneline --no-merges --since=2026-10-04 origin/main` |
| 0:05 | eplyx.dev is live | `87a7348`, `3c8965f`, `9985a7a`, `4a2d723`, `586d5c0`, `d749e9b`, `66e5ba7` | Live footage of https://eplyx.dev/cli and /workspace |
| 0:14 | No RPC to set up: one password | `ff297cc` (merged `323b9c5`) | `eplyx access` and `--status` against the live eplyx.dev |
| 0:22 | Rehearse the rollout order | `0e90d79` (16A), `dea95d2` (16B) | `eplyx rollout analyse` over the retained DepositSol |
| 0:36 | Every suite on every pull request | `08cc181`, `6361faa`, `b53734b`, `9444513` | `GET /setup` and `eplyx-server admin ops` |
| 0:47 | Sturdier storage and code | `38ea36c`, `9c61e32` | `ls server/src/api/` and the split commit's stat |
| 0:55 | End card | | |

## What is real

- **Terminals** are real interactive `bash` sessions recorded with the demo's
  recorder (`../demo-2026-10/source/pty_record.py`), defined in
  `source/sessions.mjs`. `w-log`, `w-rollout` and `w-modules` were recorded on
  12 October 2026 with the CLI built from `ff297cc` (the week's code). Idle
  gaps over 0.45 s are shortened and typing is played faster, nothing else.
- **Reused recordings** from the product demo: `access` (12 October 2026,
  against the live eplyx.dev; the password was typed into the hidden prompt and
  appears nowhere), `api` and `ops` (7 October 2026, `b53734b`, against a local
  `eplyx-server`).
- **Browser footage** of the live site was recorded on 12 October 2026 with the
  demo's screencast recorder through this environment's proxy
  (`source/capture-live.mjs`); only `eplyx.dev` was reachable from the clip. The
  pointer is drawn from the recorded pointer log.
- **Test counts** (1,103 public-tier tests, 1,489 in `make regression`, four
  browser suites) and the other facts on the fact rows are quoted from the
  commit messages, not re-measured here.

### Labelled as constructed

- The rollout candidate `fixtures/rollout/fixture_stake_pool_rollout_v2.so`
  is a constructed counterexample, not an upstream release. The rehearsal runs
  over one retained mainnet `DepositSol` with explicit signer assumptions and
  deploys nothing.
- The Step 16B ExtendProgram preparation is described, not shown.

Not included: `codex/technical-overview` (`e5fe5ab`, 4 October) is on a branch
and not deployed (https://eplyx.dev/technical-overview answers 404).

## Re-make it

From `../demo-2026-10`:

```sh
node ../weekly-2026-10-04/source/sessions.mjs w-log w-rollout w-modules   # from ../weekly-2026-10-04/source
node ../weekly-2026-10-04/source/capture-live.mjs live <clips-dir>
EPLYX_MEDIA=$PWD/../weekly-2026-10-04 node source/build-casts.mjs
W=$PWD/../weekly-2026-10-04
EPLYX_FILM_EXTRA=$W/film EPLYX_SCENES=/x/scenes.js EPLYX_OUT=$W/output EPLYX_CLIPS=<clips-dir> \
  node source/render.mjs --stills 3,10,20                                  # review
EPLYX_MEDIA=$W python3 source/score.py
EPLYX_FILM_EXTRA=$W/film EPLYX_SCENES=/x/scenes.js EPLYX_OUT=$W/output EPLYX_CLIPS=<clips-dir> \
  EPLYX_VIDEO=eplyx-weekly-master.mp4 EPLYX_AUDIO=$W/output/score.wav node source/render.mjs
```

`w-rollout` needs `~/stake-pool-program` from the demo's `setup-home.sh`, the
rollout candidate in `target/deploy/` and `proposals/rollout-upgrade.json`
(`eplyx change program-upgrade --program SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy
--candidate target/deploy/fixture_stake_pool_rollout_v2.so`). The live clip is
kept in `assets/web/` (`pack-clips.mjs` format) so the film can be re-rendered
without the network.

`output/eplyx-weekly-2026-10-04.mp4` is a two-pass 3.6 Mbit/s encode (SSIM
0.998, −13.9 LUFS) of the CRF 14 master. After the first full render the live
scene was re-rendered on its own (`--range 5.5,14.5`), to cut the blank
page-load frames, and spliced into that master frame-exactly; nothing else
changed.
