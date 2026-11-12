//! The heads-up display, a stopgap until render-engine draws screen overlays
//! (moritzbrantner/render-engine#101): the text is built here once, then shown by the page's DOM
//! in the browser and in the window title natively (the native health bar is drawn in the scene,
//! see `scene::Actors`). Health, weapon, frags and place, the round clock, a kill feed, a respawn
//! or connection banner, and the scoreboard (held Tab, or between rounds).

use shooter_sim::{Controller, Event, FighterState, Phase, Snapshot, TICK_HZ, Weapon};

/// Kill feed lines stay this long (seconds).
const FEED_SECONDS: f32 = 4.0;
const FEED_LINES: usize = 4;

/// What the HUD shows this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HudText {
    /// Health (0..=1) for the bar, when the local fighter is alive.
    pub health: Option<f32>,
    pub status: String,
    pub banner: String,
    pub feed: Vec<String>,
    /// The scoreboard's lines, while it shows.
    pub board: Option<Vec<String>>,
}

#[derive(Default)]
pub struct Hud {
    feed: Vec<(String, f32)>,
}

const fn weapon_name(weapon: Weapon) -> &'static str {
    match weapon {
        Weapon::EnergyRifle => "Energy rifle",
        Weapon::RailGun => "Rail gun",
        Weapon::RocketLauncher => "Rockets",
        Weapon::Grapple => "Grapple",
        Weapon::EnergyBlade => "Energy blade",
        Weapon::Shotgun => "Shotgun",
        Weapon::Laser => "Laser",
        Weapon::Flamethrower => "Flamethrower",
    }
}

fn name(snapshot: &Snapshot, slot: u8, local: Option<u8>) -> String {
    if Some(slot) == local {
        return "You".to_owned();
    }
    match snapshot.slot(slot).map(|fighter| fighter.controller) {
        Some(Controller::Bot) | None => format!("Bot {}", slot + 1),
        Some(Controller::Human(_)) => snapshot
            .name(slot)
            .map_or_else(|| format!("Player {}", slot + 1), str::to_owned),
    }
}

