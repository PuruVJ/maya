//! The NEEDS substrate (design doc §3 Tier 1) — per-agent drives in 0..1 that the utility scorer reads. We
//! don't store a slow-drifting need vector on every agent (the existing `energy`/`stamina`/`health` already
//! ARE the slow-drifting state, maintained by the shared metabolism pass); instead `assess` reads those plus
//! this tick's perception into the four needs the starter primitive set cares about. Keeping needs DERIVED
//! (a pure function of existing state) means emergent mode reuses the metabolism the manual sim is tested on,
//! and adds no new persistent fields to keep in sync.

/// How pressing each drive is right now (0 = satisfied … 1 = urgent).
#[derive(Clone, Copy, Debug)]
pub struct Needs {
    pub hunger: f64, // empty belly → forage / hunt / scavenge
    pub safety: f64, // a hunter bearing down → flee
    pub rest: f64,   // spent stamina → stop and recover
    pub social: f64, // alone → seek its own kind (gather / herd)
}

impl Needs {
    /// Derive the four needs from an agent's metabolic state + this tick's perception.
    /// `threat_frac` is 0 (no threat) … 1 (a hunter right on top of it), already eased by distance.
    /// `is_carnivore` gates the rest-drive (only predators sleep off exhaustion in this sim); `hungry` is the
    /// carnivore's LATCHED hunting drive (the manual sim's hysteresis flag) — a predator hunts on that latch,
    /// not on a near-full belly, so its hunger need keys off it (else it'd never choose to chase until starving).
    pub fn assess(energy: f64, stamina: f64, threat_frac: f64, crowd: u32, is_carnivore: bool, hungry: bool) -> Needs {
        let hunger = if is_carnivore {
            // a hunting-ready (latched) carnivore has a STRONG drive to chase; keener still as its belly empties
            if hungry {
                (0.85 + 0.15 * (1.0 - energy)).clamp(0.0, 1.0)
            } else {
                (0.2 * (1.0 - energy)).clamp(0.0, 1.0) // sated → little drive (it'll coast / rest)
            }
        } else {
            (1.0 - energy).clamp(0.0, 1.0) // herbivores/people: fullness IS the drive (grazing relieves it)
        };
        // safety is the eased threat proximity — 0 when nothing hunts it, rising as the hunter closes
        let safety = threat_frac.clamp(0.0, 1.0);
        // rest only matters to predators here (prey/people rest-recover passively in the metabolism pass)
        let rest = if is_carnivore { (1.0 - stamina).clamp(0.0, 1.0) } else { 0.0 };
        // social rises when an agent finds itself with few neighbours → a drive to close ranks (gather/herd)
        let social = (1.0 - crowd as f64 / SOCIAL_QUORUM).clamp(0.0, 1.0);
        Needs { hunger, safety, rest, social }
    }
}

/// Flock-neighbour count at/above which the social drive is fully satisfied (it has company).
const SOCIAL_QUORUM: f64 = 4.0;
