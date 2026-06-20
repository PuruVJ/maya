//! Headless world-sim core (Rust/WASM) — Phase 0 of the engine port
//! (docs/self-sustaining-world.md §6.6 / §7). The plan: port the existing, stable JS sim
//! (rng → clock → spatial hash → steering/flocking → food-chain) to a Rust core whose state lives in
//! WASM linear memory and whose `tick(dt)` is one call per step, then build the loop-closers (energy /
//! breeding / genome / construction) directly in Rust on top. JS keeps rendering / registration /
//! Mother Nature / the LLM and reads agent transforms back as typed-array views.
//!
//! The sim runs BROWSER-ONLY (main thread or, at scale, a Web Worker — §6.5/§6.8); Cloudflare stores the
//! durable deltas, it does NOT run the sim. Bit-exact determinism is what lets any visiting client
//! fast-forward a dormant region from the same `(seed, state, lastTick)` to the IDENTICAL world (§6.9),
//! and is the enabler for later thread-count-invariant multithreading (§6.8).
//!
//! Per §6.9 the eventual core is a UNIFIED entity buffer — organisms, trees, AND houses share one
//! struct-of-arrays lifecycle with a `kind` discriminant; near regions full-sim, far ones collapse to
//! aggregate fields realized on demand. (Not built yet — the port lands the pure modules first.)
//!
//! FIRST module ported here: the squirrel-noise RNG (mirrors src/lib/rng.ts) — the deterministic
//! foundation everything else keys off, and (addressed by `(tick, id, channel)`) the exact thing that
//! makes order-independent parallelism bit-identical. Pure `u32` wrapping arithmetic → BIT-EXACT with the
//! JS implementation (pinned in `tests` against values produced by rng.ts).

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
