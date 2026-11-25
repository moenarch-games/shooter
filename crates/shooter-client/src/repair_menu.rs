//! The quick repair menu (shooter#61, Atlas D2.i70.9), the same natively and in the browser: the
//! repair action (R by default, [`crate::controls::REPAIR`]) opens it over the match, which keeps
//! running; it lists only the parts the local robot may repair now (damaged, still connected,
//! in reach of a working repair module: the simulation's [`Fighter::repairable`], as predicted
//! online) and, while a repair runs, a way to stop it. The wheel and Q/E move through it while it
//! is open (they step through the weapons again once it closes); the repair action again
//! repairs the chosen part and closes it. Movement, aim and fire stay with the person
//! throughout. The part under the cursor (or, with the menu closed, the part under repair) is
//! lit on the robot ([`RepairMenu::highlight`]).
//!
//! The order goes to the host like a weapon switch: asked for [`RepairMenu::ASK_TICKS`] ticks
//! (inputs are unreliable datagrams; asking again for the part under repair changes nothing),
//! and the host decides: it refuses a part that is no longer eligible.

use crate::builder::part_name;
use shooter_sim::repair::{Order, Repair};
use shooter_sim::robot::{BETA, PartId};
use shooter_sim::session::Fighter;

/// One line of the menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// A part it may repair: its ID, its name and its health of its whole.
    Part {
        id: PartId,
        name: String,
        health: u16,
        whole: u16,
    },
    /// Stop the repair under way.
    Stop,
}

impl Entry {
    const fn order(&self) -> Order {
        match self {
            Self::Part { id, .. } => Order::Start(*id),
            Self::Stop => Order::Stop,
        }
    }
}

/// `fighter`'s part `id`, named for the menu: its catalog name and its ID (two legs differ).
fn named(fighter: &Fighter, index: usize) -> Option<Entry> {
    let part = fighter.build.parts().get(index)?;
    let kind = BETA.part_type(part.kind)?;
    Some(Entry::Part {
        id: part.id,
        name: format!("{} {}", part_name(kind), part.id.0),
        health: *fighter.parts.health().get(index)?,
        whole: kind.health,
    })
}

/// The menu's lines for `fighter` now: stop (while it repairs), then every part it may repair,
/// in part-ID order.
#[must_use]
pub fn entries(fighter: &Fighter) -> Vec<Entry> {
    let eligible = fighter.repairable();
    let parts = (0..fighter.build.parts().len())
        .filter(|index| eligible & 1 << index != 0)
        .filter_map(|index| named(fighter, index));
    fighter
        .repair
        .map(|_| Entry::Stop)
        .into_iter()
        .chain(parts)
        .collect()
}

/// What the HUD draws of field repair this frame ([`crate::overlay::repair_panel`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepairView {
    /// The menu's lines and the cursor's, while it is open.
    pub menu: Option<(Vec<Entry>, usize)>,
    /// The repair under way and its part's line.
    pub active: Option<(Repair, Entry)>,
    /// How many parts it may repair now (the hint while the menu is closed).
    pub repairable: u32,
}

/// The menu between ticks: open or not, the cursor and the order being asked for.
#[derive(Clone, Debug, Default)]
pub struct RepairMenu {
    open: bool,
    /// The line under the cursor, kept by what it is while the list changes.
    cursor: Option<Order>,
    /// The order and the ticks it has been asked for.
    asked: Option<(Order, u32)>,
}

