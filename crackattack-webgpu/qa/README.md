# Visual reference captures

This folder holds the visual reference bench: scripted game runs that capture
screenshots, compared byte for byte against a stored set of hashes. It catches
**visual** regressions that neither `cargo test` nor the render dump can see.

## Running the bench

```sh
qa/run-baseline.sh
```

Run it from anywhere; the script moves to `crackattack-webgpu/` itself. It:

1. empties `qa/current/`;
2. runs the 6 reference scripts (fixed seed 42, fixed time step);
3. compares the **deterministic** captures with `BASELINE_SHA256.txt` **by file name**
   (9 captures: `solo`, `solo-panic`, `shatter`). The autoplay captures of the other scripts
   are written but **not compared** (see below).

The exit code is non-zero if a script fails, a capture is not written, or a hash
differs. Any difference means stop and find the cause before going further.

### Updating the baseline

After a rendering change you have validated by eye, regenerate the manifest:

```sh
(cd qa/current && shasum -a 256 *.png > ../BASELINE_SHA256.txt)
```

Only the manifest is versioned. `qa/baseline/` is an optional local copy of the
images and is git-ignored, like `qa/current/`.

## How captures work

- **Names are deterministic.** `capture <name>` in a script writes
  `<script>-<name>.png` into the campaign folder
  (`--set session:qa.capture_dir=…`, default `<Pictures>/Crack Attack/qa/`).
  A name is required, must be a plain slug, and may appear only once per script.
  An existing file with the same name makes the capture fail, because a stale
  file would falsify the comparison. Run each campaign in an empty folder;
  `run-baseline.sh` does this.
- **Exit waits for the files.** `quit` and `max-frames` wait until every PNG is
  actually on disk (GPU readback and encoding thread; 10 s timeout, then a
  non-zero exit code). No `wait` is needed before `quit`.
- **Seed lock.** A script that captures without an explicit seed is refused
  before the window opens. The default seed is the Unix time, so such captures
  could not be reproduced.
- **Machine-readable summary.** The last log line of a run looks like
  `script: summary — captures planned=… requested=… written=… seed=… surface=…`.
  A missing or failed capture makes the exit code non-zero.
- **Scripts are tested.** `cargo test` parses the 6 reference `.mini` files, so
  a broken script or a grammar change fails in the tests, not in the middle of a
  campaign. `assert flow=playing` lines guard the captures against the game being
  on the wrong screen.
- **Manual capture.** F12 in game keeps its timestamped name in the root capture
  folder; it is not part of the bench.

## What each tool can and cannot see

| Tool | Sees | Does not see |
|---|---|---|
| `cargo test` (naga tests, `.mini` parsing, layouts) | WGSL errors, entry points, broken scripts, ABI asserts | Shader/layout compatibility at runtime, and anything visual |
| `--render-dump` (JSONL) | The render **commands** (draws, data hashes) | The **pixels**: two shaders that render differently give the same dump |
| `--autoplay` alone | Smoke test: it runs and exits cleanly. The AI itself is deterministic for a seed; what varies is the real-time waits of the app | Not deterministic with the normal clock, so useless for comparison |
| **Miniscript + `capture`** (this folder) | The pixels, with a fixed step and seed: comparable byte for byte on the same machine and backend | Whatever the scenario does not reach |
| Real launch | Actual pipeline creation (covers the limit of the naga tests) | — |

## Known pitfalls

- **Same machine, same resolution.** A capture depends on the window size
  (700×700, recorded in the summary) and on the display scale. Never compare
  across machines.
- **Pinned frames depend on the simulation.** Captures such as `shatter` (frame
  1430) are taken at absolute frames. If the simulation changes legitimately,
  probe around the frame with temporary captures, pin the new frame, and
  regenerate the baseline.
- **Autoplay captures are not deterministic, so they are not asserted.** The VS
  scripts run the autoplay, whose transitions depend on real-time waits. Measured: two
  runs of the same binary give different `shader-vs-ia-f1800` and `f2600`;
  `round3-visible` and the `dying-awaking` captures of `-anim` drift the same way. A hash on
  them would make the bench fail at random, so they are out of `BASELINE_SHA256.txt`: the
  scripts still run and write their images (`qa/current/shader-vs-ia-*.png`) for a visual
  check. The `round3` guard (round 3 must be visible, see below) is therefore reviewed by eye,
  not by hash.
- **The manifest depends on the machine.** On a Retina Mac the captures are 1400x1400
  (a 700-point window at scale 2), not the 700x700 recorded in the summary of a
  non-Retina setup, so the hashes recorded on another display never match. This was
  checked by rebuilding the commit that recorded the manifest: its own deterministic
  captures did not match its own hashes on that Mac. Regenerate the manifest on the
  machine you compare on, after checking the images by eye.

- **A script can freeze at launch, with no message.** About one launch in ten to twenty, macOS
  reports the new window as occluded (hidden behind another window, or not yet shown) and no
  "restored" event follows. While `occluded` is set, the main loop skips rendering, and a
  miniscript advances on **rendered** frames (`frame_counter` is incremented when a frame is
  submitted): the script stays frozen until the window becomes visible again. Seen as a run that
  never finishes, with a log that stops right after `Window created` or `RoundStarted(1)` and no
  capture folder. Measured: 4 of 24 launches stalled in one series, 1 of 24 in a series where
  the occlusion events were logged, and the stalled one received `occluded = true` at creation.
  Keep the game window in front and visible during a campaign, and rerun a launch that stalls.
- **A campaign can stop on `captures: drain timed out`.** The exit path waits 10 s for the GPU
  readback of a capture; sometimes the callback never comes and the PNG stays empty (0 bytes),
  on a pair of captures a few frames apart. Seen twice in full campaigns, never in about fifty
  launches of the same scripts afterwards, so the cause is not established (it may be the same
  visibility problem). A 0-byte PNG is a failed capture, never a baseline: rerun the campaign.

## Scripts

| Script | Covers |
|---|---|
| `shader-solo.mini` | Opening curtain, static blocks in 6 colors, swap, level lights, HUD, squash |
| `shader-solo-panic.mini` | Guaranteed panic tint (two phases of the flashing pulse) |
| `shader-vs-ia.mini` (not asserted) | Long match: chains and sparkles, garbage (shells, flavors), inter-round screen |
| `shader-vs-ia-shatter.mini` | Guaranteed garbage shatter (frame 1430) and awaking (frame 1500) |
| `shader-vs-ia-anim.mini` (not asserted) | Blocks caught mid-animation (frames 1515 and 1525): rotating dying shards and awaking rows |
| `shader-vs-ia-round3.mini` (not asserted) | Regression guard for black rounds: round 3 (frame 5600) must be visible. A stale flag used to start every second round behind a closed curtain |

`i18n-controls.mini` is a separate helper, not part of the baseline: it captures
the Controls tab of the options menu so the English and French versions can be
compared by eye (the two captures must differ). Its usage is in its header; the
results are in `qa/i18n-check/`.
