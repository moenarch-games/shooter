//! Footsteps, landings and positional gunfire through game-audio's director: kira natively, Web
//! Audio in the browser. Gunfire and explosions are spatial cues placed where they happen (the
//! listener is the camera). The clips are generated tones and noise bursts until asset-tooling
//! provides real ones.

use crate::scene::METRES_PER_UNIT;
use game_audio::{CueEvent, Director, DirectorConfig, SoundManifest};
use shooter_sim::{Event, StepReport, Weapon};

const MANIFEST: &str = r#"{
  "attenuation": { "min_distance": 2, "max_distance": 60 },
  "clips": [
    { "id": "step", "source": { "tone": { "frequency_hz": 140, "duration_ms": 55, "waveform": "noise", "amplitude": 0.35 } } },
    { "id": "land", "source": { "tone": { "frequency_hz": 90, "duration_ms": 110, "waveform": "noise", "amplitude": 0.6 } } },
    { "id": "rifle", "source": { "tone": { "frequency_hz": 880, "duration_ms": 50, "waveform": "square", "amplitude": 0.25 } } },
    { "id": "rail", "source": { "tone": { "frequency_hz": 220, "duration_ms": 280, "waveform": "saw", "amplitude": 0.45 } } },
    { "id": "rocket", "source": { "tone": { "frequency_hz": 160, "duration_ms": 140, "waveform": "noise", "amplitude": 0.45 } } },
    { "id": "boom", "source": { "tone": { "frequency_hz": 60, "duration_ms": 450, "waveform": "noise", "amplitude": 0.9 } } },
    { "id": "hit", "source": { "tone": { "frequency_hz": 1400, "duration_ms": 35, "waveform": "sine", "amplitude": 0.3 } } },
    { "id": "hurt", "source": { "tone": { "frequency_hz": 110, "duration_ms": 120, "waveform": "square", "amplitude": 0.4 } } }
  ],
  "cues": [
    { "kind": "player.step", "clips": ["step"], "max_voices": 2, "gain_db": -8, "pitch_jitter": 0.12 },
    { "kind": "player.land", "clips": ["land"], "max_voices": 1, "gain_db": -4, "cooldown_ms": 150 },
    { "kind": "weapon.energyRifle", "clips": ["rifle"], "max_voices": 4, "gain_db": -6, "pitch_jitter": 0.05, "spatial": true },
    { "kind": "weapon.railGun", "clips": ["rail"], "max_voices": 2, "gain_db": -2, "spatial": true },
    { "kind": "weapon.rocketLauncher", "clips": ["rocket"], "max_voices": 2, "gain_db": -4, "spatial": true },
    { "kind": "rocket.explode", "clips": ["boom"], "priority": 70, "max_voices": 4, "pitch_jitter": 0.08, "spatial": true },
    { "kind": "target.hit", "clips": ["hit"], "max_voices": 2, "gain_db": -6, "cooldown_ms": 30 },
    { "kind": "player.hurt", "clips": ["hurt"], "max_voices": 1, "gain_db": -6, "cooldown_ms": 80 }
  ]
}"#;

/// Distance walked on the ground between two footsteps.
const STRIDE_UNITS: f32 = 210.0;

#[cfg(not(target_arch = "wasm32"))]
type Output = Option<game_audio::KiraBackend>;
#[cfg(target_arch = "wasm32")]
type Output = game_audio::WebAudioBackend;

const fn fire_cue(weapon: Weapon) -> &'static str {
    match weapon {
        Weapon::EnergyRifle => "weapon.energyRifle",
        Weapon::RailGun => "weapon.railGun",
        Weapon::RocketLauncher => "weapon.rocketLauncher",
    }
}

fn metres(units: [f32; 3]) -> [f32; 3] {
    units.map(|axis| axis * METRES_PER_UNIT)
}

pub struct Sounds {
    director: Director,
    output: Output,
    walked: f32,
    cues: u64,
}

impl Sounds {
    /// # Panics
    ///
    /// If the built-in manifest does not parse.
    #[must_use]
    pub fn new() -> Self {
        let manifest = SoundManifest::from_json(MANIFEST).expect("the sound manifest parses");
        #[cfg(not(target_arch = "wasm32"))]
        let output = game_audio::KiraBackend::new()
            .inspect_err(|error| eprintln!("audio unavailable: {error}"))
            .ok();
        #[cfg(target_arch = "wasm32")]
        let output = game_audio::WebAudioBackend::new("");
        Self {
            director: Director::new(manifest, DirectorConfig::default()),
            output,
            walked: 0.0,
            cues: 0,
        }
    }

    /// Call from a user gesture: the browser starts audio only after one.
    pub fn unlock(&mut self) {
        #[cfg(target_arch = "wasm32")]
        self.output.unlock();
    }

    fn cue(&mut self, kind: &str, at: Option<[f32; 3]>, now_ms: f64) {
        self.cues += 1;
        let mut event = CueEvent::new(format!("{kind}:{}", self.cues), kind);
        if let Some(at) = at {
            event = event.at(at);
        }
        self.director.trigger(&event, now_ms);
    }

    /// The camera, in metres: where the listener stands and its right ear's direction.
    pub fn listen(&mut self, eye: [f32; 3], right: [f32; 3]) {
        self.director.set_listener(eye, right);
    }

    /// After a tick: `moved` is the horizontal distance (units) the player covered; `local` is
    /// the player's slot (its hits on others tick, hits on it thud).
    pub fn tick(
        &mut self,
        report: StepReport,
        moved: f32,
        events: &[Event],
        local: Option<u8>,
        now_ms: f64,
    ) {
        if report.landed {
            self.walked = 0.0;
            self.cue("player.land", None, now_ms);
        } else if report.grounded && moved > 0.5 {
            self.walked += moved;
            if self.walked >= STRIDE_UNITS {
                self.walked -= STRIDE_UNITS;
                self.cue("player.step", None, now_ms);
            }
        }
        for event in events {
            match *event {
                Event::Fired { weapon, eye, .. } => {
                    self.cue(fire_cue(weapon), Some(metres(eye)), now_ms);
                }
                Event::Exploded { at } => self.cue("rocket.explode", Some(metres(at)), now_ms),
                Event::Damaged { .. } => self.cue("target.hit", None, now_ms),
                Event::Hit {
                    victim, attacker, ..
                } => {
                    if Some(victim) == local {
                        self.cue("player.hurt", None, now_ms);
                    } else if attacker.is_some() && attacker == local {
                        self.cue("target.hit", None, now_ms);
                    }
                }
                Event::Beam { .. }
                | Event::Killed { .. }
                | Event::Spawned { .. }
                | Event::RoundOver { .. } => {}
            }
        }
    }

    /// Sends the decided commands to the device.
    pub fn pump(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(output) = &mut self.output {
            self.director.pump(output);
        } else {
            self.director.drain();
        }
        #[cfg(target_arch = "wasm32")]
        self.director.pump(&mut self.output);
    }
}

impl Default for Sounds {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_manifest_parses() {
        game_audio::SoundManifest::from_json(super::MANIFEST).unwrap();
    }
}
