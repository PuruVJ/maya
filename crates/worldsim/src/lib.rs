//! Headless world-sim core (Rust/WASM) — Phase 0 of the engine port
//! (docs/self-sustaining-world.md §6.6 / §7). The plan: port the existing, stable JS sim
//! (rng → clock → spatial hash → steering/flocking → food-chain) to a Rust core whose state lives in
//! WASM linear memory and whose `tick(dt)` is one call per step, then build the loop-closers (energy /
//! breeding / genome / construction) directly in Rust on top. JS keeps rendering / registration /
//! Mother Nature / the LLM and reads agent transforms back as typed-array views.
//!
//! FIRST module ported here: the squirrel-noise RNG (mirrors src/lib/rng.ts) — the deterministic
//! foundation everything else keys off. All math is pure `u32` wrapping arithmetic, so it is BIT-EXACT
//! with the JS implementation (pinned in `tests` against values produced by rng.ts). That bit-exactness
//! is the whole prize: a world evolves identically in the browser, a Web Worker, and the Cloudflare
//! server tick → replay / time-travel / shared-world all reproduce exactly.

mod rng;

pub use rng::{hash, hash_keys, rand, seed_from};

// Thin wasm-bindgen surface — only compiled for the wasm target. JS calls in for the parity check now,
// and (later) for the per-tick sim. Native `cargo test` skips this entirely.
#[cfg(target_arch = "wasm32")]
mod wasm_api {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub fn rng_hash(position: i32, seed: i32) -> u32 {
        crate::rng::hash(position, seed)
    }
    #[wasm_bindgen]
    pub fn rng_hash_keys(seed: u32, keys: &[i32]) -> u32 {
        crate::rng::hash_keys(seed, keys)
    }
    #[wasm_bindgen]
    pub fn rng_rand(seed: u32, keys: &[i32]) -> f64 {
        crate::rng::rand(seed, keys)
    }
    #[wasm_bindgen]
    pub fn rng_seed_from(s: &str) -> u32 {
        crate::rng::seed_from(s)
    }
}
