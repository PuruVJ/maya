//! The sim's addressed-RNG entry point. Every per-individual / per-tick draw in the sim keys off this single
//! base seed by `(seedId, [tick,] channel)`, so the whole world is a pure function of (seed, tick) — the
//! determinism north star (no stateful streams → order-independent + thread-count-invariant, §6.8). Modules
//! own their own channel constants; this just provides the base + the `rand`/`range` helpers over it.

use crate::rng;

/// = `rng::seed_from("worldgen-agents")` (verified by the rng parity tests) — the live sim's base seed.
pub const BASE_SEED: u32 = 4_204_040_608;

/// A float in [0, 1) at this addressed coordinate.
#[inline]
pub fn rand(keys: &[i32]) -> f64 {
    rng::rand(BASE_SEED, keys)
}

/// A float in [lo, hi) at this addressed coordinate.
#[inline]
pub fn range(lo: f64, hi: f64, keys: &[i32]) -> f64 {
    lo + (hi - lo) * rand(keys)
}
