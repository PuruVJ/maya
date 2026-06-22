//! The PRIMITIVE verb library (design doc §3 Tier 1) — the small, composable action vocabulary the utility
//! scorer chooses among each tick. Each verb already half-exists in the manual brain; here they're named,
//! first-class options scored by how much they'd relieve the agent's most-pressing needs. Complex behaviour
//! is meant to EMERGE from which verb wins when (a thirsty idle agent near water → `Drink`; a settled,
//! well-fed pair → `Settle`), not from a hand-authored priority chain.
//!
//! `Options` is the per-agent FEASIBILITY snapshot — which verbs are even available this tick (is there prey
//! in reach? a carcass? a fellow to follow?) — gathered once from the shared perception pass and handed to
//! the scorer, so scoring stays a pure function of (needs, genome, options).

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Primitive {
    Flee,         // run from the nearest hunter (people curve toward home/refuge)
    Hunt,         // a hungry carnivore stalks + catches live prey
    Scavenge,     // a hungry carnivore feeds on a fresh carcass
    MenacePlayer, // a lone hungry apex pressures the player (non-lethal)
    RivalFight,   // an apex turns on a crowding same-rank rival (territory)
    Drink,        // an idle cat pads to the water's edge after a fish (never catches)
    Follow,       // close ranks toward a fellow of its kind (gather / herd)
    Rest,         // a spent predator stops to recover (then the metabolism pass dozes it off)
    Wander,       // the default — graze / roam (herbivores refuel by staying calm here)
}

/// Which verbs are feasible for this agent this tick (filled from the shared perception pass).
#[derive(Clone, Copy, Default)]
pub struct Options {
    pub threat: bool,       // a hunter has marked this agent as prey
    pub threat_frac: f64,   // 0..1 how close that hunter is (eased) → urgency of fleeing
    pub bully: bool,        // freshly lost a rival fight → still fleeing its bully
    pub prey: bool,         // a live prey target is in reach (carnivore)
    pub prey_close: f64,    // 0..1 how close that prey is → the commit/lunge bonus on Hunt
    pub carrion: bool,      // a fresh carcass is in reach (hungry carnivore)
    pub menace_player: bool,// a lone hungry apex with the player in range + closer than its prey
    pub rival: bool,        // a same-rank apex has crowded it past its patience
    pub fish: bool,         // an idle cat is near a lake fish
    pub fellow: bool,       // a same-kind neighbour to gather toward
    pub exhausted: bool,    // a carnivore run to empty stamina with nothing pressing
}
