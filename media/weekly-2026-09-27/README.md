# Eplyx — The week in motion

## Revised cut — faster, dynamic, explanatory

**[Watch V2](watch-v2.html) · [V2 MP4](output/eplyx-weekly-2026-09-27-v2.mp4) · [Editable V2 composition](movie-v2.html)**

V2 is **2:08 at 1920 × 1080 / 30 fps**. It replaces the static frontend sequences with recorded browser sessions: the native homepage intro, live hero motion, dashboard loading, page navigation, typing, clicks, the Technical toggle, copying a reproduction command, selecting runs to compare, observation capture, queued checks, and completed results. Persistent three-step guides explain the input, engine action and meaning of the result. It retains the original purple palette, music, archival governance evidence and four-pane ending.

The browser recordings use the actual local frontend/service with the committed synthetic provider and fixtures. Native animation is enabled. The capture browser emulates 70–220 ms network latency, making real loading states visible without changing API data. The pointer and click ring are capture overlays tied to actual browser events. Input timing and scrolling are edited; lengthy queued waits are condensed and labeled. They are not benchmarks. No status or result is fabricated.

The original recordings are `assets/motion/*.webm`; their editable H.264 intermediates are `assets/motion/*.mp4`. `assets/motion/capture-log.json` records action timestamps and capture settings. No login credentials are included. `source/capture-motion.mjs` generates these recordings, and `source/film-v2.js` maps actual source time to editorial time. The native Orca/Drift and governance component captures retain the provenance documented below.

V2's timeline is `output/story-v2.json`. Re-render it from existing footage with:

```sh
EPLYX_CHROME=/path/to/chromium FFMPEG=/path/to/ffmpeg \
  node media/weekly-2026-09-27/source/render-v2.mjs
```

Use `--stills` for the 34 review frames. The final export is decoded end to end; its measured properties and moving-frame checks are recorded in `output/verification-v2.json`. V1 remains available for comparison.

## Original cut

**2:20 · 1920 × 1080 · 30 fps · H.264 MP4 · stereo AAC**

Open [the finished film](output/eplyx-weekly-2026-09-27.mp4). The [editable browser composition](movie.html) includes play/pause and seeking. The film has an original electronic score, burned-in editorial captions, and an optional English subtitle track; it has no voice-over.

## Edit

| Time | Sequence |
| --- | --- |
| 0:00 | The actual Eplyx homepage hero |
| 0:08 | The week's three product areas |
| 0:14 | Historical replay, native v0, evidence, causal checkpoints |
| 0:26 | Actual Orca and Drift economic impact components |
| 0:36 | Real hosted upgrade job, ChangeSpec, impact view |
| 0:48 | Squads proposal binding and deployment attestation |
| 1:00 | Public transition page |
| 1:06 | Real CLI output and local migration dashboard |
| 1:18 | Stress cases, search, authorities and state drift |
| 1:28 | Saved counterexample, reproduction and comparison |
| 1:38 | Lifecycle policy, readiness and current paths |
| 1:48 | Actual browser observation, candidate check and scenario |
| 2:00 | Four-pane CLI / dashboard / hosted / lifecycle workflow |
| 2:14 | Animated brand close |

## What is real

The product assets, fonts, colors and UI come from this repository at `a3eac9d`. Screenshot pixels are from the unmodified website, the actual `eplyx dashboard` binary, the actual local hosted service, or the production report/governance components rendering retained engine evidence. The motion graphics, framing, cursor cue and explanatory captions are editorial additions.

- **Replay:** the Orca CLI and JSON report were freshly executed offline with the committed historical bundle and candidate. Drift uses its committed U14 report. Both native impact cards use the production `HostedReport` renderer; their capture harness supplies a display wrapper, not a new analytical result.
- **Governance:** the native `GovernanceSection` uses the sealed G1 mainnet binding and G2 `active-not-executed.json`. Its display timestamp is set to September 25 to identify archived evidence. The video does not claim a real `deployed_match`.
- **Upgrades:** the local hosted service actually executed the committed controlled Stake Pool candidate. The captured failure is its real result.
- **Migration:** dashboard screens use the committed synthetic transition-acceptance fixtures. CLI excerpts come from a fresh run of the minimal reference fixture. The reference and deficient-reserve candidates are different examples and are labeled accordingly. The strict reference gate failed with incomplete population evidence; exporting an unsigned plan succeeded.
- **Lifecycle:** the saved lifecycle page is a hypothetical declaration. The current-state browser flow runs against the repository's synthetic local provider. Observe, offline Transfer, candidate and scenario actions were actually submitted to the local service and awaited. These are not live-provider qualification or deployment claims.
- **CLI typography:** long identifiers are abbreviated, the command's run ID is represented as `$RUN`, and the strict-gate result is condensed. Full original stdout, stderr and exit codes remain in `assets/cli-recording.json` and `assets/cli-transcript.txt`.

Individual screen sources are recorded in `assets/capture-manifest.json`; retained assets have SHA-256 entries in `assets/checksums.json`. Original font licenses accompany the fonts. Music is synthesized by `source/soundtrack.py` without external samples.

## Re-render

Requires the repository's Node / Playwright dependencies, Chromium, Python with NumPy, and FFmpeg with libx264/AAC. Existing captures are sufficient; no app server or provider is required for rendering.

```sh
python3 media/weekly-2026-09-27/source/soundtrack.py
EPLYX_CHROME=/path/to/chromium FFMPEG=/path/to/ffmpeg \
  node media/weekly-2026-09-27/source/render.mjs
```

Use `--stills` to render the review frames only. `source/film.js` holds the deterministic timeline and animation; `movie.html` is its standalone player. The renderer binds only to loopback on port 4414 while it runs.

Capturing new product screens requires the local public server, dashboard fixture servers, and the repository's cloud fixture service; see `source/capture.mjs`, `source/capture-details.mjs`, and `source/record-cli.mjs`. The cloud seed credentials remain in temporary storage and are not included in this directory.

## Verification

The final movie is decoded end to end, its runtime and frame count are checked, and representative frames are extracted from the encoded MP4 for visual review. `output/qa` retains composition review frames. See `output/verification.json` for the export's measured properties.