impl RepairMenu {
    /// Half a second: well over a round trip.
    pub const ASK_TICKS: u32 = 30;

    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.open
    }

    /// The cursor's line in `entries` (the first part when its line is gone).
    fn at(&self, entries: &[Entry]) -> usize {
        self.cursor
            .and_then(|cursor| entries.iter().position(|entry| entry.order() == cursor))
            .or_else(|| entries.iter().position(|entry| *entry != Entry::Stop))
            .unwrap_or(0)
    }

    /// One local tick: `presses` of the repair action and `steps` of the weapon cycle (taken
    /// for the cursor while the menu is open, else left to the weapons) for the local fighter
    /// (`None` while it is dead or not playing: the menu closes). This tick's repair order.
    pub fn tick(
        &mut self,
        fighter: Option<&Fighter>,
        presses: u32,
        steps: &mut i32,
    ) -> Option<Order> {
        let Some(fighter) = fighter else {
            *self = Self::default();
            return None;
        };
        let entries = entries(fighter);
        for _ in 0..presses {
            if self.open {
                if let Some(entry) = entries.get(self.at(&entries)) {
                    self.asked = Some((entry.order(), 0));
                }
                self.open = false;
            } else if !entries.is_empty() {
                self.open = true;
                self.cursor = None;
            }
        }
        if self.open && entries.is_empty() {
            self.open = false;
        }
        if self.open {
            let steps = std::mem::take(steps);
            if steps != 0 {
                let count = i32::try_from(entries.len()).unwrap_or(i32::MAX);
                let at = i32::try_from(self.at(&entries)).unwrap_or(0);
                let moved = usize::try_from((at + steps).rem_euclid(count)).unwrap_or(0);
                self.cursor = entries.get(moved).map(Entry::order);
            }
        }
        let (order, asked) = self.asked.as_mut()?;
        *asked += 1;
        let order = *order;
        if *asked >= Self::ASK_TICKS {
            self.asked = None;
        }
        Some(order)
    }

    /// The part lit on the local robot: the one under the cursor while the menu is open, else
    /// the one under repair.
    #[must_use]
    pub fn highlight(&self, fighter: &Fighter) -> Option<PartId> {
        if self.open {
            let entries = entries(fighter);
            return match entries.get(self.at(&entries))? {
                Entry::Part { id, .. } => Some(*id),
                Entry::Stop => fighter.repair.map(|repair| repair.target),
            };
        }
        fighter.repair.map(|repair| repair.target)
    }

    /// What the HUD shows of it for `fighter`.
    #[must_use]
    pub fn view(&self, fighter: &Fighter) -> RepairView {
        let entries = entries(fighter);
        let active = fighter.repair.and_then(|repair| {
            let index = fighter
                .build
                .parts()
                .iter()
                .position(|part| part.id == repair.target)?;
            Some((repair, named(fighter, index)?))
        });
        RepairView {
            menu: self.open.then(|| {
                let at = self.at(&entries);
                (entries, at)
            }),
            active,
            repairable: fighter.repairable().count_ones(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shooter_sim::robot::MECHANIC;
    use shooter_sim::{Arena, Intent, Session};

    /// The practice match's local fighter as the mechanic, its leg (5) and rifle (7) worn.
    fn worn() -> Session {
        let mut session = Session::new(Arena::greybox()).unwrap();
        session.rebuild(0, MECHANIC).unwrap();
        session.tick(Intent::IDLE).unwrap();
        let build = session.fighters[0].build;
        session.fighters[0].parts.hurt(&build, 2, 20);
        session.fighters[0].parts.hurt(&build, 4, 10);
        session
    }

    #[test]
    fn the_menu_lists_only_eligible_parts_and_orders_the_chosen_one() {
        let mut session = worn();
        let me = session.local();
        let listed: Vec<PartId> = entries(me)
            .iter()
            .filter_map(|entry| match entry {
                Entry::Part { id, .. } => Some(*id),
                Entry::Stop => None,
            })
            .collect();
        assert_eq!(listed, [PartId(5), PartId(7)]);
        let mut menu = RepairMenu::default();
        // Closed, the steps are the weapons'.
        let mut steps = 1;
        assert_eq!(menu.tick(Some(me), 0, &mut steps), None);
        assert_eq!(steps, 1);
        // R opens it on the first part, lit on the robot; a step moves to the next.
        let mut steps = 0;
        assert_eq!(menu.tick(Some(me), 1, &mut steps), None);
        assert!(menu.is_open());
        assert_eq!(menu.highlight(me), Some(PartId(5)));
        let mut steps = 1;
        menu.tick(Some(me), 0, &mut steps);
        assert_eq!(steps, 0, "the menu took the step");
        assert_eq!(menu.highlight(me), Some(PartId(7)));
        // R again orders it, closes the menu and keeps asking for a while.
        let mut steps = 0;
        assert_eq!(
            menu.tick(Some(me), 1, &mut steps),
            Some(Order::Start(PartId(7)))
        );
        assert!(!menu.is_open());
        for _ in 1..RepairMenu::ASK_TICKS {
            assert_eq!(
                menu.tick(Some(me), 0, &mut steps),
                Some(Order::Start(PartId(7)))
            );
        }
        assert_eq!(menu.tick(Some(me), 0, &mut steps), None);
        // The order through the practice match: the rifle is under repair and lit.
        let order = Intent {
            repair: Some(Order::Start(PartId(7))),
            ..Intent::IDLE
        };
        session.tick(order).unwrap();
        let me = session.local();
        assert_eq!(me.repair.unwrap().target, PartId(7));
        assert_eq!(menu.highlight(me), Some(PartId(7)));
        // While it repairs, the menu offers to stop it.
        assert_eq!(entries(me).first(), Some(&Entry::Stop));
        let view = menu.view(me);
        assert!(view.menu.is_none());
        assert!(matches!(
            view.active,
            Some((_, Entry::Part { id: PartId(7), .. }))
        ));
    }

    #[test]
    fn nothing_to_repair_keeps_the_menu_shut_and_death_closes_it() {
        let mut session = Session::new(Arena::greybox()).unwrap();
        session.rebuild(0, MECHANIC).unwrap();
        session.tick(Intent::IDLE).unwrap();
        let mut menu = RepairMenu::default();
        let mut steps = 0;
        menu.tick(Some(session.local()), 1, &mut steps);
        assert!(!menu.is_open(), "nothing is damaged");
        let session = worn();
        menu.tick(Some(session.local()), 1, &mut steps);
        assert!(menu.is_open());
        menu.tick(None, 0, &mut steps);
        assert!(!menu.is_open());
    }
}
