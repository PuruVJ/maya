//! The food-chain entity config + per-individual birth rolls — port of the `ECO` / `PREY_PRIZE` tables and
//! `speedFor` / aggressive / `slashMax` rolls from `agents.svelte.ts`. The data is a compile-time table here;
//! the spec wants it eventually data-driven (a JS/JSON config the Rust reads, for instant balancing) — that's
//! a later seam. The rolls are deterministic via the addressed RNG (birth draws key by `(seedId, channel)`),
//! matching the JS bit-for-bit (no transcendentals → exact; verified in tests against captured JS values).
//!
//! rank = trophic level (higher eats lower). hunts: Lower = eats anything below its rank · Humans = an
//! aggressive person hunts its own kind · None = pure prey. full_after = kills before a food-coma; sleep_secs
//! = that nap's length; mob_toll = how many swarmers it slashes dead before being dragged down.

use crate::simrng::{rand, range};

// birth channels (key by [seedId, CH]); match the JS `CH = {speed:2, aggro:3, slash:4}`. Distinct from
// steering's birth channels (10-13) and its per-tick channels (different key arity → no collision).
const CH_SPEED: i32 = 2;
const CH_AGGRO: i32 = 3;
const CH_SLASH: i32 = 4;

const AGGRO_PROB: f64 = 0.02; // share of people that turn aggressive (hunt their own kind) — rare flavour, not a population sink

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Rabbit,
    Cat,
    Kangaroo,
    Person,
    Lion,
    Dinosaur,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hunts {
    None,
    Lower,
    Humans,
}

#[derive(Clone, Copy, Debug)]
pub struct Eco {
    pub rank: u8,
    pub speed_lo: f64,
    pub speed_hi: f64,
    pub endurance: f64,
    pub hunts: Hunts,
    pub full_after: Option<u32>,    // hunters only — kills before a food-coma nap
    pub sleep_secs: Option<f64>,    // that nap's length (fallback DEFAULT_SLEEP_SECS otherwise)
    pub mob_toll: Option<(u32, u32)>, // attackers it slays while mobbed before falling
}

pub const DEFAULT_SLEEP_SECS: f64 = 10.0;

/// Stable u8 code ↔ Kind, for the JS bridge (JS passes a small int per spawn). Order = the enum order.
pub const fn kind_from_code(code: u8) -> Kind {
    match code {
        0 => Kind::Rabbit,
        1 => Kind::Cat,
        2 => Kind::Kangaroo,
        3 => Kind::Person,
        4 => Kind::Lion,
        _ => Kind::Dinosaur,
    }
}

/// The eco profile for a kind (the `ECO` table).
pub const fn eco(kind: Kind) -> Eco {
    match kind {
        // RABBIT: the fastest BOLTER but the worst stamina (user: "higher speed, but tire faster") — it explodes away,
        // then GASES OUT fast, so a steadier predator runs it down once its sprint is spent. Was top speed + endurance
        // 1.0 (never tired) → it fled forever, near-uncatchable. Speed up, endurance way down (1.0 → 0.4).
        Kind::Rabbit => Eco { rank: 1, speed_lo: 4.0, speed_hi: 5.2, endurance: 0.4, hunts: Hunts::None, full_after: None, sleep_secs: None, mob_toll: None },
        // Predators were SLOWER than their prey (cat 3.0–3.9 vs rabbit 3.6–4.8): even with the chase-vs-flee boost
        // the fastest prey outran the fastest hunter, so carnivores starved amid 40 rabbits (telemetry: all starve
        // victims were cats/lions, only 3 kills). Bumped so a committed chase reliably runs down an AVERAGE prey,
        // while the fastest (high-vigor) still occasionally escape — predation works, selection still bites.
        Kind::Cat => Eco { rank: 2, speed_lo: 3.5, speed_hi: 4.5, endurance: 0.8, hunts: Hunts::Lower, full_after: None, sleep_secs: Some(10.0), mob_toll: Some((1, 2)) },
        Kind::Kangaroo => Eco { rank: 2, speed_lo: 3.4, speed_hi: 4.6, endurance: 0.9, hunts: Hunts::None, full_after: None, sleep_secs: None, mob_toll: None },
        Kind::Person => Eco { rank: 3, speed_lo: 1.8, speed_hi: 2.5, endurance: 0.6, hunts: Hunts::Humans, full_after: None, sleep_secs: None, mob_toll: None },
        // apex: a touch faster than the cat AND more stamina (0.4→0.55) so a lion can sustain a chase to the kill.
        Kind::Lion => Eco { rank: 4, speed_lo: 3.7, speed_hi: 4.8, endurance: 0.55, hunts: Hunts::Lower, full_after: Some(5), sleep_secs: Some(16.0), mob_toll: Some((1, 3)) },
        Kind::Dinosaur => Eco { rank: 5, speed_lo: 4.8, speed_hi: 6.2, endurance: 0.3, hunts: Hunts::Lower, full_after: Some(9), sleep_secs: Some(24.0), mob_toll: Some((2, 5)) },
    }
}