fn clock(ticks: u32) -> String {
    let seconds = ticks / u32::from(TICK_HZ);
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// Fighters by frags (then fewer deaths, then slot).
fn ranking(snapshot: &Snapshot) -> Vec<&FighterState> {
    let mut fighters: Vec<&FighterState> = snapshot.fighters.iter().collect();
    fighters.sort_by_key(|f| (-f.frags, f.deaths, f.slot));
    fighters
}

impl Hud {
    /// Takes in a tick's events (kill feed).
    pub fn events(&mut self, snapshot: &Snapshot, events: &[Event], local: Option<u8>) {
        for event in events {
            let line = match *event {
                Event::Killed {
                    victim,
                    killer,
                    weapon,
                } => match killer.filter(|killer| *killer != victim) {
                    Some(killer) => format!(
                        "{} ⟶ {} ({})",
                        name(snapshot, killer, local),
                        name(snapshot, victim, local),
                        weapon.map_or("?", weapon_name)
                    ),
                    None => format!("{} self-destructed", name(snapshot, victim, local)),
                },
                Event::RoundOver { winner } => format!(
                    "Round over: {} wins",
                    winner.map_or_else(|| "nobody".to_owned(), |w| name(snapshot, w, local))
                ),
                _ => continue,
            };
            self.feed.push((line, 0.0));
        }
    }

    /// The text for this frame, `seconds` after the last.
    pub fn text(
        &mut self,
        snapshot: Option<&Snapshot>,
        local: Option<u8>,
        status: Option<&str>,
        show_board: bool,
        seconds: f32,
    ) -> HudText {
        for (_, age) in &mut self.feed {
            *age += seconds;
        }
        self.feed.retain(|(_, age)| *age < FEED_SECONDS);
        let start = self.feed.len().saturating_sub(FEED_LINES);
        let feed = self.feed[start..]
            .iter()
            .map(|(line, _)| line.clone())
            .collect();
        let Some(snapshot) = snapshot else {
            return HudText {
                banner: status.unwrap_or("waiting for the match").to_owned(),
                feed,
                ..HudText::default()
            };
        };
        let me = local.and_then(|slot| snapshot.slot(slot));
        let ranked = ranking(snapshot);
        let place = me.and_then(|me| ranked.iter().position(|f| f.slot == me.slot));
        let round = match snapshot.phase {
            Phase::Playing { ends_at } => {
                ends_at.map_or_else(String::new, |at| clock(at.saturating_sub(snapshot.tick)))
            }
            Phase::Intermission { .. } => "round over".to_owned(),
        };
        let status_line = me.map_or_else(String::new, |me| {
            format!(
                "♥ {}   {}   {} frags (to {})   #{} of {}   {}",
                me.health.max(0),
                weapon_name(me.held),
                me.frags,
                snapshot.frag_limit,
                place.map_or(0, |place| place + 1),
                ranked.len(),
                round
            )
        });
        let banner = if let Some(status) = status {
            status.to_owned()
        } else if let Phase::Intermission {
            winner,
            restarts_at,
        } = snapshot.phase
        {
            format!(
                "{} won · next round in {}",
                winner.map_or_else(|| "Nobody".to_owned(), |w| name(snapshot, w, local)),
                clock(restarts_at.saturating_sub(snapshot.tick) + u32::from(TICK_HZ) - 1)
            )
        } else if let Some(me) = me.filter(|me| !me.alive) {
            format!(
                "Respawning in {:.1} s",
                f64::from(me.respawn_in) / f64::from(TICK_HZ)
            )
        } else {
            String::new()
        };
        let intermission = matches!(snapshot.phase, Phase::Intermission { .. });
        let board = (show_board || intermission).then(|| {
            ranked
                .iter()
                .enumerate()
                .map(|(index, fighter)| {
                    format!(
                        "{}. {:<16} {:>3} frags {:>3} deaths",
                        index + 1,
                        name(snapshot, fighter.slot, local),
                        fighter.frags,
                        fighter.deaths
                    )
                })
                .collect()
        });
        #[allow(clippy::cast_precision_loss)]
        let health = me
            .filter(|me| me.alive)
            .map(|me| me.health as f32 / shooter_sim::session::MAX_HEALTH as f32);
        HudText {
            health,
            status: status_line,
            banner,
            feed,
            board,
        }
    }
}

/// Shows the HUD in the page (`web/index.html` holds the elements).
#[cfg(target_arch = "wasm32")]
pub fn show(text: &HudText, last: &mut HudText) {
    if text == last {
        return;
    }
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let set = |id: &str, value: &str| {
        if let Some(element) = document.get_element_by_id(id) {
            element.set_text_content(Some(value));
        }
    };
    set("hud-status", &text.status);
    set("hud-banner", &text.banner);
    set("hud-feed", &text.feed.join("\n"));
    set(
        "hud-board",
        &text.board.as_deref().unwrap_or_default().join("\n"),
    );
    if let Some(element) = document.get_element_by_id("hud-health") {
        let width = format!("width: {:.1}%", text.health.unwrap_or(0.0) * 100.0);
        let _ = element.set_attribute("style", &width);
    }
    *last = text.clone();
}

/// Shows the HUD natively: the window title (the health bar is in the scene).
#[cfg(not(target_arch = "wasm32"))]
pub fn show(text: &HudText, last: &mut HudText, window: &winit::window::Window) {
    if text == last {
        return;
    }
    let mut parts = vec![text.status.clone()];
    if !text.banner.is_empty() {
        parts.push(text.banner.clone());
    }
    if let Some(line) = text.feed.last() {
        parts.push(line.clone());
    }
    if let Some(board) = &text.board {
        parts.push(board.join(" | "));
    }
    window.set_title(&format!("shooter — {}", parts.join("   ·   ")));
    *last = text.clone();
}
