//! Footsteps and landings through game-audio's director: kira natively, Web Audio in the browser.
//! The clips are generated noise bursts until asset-tooling provides real ones.

use game_audio::{CueEvent, Director, DirectorConfig, SoundManifest};
use shooter_sim::StepReport;

const MANIFEST: &str = r#"{
  "clips": [
    { "id": "step", "source": { "tone": { "frequency_hz": 140, "duration_ms": 55, "waveform": "noise", "amplitude": 0.35 } } },
    { "id": "land", "source": { "tone": { "frequency_hz": 90, "duration_ms": 110, "waveform": "noise", "amplitude": 0.6 } } }
  ],
  "cues": [
    { "kind": "player.step", "clips": ["step"], "max_voices": 2, "gain_db": -8, "pitch_jitter": 0.12 },
    { "kind": "player.land", "clips": ["land"], "max_voices": 1, "gain_db": -4, "cooldown_ms": 150 }
  ]
}"#;

/// Distance walked on the ground between two footsteps.
const STRIDE_UNITS: f32 = 210.0;

#[cfg(not(target_arch = "wasm32"))]
type Output = Option<game_audio::KiraBackend>;
#[cfg(target_arch = "wasm32")]
type Output = game_audio::WebAudioBackend;

pub struct Footsteps {
    director: Director,
    output: Output,
    walked: f32,
    cues: u64,
}

impl Footsteps {
    /// # Panics
    ///
    /// If the built-in manifest does not parse.
    #[must_use]
    pub fn new() -> Self {
        let manifest = SoundManifest::from_json(MANIFEST).expect("the footstep manifest parses");
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

    fn cue(&mut self, kind: &str, now_ms: f64) {
        self.cues += 1;
        let key = format!("{kind}:{}", self.cues);
        self.director.trigger(&CueEvent::new(key, kind), now_ms);
    }

    /// After a tick: `moved` is the horizontal distance (units) the player covered.
    pub fn tick(&mut self, report: StepReport, moved: f32, now_ms: f64) {
        if report.landed {
            self.walked = 0.0;
            self.cue("player.land", now_ms);
        } else if report.grounded && moved > 0.5 {
            self.walked += moved;
            if self.walked >= STRIDE_UNITS {
                self.walked -= STRIDE_UNITS;
                self.cue("player.step", now_ms);
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

impl Default for Footsteps {
    fn default() -> Self {
        Self::new()
    }
}