/// How desirable a kind is as PREY (the `PREY_PRIZE` table) — weighed against distance when a hunter picks.
pub const fn prize(kind: Kind) -> f64 {
    match kind {
        Kind::Rabbit => 0.7,
        Kind::Cat => 1.0,
        Kind::Kangaroo => 1.4,
        Kind::Lion => 1.8,
        Kind::Person => 2.0,
        Kind::Dinosaur => 2.6,
    }
}

pub fn sleep_secs(kind: Kind) -> f64 {
    eco(kind).sleep_secs.unwrap_or(DEFAULT_SLEEP_SECS)
}

/// A per-individual max speed in this kind's range (varies every individual; deterministic by seedId).
pub fn speed_for(kind: Kind, seed_id: i32) -> f64 {
    let e = eco(kind);
    range(e.speed_lo, e.speed_hi, &[seed_id, CH_SPEED])
}

/// Whether a person is aggressive (hunts its own kind) — a seeded coin-flip. (Only meaningful for Person.)
pub fn aggressive(seed_id: i32) -> bool {
    rand(&[seed_id, CH_AGGRO]) < AGGRO_PROB
}

/// This individual's ferocity — attackers it can slay in one mob fight (from mob_toll), seeded per individual.
pub fn slash_max(kind: Kind, seed_id: i32) -> u32 {
    match eco(kind).mob_toll {
        Some((lo, hi)) => lo + (rand(&[seed_id, CH_SLASH]) * (hi - lo + 1) as f64).floor() as u32,
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eco_table() {
        assert_eq!(eco(Kind::Cat).rank, 2);
        assert_eq!(eco(Kind::Dinosaur).speed_hi, 6.2);
        assert_eq!(eco(Kind::Lion).mob_toll, Some((1, 3)));
        assert_eq!(eco(Kind::Lion).full_after, Some(5));
        assert_eq!(eco(Kind::Rabbit).hunts, Hunts::None);
        assert_eq!(eco(Kind::Person).hunts, Hunts::Humans);
        assert_eq!(eco(Kind::Cat).hunts, Hunts::Lower);
        assert_eq!(prize(Kind::Person), 2.0);
        assert_eq!(sleep_secs(Kind::Rabbit), DEFAULT_SLEEP_SECS); // no explicit → fallback
        assert_eq!(sleep_secs(Kind::Dinosaur), 24.0);
    }

    // birth rolls captured from agents.svelte.ts's rng (BASE seed "worldgen-agents", CH speed=2/aggro=3/slash=4)
    #[test]
    fn birth_roll_parity() {
        let close = |a: f64, b: f64| assert!((a - b).abs() < 1e-5, "expected ~{b}, got {a}");
        close(speed_for(Kind::Cat, 100), 3.640601);
        close(speed_for(Kind::Dinosaur, 7), 4.844612);
        close(speed_for(Kind::Rabbit, 12345), 4.69204); // range bumped [3.6,4.8]→[4.0,5.2] (faster bolter); width 1.2 unchanged → +0.4
        assert!(!aggressive(5));
        assert!(!aggressive(50));
        assert!(!aggressive(3));
        assert_eq!(slash_max(Kind::Cat, 100), 1);
        assert_eq!(slash_max(Kind::Lion, 7), 1);
        assert_eq!(slash_max(Kind::Dinosaur, 7), 2);
        assert_eq!(slash_max(Kind::Rabbit, 7), 0); // no mob_toll → 0
    }
}
