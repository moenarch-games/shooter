# Data back to the foundations

The shooter sends its foundation crates data from real matches: replays as fixtures, and
runtime profiles (Atlas D4.2). Every match is scripted and deterministic. Nothing is collected from
real players.

## The loop

1. **Record** a scripted match. `shooter-replay record` hosts a deathmatch through game-server's
   `MatchRuntime` with replay capture on. It has six slots of seeded bots and scripted people
   (`--people`, default 2). The people join one second apart and send one command per tick at a
   fixed input latency (`--latency`, default 6 ticks = 100 ms), so their shots are
   lag-compensated. Every second person leaves three quarters of the way in. The replay file is
   the bot seed followed by game-server's own replay log: admissions, commands, removals and a
   digest of every tick's snapshot.
2. **Verify** it: `shooter-replay verify` replays the log into a fresh match through
   game-server's `verify_replay`, and every tick must match its digest. Recording the same
   script twice gives identical bytes (`crates/shooter-server/src/replay.rs` tests).
3. **Promote** a window into a foundation's fixture format (`shooter-replay promote`):

   | `--target` | Output | For |
   | --- | --- | --- |
   | `game-server` | The log in game-server's replay format, cut after `--to`, plus a `key=value` `.txt` manifest. It always starts at tick 0: the shooter's snapshot is not a restorable baseline | game-server's replay codec and runtime tests (`tests/fixtures/shooter/`) |
   | `physics` | JSON: arena boxes and ramps, the character box with its contact offset and step-up, gravity, and for each tick `--from..=--to` every fighter's intent, the move its step handed the character controller (`move`: the displacement in units, gravity, ground settle and sliding around other fighters included; `null` without a body), and the position, velocity and ground contact after it | a physics-engine A/B scenario, driving each character with `move` |
   | `render` | JSON: one frame at `--to`. It has the camera at a person's eye, the arena, the fighters, rockets in flight, and the last 6 ticks of shots, beams, explosions and hits | a render-engine scene |

4. **Hand it over:** open a PR on the foundation with the fixture and a test, or file an issue
   with the command when the foundation has no format for it yet.
5. **Profile:** `scripts/profile.sh [OUT] [native|web|all]` captures runtime-profiler scenarios
   (default all). The native lane has two.
   `profiles/bot-match.yaml` is about 5 1/2 minutes of match ticks without a window; the script
   also records the same match as a replay. It collects `process` and `native-perf`: the
   bundle's `hotspots.json` holds the sampled hotspots with call paths, and the script renders
   the top ones to `bot-match/hotspots.md`. `native-perf` needs `perf` and
   `kernel.perf_event_paranoid` at 2 or lower (user-space cycles). `profiles/native-client.yaml` runs
   `shooter-client --frames 600`: the native client draws 600 frames (10 s) of a practice match
   offscreen at 1280x720, one tick per frame, with the local fighter played by the replays'
   scripted person 0. The script runs it once more to record its frame times (mean, p50, p95,
   max). `WGPU_FORCE_FALLBACK_ADAPTER=1` picks lavapipe, as CI does. `profile.yml` runs it on
   `main` and on PRs that touch the crates, and uploads `OUT` as the
   `validate-repo-artifacts-profile` artifact. The numbers are evidence only: nothing gates on
   them, and lavapipe frame times measure a CPU rasterizer, not a GPU.

   The web lane builds `web/dist` and captures `profiles/web-client.yaml`, a `browser-journey`
   (collector `browser-chromium`). `scripts/profile-web-journey.mjs` serves `web/dist` on
   loopback and opens `?frames=120`: the page's profile run plays the same scripted practice
   match, one tick per animation frame, in the profiler's 1280x720 headless Chromium. Each frame
   is a `runtime-profiler:js-to-wasm:frame` User Timing measure, so the bundle's trace summary
   lists the frames as JS-to-WASM boundary evidence. When done the page sets
   `window.shooterProfile` to its frame times: `workMs` is the frame's work (tick, scene, GPU
   submission) and `intervalMs` is the time between frames, which includes waiting on the GPU.
   The journey writes it (or the page's error, adapter and last console lines) to
   `web-client/frames.json`. Chromium has no GPU in CI, so WebGPU runs on SwiftShader
   (`--use-webgpu-adapter=swiftshader`; with `--use-angle=swiftshader` as well the GPU process
   died on a hosted runner). The browser run is 120 frames, not 600, because on SwiftShader
   render-engine's particle staging belt fails after about 200 frames
   ([render-engine#108](https://github.com/moritzbrantner/render-engine/issues/108)). A hardware
   adapter runs 600 cleanly. It needs Node, `bun install` (Playwright)
   and `bunx playwright install chromium`. `profile.yml` runs it as a second job and uploads the
   `validate-repo-artifacts-web-profile` artifact.

```sh
cargo run --release -p shooter-server --bin shooter-replay -- record --ticks 1200 -o dm.shrp
cargo run --release -p shooter-server --bin shooter-replay -- verify dm.shrp
cargo run --release -p shooter-server --bin shooter-replay -- promote dm.shrp --target game-server --to 1200 -o deathmatch-v1.replay
cargo run --release -p shooter-server --bin shooter-replay -- promote dm.shrp --target physics --from 300 --to 420 -o window.json
cargo run --release -p shooter-server --bin shooter-replay -- promote dm.shrp --target render --to 900 -o frame.json
cargo run --release -p shooter-client -- --frames 600
scripts/profile.sh                       # all lanes; `scripts/profile.sh target/profile web` for the browser only
```

## Where the data went

| Foundation | State |
| --- | --- |
| game-server | 20 s deathmatch replay fixture with a codec and runtime-shape test: [game-server#84](https://github.com/moritzbrantner/game-server/pull/84) |
| physics-engine | Accepted as the `shooter_window` A/B scenario ([physics-engine#558](https://github.com/moritzbrantner/physics-engine/issues/558)). Its window: `record --seed 33 --ticks 720`, then `promote --target physics --from 580 --to 699` (a rocket blast lifts slot 0 at tick 610, slot 3 launches off a jump pad at tick 634) |
| render-engine | Proposed as a scene fixture: [render-engine#105](https://github.com/moritzbrantner/render-engine/issues/105) |
| runtime-profiler | Pinned at `2110e16` (the `;` in Rust symbols fix, [runtime-profiler#149](https://github.com/moritzbrantner/runtime-profiler/issues/149)). The bot match collects `native-perf` hotspots. The native client stays on `process`: its perf report (lavapipe and wgpu stacks) exceeds the profiler's 8 MiB limit ([runtime-profiler#151](https://github.com/moritzbrantner/runtime-profiler/issues/151)) |

Profiling the browser client is [#28](https://github.com/moenarch-games/shooter/issues/28).
