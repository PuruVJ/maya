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

mod clock;
mod eco;
mod rng;
mod simrng;
mod spatialhash;
mod world;
mod steering;

pub use clock::{SimClock, DT};
pub use eco::{aggressive, eco, kind_from_code, prize, sleep_secs, slash_max, speed_for, Eco, Hunts, Kind, DEFAULT_SLEEP_SECS};
pub use rng::{hash, hash_keys, rand, seed_from};
pub use spatialhash::SpatialHashGrid;
pub use steering::{Agent, AgentOpts, Behavior};
pub use world::{make_managed, opts_for, ManagedAgent, Snapshot, World};

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

    // ───────────────────────── the agent-sim bridge ─────────────────────────
    // One `Sim` per world. JS spawns agents (by kind-code + seedId), drives it with `step(dt)` once per frame
    // (the Rust clock sub-steps to fixed DT internally), and reads transforms back as typed-array VIEWS over
    // WASM memory — NEVER a JS↔WASM call per agent. Pointers are stable between spawns; re-fetch them after any
    // `spawn` (the buffers may grow/reallocate) or if `memory.buffer` detaches on growth.
    use crate::steering::Agent;
    use crate::world::{opts_for, Snapshot, World};

    #[wasm_bindgen]
    pub struct Sim {
        world: World,
        snap: Snapshot,
    }

    #[wasm_bindgen]
    impl Sim {
        #[wasm_bindgen(constructor)]
        pub fn new() -> Sim {
            Sim { world: World::new(), snap: Snapshot::default() }
        }

        /// Spawn an agent from a kind-code (0 rabbit·1 cat·2 kangaroo·3 person·4 lion·5 dinosaur) + a stable
        /// per-individual `seed_id` (its traits/speed key off this). Returns its index = its read-back slot.
        pub fn spawn(&mut self, x: f64, z: f64, kind_code: u8, radius: f64, seed_id: i32) -> usize {
            let kind = crate::eco::kind_from_code(kind_code);
            let agent = Agent::new(x, z, seed_id, &opts_for(kind, seed_id));
            self.world.spawn(agent, kind, radius, seed_id)
        }

        /// Spawn into a stable read-back slot recycled by the worker proxy's free-list.
        pub fn spawn_at(&mut self, slot: usize, x: f64, z: f64, kind_code: u8, radius: f64, seed_id: i32) -> usize {
            let kind = crate::eco::kind_from_code(kind_code);
            let agent = Agent::new(x, z, seed_id, &opts_for(kind, seed_id));
            self.world.spawn_at(slot, agent, kind, radius, seed_id)
        }

        pub fn set_player(&mut self, x: f64, z: f64) {
            self.world.set_player(x, z);
        }

        /// Mark a spawned agent (by index) as the player's pet → it follows you and won't flee you.
        pub fn set_companion(&mut self, i: usize) {
            self.world.set_companion(i);
        }

        /// Remove agent `i` (its world-object was deleted / world cleared) → it goes inert, no longer a ghost.
        pub fn despawn(&mut self, i: usize) {
            self.world.despawn(i);
        }

        /// Stamp a newborn (by index) with a maturation breed-cooldown so it can't breed until it grows up.
        pub fn set_breed_cooldown(&mut self, i: usize, cd: f64) {
            self.world.set_breed_cooldown(i, cd);
        }
        /// Apply a bred baby's inherited vigor gene (by index) — scales its speed (genetics/evolution).
        pub fn set_gene(&mut self, i: usize, gene: f64) {
            self.world.set_gene(i, gene);
        }
        /// The cooldown JS should stamp on a newborn.
        pub fn juvenile_cd(&self) -> f64 {
            self.world.juvenile_cd()
        }
        /// Newborns from the last step(): count of births (each is [kindCode, x, z, gene]).
        pub fn birth_count(&self) -> usize {
            self.world.births().len() / 4
        }
        /// Pointer to the flat births buffer [kindCode, x, z, gene, …] (length = birth_count()*4) for a zero-copy read.
        pub fn births_ptr(&self) -> *const f32 {
            self.world.births().as_ptr()
        }
        /// House-build requests from the last step(): count (each is [x, z]).
        pub fn build_count(&self) -> usize {
            self.world.builds().len() / 2
        }
        /// Pointer to the flat builds buffer [x, z, …] (length = build_count()*2) for a zero-copy read.
        pub fn builds_ptr(&self) -> *const f32 {
            self.world.builds().as_ptr()
        }
        /// Telemetry events from the last step(): count (each is [code, kind, x, z]).
        pub fn event_count(&self) -> usize {
            self.world.events().len() / 4
        }
        /// Pointer to the flat events buffer [code, kind, x, z, …] (length = event_count()*4) for a zero-copy read.
        pub fn events_ptr(&self) -> *const f32 {
            self.world.events().as_ptr()
        }

        pub fn set_night(&mut self, n: f64) {
            self.world.set_night(n);
        }

        pub fn set_pop_scale(&mut self, s: f64) {
            self.world.set_pop_scale(s);
        }

        /// Per-kind breeding vitality from the JS "Mother Nature" director (6 floats, by Kind index).
        pub fn set_vitality(&mut self, v: &[f64]) {
            self.world.set_vitality(v);
        }

        /// Replace the lake-fish lure points from a flat [x0,z0,x1,z1,…] buffer.
        pub fn set_fish(&mut self, xz: &[f64]) {
            self.world.set_fish(xz);
        }

        /// Replace the solid obstacles from a packed [x,z,r,hx,hz,cos,sin] per obstacle (7 f64s each); a CIRCLE
        /// is signalled by `hx = NaN`, else it's an oriented box. Agents are pushed out + slide along surfaces.
        pub fn set_obstacles(&mut self, flat: &[f64]) {
            self.world.set_obstacles(flat);
        }

        /// Advance by real elapsed seconds (the clock emits N fixed-DT ticks), then refresh the read-back.
        pub fn step(&mut self, real_dt: f64) {
            self.world.step(real_dt);
            self.snap.fill(&self.world);
        }

        pub fn count(&self) -> usize {
            self.snap.xs.len()
        }

        /// 0..1 — how imminent a player-hunting predator is (eased; drives the UI danger vignette).
        pub fn danger(&self) -> f64 {
            self.world.danger
        }

        // Pointers into WASM linear memory for zero-copy typed-array views (length = `count()`):
        //   new Float32Array(memory.buffer, sim.xs_ptr(), sim.count())   // likewise zs / headings / healths
        //   new Uint32Array (memory.buffer, sim.flags_ptr(), sim.count())// bit0 dead · bit1 asleep · bit2 moving
        pub fn xs_ptr(&self) -> *const f32 {
            self.snap.xs.as_ptr()
        }
        pub fn zs_ptr(&self) -> *const f32 {
            self.snap.zs.as_ptr()
        }
        pub fn headings_ptr(&self) -> *const f32 {
            self.snap.headings.as_ptr()
        }
        pub fn healths_ptr(&self) -> *const f32 {
            self.snap.healths.as_ptr()
        }
        pub fn flags_ptr(&self) -> *const u32 {
            self.snap.flags.as_ptr()
        }
        pub fn behaviors_ptr(&self) -> *const u8 {
            self.snap.behaviors.as_ptr()
        }
        pub fn progress_ptr(&self) -> *const f32 {
            self.snap.progress.as_ptr()
        }
    }
}
