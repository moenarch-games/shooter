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
   | `physics` | JSON: arena boxes and ramps, the character box, gravity, and for each tick `--from..=--to` every fighter's intent, position, velocity and ground contact | a physics-engine A/B scenario |
   | `render` | JSON: one frame at `--to`. It has the camera at a person's eye, the arena, the fighters, rockets in flight, and the last 6 ticks of shots, beams, explosions and hits | a render-engine scene |

4. **Hand it over:** open a PR on the foundation with the fixture and a test, or file an issue
   with the command when the foundation has no format for it yet.
5. **Profile:** `scripts/profile-bot-match.sh [OUT]` captures `profiles/bot-match.yaml`, about
   5 1/2 minutes of match ticks, with runtime-profiler. It also records the same match as a
   replay. `profile.yml` runs it on `main` and on PRs that touch the simulation, and uploads `OUT`
   as the `validate-repo-artifacts-bot-match-profile` artifact. The numbers are evidence only:
   nothing gates on them.

```sh
cargo run --release -p shooter-server --bin shooter-replay -- record --ticks 1200 -o dm.shrp
cargo run --release -p shooter-server --bin shooter-replay -- verify dm.shrp
cargo run --release -p shooter-server --bin shooter-replay -- promote dm.shrp --target game-server --to 1200 -o deathmatch-v1.replay
cargo run --release -p shooter-server --bin shooter-replay -- promote dm.shrp --target physics --from 300 --to 420 -o window.json
cargo run --release -p shooter-server --bin shooter-replay -- promote dm.shrp --target render --to 900 -o frame.json
scripts/profile-bot-match.sh
```

## Where the data went

| Foundation | State |
| --- | --- |
| game-server | 20 s deathmatch replay fixture with a codec and runtime-shape test: [game-server#84](https://github.com/moritzbrantner/game-server/pull/84) |
| physics-engine | Proposed as an A/B scenario, ask-first: [physics-engine#558](https://github.com/moritzbrantner/physics-engine/issues/558) |
| render-engine | Proposed as a scene fixture: [render-engine#105](https://github.com/moritzbrantner/render-engine/issues/105) |
| runtime-profiler | The `native-perf` collector fails on perf 7.x ([runtime-profiler#146](https://github.com/moritzbrantner/runtime-profiler/issues/146)), so the profile uses `process` only |

Profiling the windowed native client and the browser client is
[#21](https://github.com/moenarch-games/shooter/issues/21).
