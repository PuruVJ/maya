//! The UTILITY SCORER (design doc §3 Tier 1) — the heart of emergent mode. Each feasible primitive is scored
//! by how much it would relieve the agent's most-pressing needs, WEIGHTED by that individual's behaviour
//! genome; the highest score wins. There is NO hand-authored priority order — "flee beats eat beats wander"
//! is not coded, it FALLS OUT of the numbers: a coward (high `safety` weight) flees a distant hunter a bold
//! glutton (high `food`, low `safety`) would keep eating through. That divergence, inherited + selected, is
//! where strategies emerge.
//!
//! Pure function of `(needs, genome, options)` → deterministic, no RNG, no per-tick state. Hysteresis lives
//! upstream in the metabolism latches (`hungry`) and the mobbing latch the manual sim already tunes, so the
//! scorer itself stays stateless; the small wander floor keeps an option-less agent from twitching.

use super::genome::Genome;
use super::needs::Needs;
use super::primitives::{Options, Primitive};

/// Score every feasible primitive and return the winner. Ties resolve by the candidate order below (safety-
/// first), which only matters when two scores are bit-equal — rare, and deterministic when it happens.
pub fn choose(needs: &Needs, g: &Genome, o: &Options) -> Primitive {
    // a hunter bearing down close is near-existential → the ×(1+frac) term lets a real threat dominate even a
    // starving agent's food drive, while a hunter merely on the horizon (low frac) leaves room to keep foraging.
    let flee = if o.threat || o.bully {
        let urgency = if o.bully { 1.0 } else { o.threat_frac };
        needs.safety * g.safety * (0.6 + 1.6 * urgency)
    } else {
        0.0
    };
    // a predator with prey in sight presses the attack HARDER the closer it gets (the manual "close → lunge"):
    // the commit bonus lets a hungry hunter drive through a mob/standoff for an adjacent kill instead of fleeing.
    let hunt = if o.prey { needs.hunger * g.food * (1.0 + COMMIT_GAIN * o.prey_close) } else { 0.0 };
    let scavenge = if o.carrion { needs.hunger * g.food * 0.7 } else { 0.0 }; // leftovers rank below a fresh kill
    let menace = if o.menace_player { needs.hunger * g.food * 0.85 } else { 0.0 };
    let rival = if o.rival { 0.7 + 0.3 * g.safety.recip() } else { 0.0 }; // bolder apexes pick fights sooner
    let drink = if o.fish { 0.28 } else { 0.0 }; // a low idle curiosity — any real business outscores it
    let follow = if o.fellow { needs.social * g.social * 0.55 } else { 0.0 };
    let rest = if o.exhausted { needs.rest * g.rest } else { 0.0 };
    let wander = WANDER_FLOOR; // the default baseline — graze / roam when nothing scores higher

    // candidate order = the tie-break (safety first). Strict `>` keeps the FIRST max, so ties favour earlier.
    let candidates = [
        (Primitive::Flee, flee),
        (Primitive::Hunt, hunt),
        (Primitive::Scavenge, scavenge),
        (Primitive::MenacePlayer, menace),
        (Primitive::RivalFight, rival),
        (Primitive::Rest, rest),
        (Primitive::Follow, follow),
        (Primitive::Drink, drink),
        (Primitive::Wander, wander),
    ];
    let mut best = Primitive::Wander;
    let mut best_score = f64::NEG_INFINITY;
    for (p, s) in candidates {
        if s > best_score {
            best_score = s;
            best = p;
        }
    }
    best
}

const WANDER_FLOOR: f64 = 0.16; // baseline utility of just roaming — sets how easily a weak drive overrides idling
const COMMIT_GAIN: f64 = 2.0; // how hard prey-proximity boosts Hunt → a near-adjacent kill beats fleeing the mob
