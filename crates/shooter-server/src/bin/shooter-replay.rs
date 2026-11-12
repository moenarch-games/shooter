//! Records, verifies and promotes deterministic match replays (docs/foundation-data.md).
//!
//! ```text
//! shooter-replay record [--seed N] [--ticks N] [--people N] [--latency N] [-o FILE]
//! shooter-replay verify FILE
//! shooter-replay promote FILE --target game-server|physics|render [--from T] [--to T] -o OUT
//! ```
//!
//! `record` without `-o` just plays the match (the profiled bot match in CI). `promote` writes
//! `OUT` and, for game-server, `OUT` with a `.txt` manifest beside it. physics takes ticks
//! `--from..=--to`; render takes the frame at `--to`; game-server always starts at tick 0.

use shooter_server::promote;
use shooter_server::replay::{Replay, Script, record, verify};
use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Instant;
use std::{env, fs};

const USAGE: &str =
    "usage: shooter-replay record [--seed N] [--ticks N] [--people N] [--latency N] [-o FILE]
       shooter-replay verify FILE
       shooter-replay promote FILE --target game-server|physics|render [--from T] [--to T] -o OUT";

struct Args {
    positional: Vec<String>,
    flags: HashMap<String, String>,
}

fn parse(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut parsed = Args {
        positional: Vec::new(),
        flags: HashMap::new(),
    };
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        if let Some(name) = arg.strip_prefix("--").or_else(|| arg.strip_prefix('-')) {
            let value = args.next().ok_or(format!("{arg} needs a value"))?;
            parsed.flags.insert(name.to_owned(), value);
        } else {
            parsed.positional.push(arg);
        }
    }
    Ok(parsed)
}

impl Args {
    fn number<T: std::str::FromStr>(&self, name: &str, default: T) -> Result<T, String> {
        self.flags.get(name).map_or(Ok(default), |value| {
            value.parse().map_err(|_| format!("--{name}: not a number"))
        })
    }

    fn output(&self) -> Option<PathBuf> {
        self.flags
            .get("o")
            .or(self.flags.get("output"))
            .map(PathBuf::from)
    }
}

fn load(path: &Path) -> Result<Replay, Box<dyn Error>> {
    Ok(Replay::decode(&fs::read(path)?)?)
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let command = args.next().ok_or(USAGE)?;
    let args = parse(args)?;
    match command.as_str() {
        "record" => {
            let script = Script {
                seed: args.number("seed", Script::DEFAULT.seed)?,
                ticks: args.number("ticks", Script::DEFAULT.ticks)?,
                people: args.number("people", Script::DEFAULT.people)?,
                latency_ticks: args.number("latency", Script::DEFAULT.latency_ticks)?,
            };
            let started = Instant::now();
            let replay = record(script)?;
            let elapsed = started.elapsed();
            let bytes = replay.encode()?;
            println!(
                "{} ticks, {} records, {} bytes, {:.1} ms ({:.3} ms a tick)",
                replay.ticks(),
                replay.log.records().len(),
                bytes.len(),
                elapsed.as_secs_f64() * 1e3,
                elapsed.as_secs_f64() * 1e3 / f64::from(script.ticks.max(1)),
            );
            if let Some(path) = args.output() {
                fs::write(&path, bytes)?;
                println!("wrote {}", path.display());
            }
        }
        "verify" => {
            let path = args.positional.first().ok_or(USAGE)?;
            let verified = verify(&load(Path::new(path))?)?;
            println!(
                "{path}: {} records, {} ticks verified, final tick {}",
                verified.records_verified,
                verified.checkpoints_verified,
                verified.final_snapshot.tick
            );
        }
        "promote" => {
            let path = args.positional.first().ok_or(USAGE)?;
            let replay = load(Path::new(path))?;
            let output = args.output().ok_or("promote needs -o OUT")?;
            let to = args.number("to", replay.ticks())?;
            let from = args.number("from", 0)?;
            let source = format!(
                "shooter-replay promote {} --target {} --from {from} --to {to}",
                Path::new(path)
                    .file_name()
                    .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
                args.flags.get("target").map_or("", String::as_str)
            );
            match args.flags.get("target").map(String::as_str) {
                Some("game-server") => {
                    let (bytes, manifest) = promote::game_server(&replay, to, &source)?;
                    fs::write(&output, bytes)?;
                    fs::write(output.with_extension("txt"), manifest)?;
                }
                Some("physics") => {
                    fs::write(&output, promote::physics(&replay, from, to, &source)?)?;
                }
                Some("render") => fs::write(&output, promote::render(&replay, to, &source)?)?,
                _ => return Err("--target is game-server, physics or render".into()),
            }
            println!("wrote {}", output.display());
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}
