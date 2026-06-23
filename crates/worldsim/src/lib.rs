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
mod engine;
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
pub use world::{cap_for, ff_targets, make_managed, opts_for, ManagedAgent, Snapshot, World};

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

    /// Carrying caps for the 6 kinds given live counts + world-area scale — the SAME `cap_for` the sim uses, so JS
    /// (load-trim / scatter) never re-derives the formula. Returns [rabbit, cat, kangaroo, person, lion, dino].
    #[wasm_bindgen]
    pub fn pop_caps(rabbit: u32, cat: u32, kangaroo: u32, person: u32, lion: u32, dino: u32, scale: f64) -> Vec<u32> {
        use crate::eco::Kind;
        let pop = [rabbit as usize, cat as usize, kangaroo as usize, person as usize, lion as usize, dino as usize];
        [Kind::Rabbit, Kind::Cat, Kind::Kangaroo, Kind::Person, Kind::Lion, Kind::Dinosaur]
            .iter()
            .map(|&k| crate::world::cap_for(k, &pop, scale) as u32)
            .collect()
    }

    /// Aggregate fast-forward: advance the 6 populations by `dt` seconds away toward carrying capacity (closed-form
    /// logistic). Returns target headcounts [rabbit, cat, kangaroo, person, lion, dino] — JS materialises the deltas.
    #[wasm_bindgen]
    pub fn ff_targets(rabbit: u32, cat: u32, kangaroo: u32, person: u32, lion: u32, dino: u32, scale: f64, dt: f64) -> Vec<u32> {
        let pop = [rabbit as usize, cat as usize, kangaroo as usize, person as usize, lion as usize, dino as usize];
        crate::world::ff_targets(&pop, scale, dt).to_vec()
    }

    /// Closed-form VIGOR drift for a dormant region over `dt` seconds away — evolves the offloaded population's mean
    /// gene under predation pressure (no ticking). Lets a dormant region EVOLVE via the clock, not stay frozen.
    #[wasm_bindgen]
    pub fn ff_gene(gene: f64, rabbit: u32, cat: u32, kangaroo: u32, person: u32, lion: u32, dino: u32, dt: f64) -> f64 {
        let pop = [rabbit as usize, cat as usize, kangaroo as usize, person as usize, lion as usize, dino as usize];
        crate::world::ff_gene(gene, &pop, dt)
    }

    /// Spawn-spread layout for a big creature batch ("100 humans"): BANDS of up to 10 laid on a golden-spiral
    /// around the anchor, members loosely clustered within each band, spread wide (~22·√count) so most land BEYOND
    /// the mesh-reveal radius → cheap LOD impostors, no mount-storm jank. Returns flat [x,z,…] snapped to 0.5 m.
    /// The deterministic op→placement math lives HERE in Rust, not in the JS engine.
    #[wasm_bindgen]
    pub fn band_spread(count: u32, ax: f64, az: f64, r: f64) -> Vec<f64> {
        crate::world::band_spread(count as usize, ax, az, r)
    }

    /// NATURAL PONDS near (px,pz) within `reach` — Rust owns the world's water (a deterministic, even, infinite
    /// pond field); the renderer calls this once per area to DRAW them. Flat [x, z, radius, …]. Cheap + stateless.
    #[wasm_bindgen]
    pub fn ponds_near(px: f64, pz: f64, reach: f64) -> Vec<f64> {
        crate::engine::ponds_near(px, pz, reach).into_iter().flat_map(|(x, z, r)| [x, z, r]).collect()
    }

    /// Per-kind MIGRATION weight, by Kind order [rabbit, cat, kangaroo, person, lion, dinosaur] — the sim's source
    /// of truth (world::migrate_weight), so the HUD reads it from here instead of hard-coding a duplicate copy.
    #[wasm_bindgen]
    pub fn migrate_weights() -> Vec<f64> {
        use crate::eco::Kind::*;
        [Rabbit, Cat, Kangaroo, Person, Lion, Dinosaur].map(crate::world::migrate_weight).to_vec()
    }

    /// The RENDER slice of the eco table — [rank, speed_lo, speed_hi] per kind, by Kind order. Rust owns the full
    /// canonical eco.rs; the renderer reads ONLY what it needs (gait speed range + rank) from here, no JS copy.
    #[wasm_bindgen]
    pub fn eco_render() -> Vec<f64> {
        use crate::eco::{eco, Kind::*};
        [Rabbit, Cat, Kangaroo, Person, Lion, Dinosaur]
            .iter()
            .flat_map(|&k| {
                let e = eco(k);
                [e.rank as f64, e.speed_lo, e.speed_hi]
            })
            .collect()
    }

    /// THE ENGINE (no JS engine): apply `ops_json` to `world_json` for a player at (px,pz,yaw). Returns a JSON
    /// string `{"world": <new world>, "conflicts": [...]}`. The world DOM round-trips unknown fields untouched.
    /// Faithful port of the old engine.ts applyOps — see crate::engine (parity-tested against the JS originals).
    #[wasm_bindgen]
    pub fn apply_ops(world_json: &str, ops_json: &str, px: f64, pz: f64, yaw: f64) -> String {
        let mut world = jzon::parse(world_json).unwrap_or_else(|_| jzon::JsonValue::new_object());
        let ops = jzon::parse(ops_json).unwrap_or_else(|_| jzon::JsonValue::new_array());
        let conflicts = crate::engine::apply_ops(&mut world, &ops, px, pz, yaw);
        let mut out = jzon::JsonValue::new_object();
        out["world"] = world;
        let mut c = jzon::JsonValue::new_array();
        for cf in conflicts {
            let _ = c.push(cf);
        }
        out["conflicts"] = c;
        out.dump()
    }

    // ───────────────────────── the agent-sim bridge ─────────────────────────
    // One `Sim` per world. JS spawns agents (by kind-code + seedId), drives it with `step(dt)` once per frame
    // (the Rust clock sub-steps to fixed DT internally), and reads transforms back as typed-array VIEWS over
    // WASM memory — NEVER a JS↔WASM call per agent. Pointers are stable between spawns; re-fetch them after any
    // `spawn` (the buffers may grow/reallocate) or if `memory.buffer` detaches on growth.
    use crate::steering::Agent;
    use crate::world::{opts_for, BehaviorMode, Snapshot, World};

    #[wasm_bindgen]
    pub struct Sim {
        world: World,
        snap: Snapshot,
    }

    #[wasm_bindgen]
    impl Sim {
        #[wasm_bindgen(constructor)]
        pub fn new() -> Sim {
            let mut world = World::new();
            world.set_behavior_mode(BehaviorMode::Emergent); // the GAME runs the emergent brain by default
            world.set_player_immune(true); // the player is not prey (user: "give me immunity, no animals hunt me")
            world.set_natural_water(true); // Rust owns the world's water: an even procedural pond field everywhere
            Sim { world, snap: Snapshot::default() }
        }

        /// Toggle player immunity (1 = no predator hunts/menaces you, danger stays 0 · 0 = you're fair game).
        pub fn set_player_immune(&mut self, immune: u32) {
            self.world.set_player_immune(immune != 0);
        }

        /// Switch the decision brain (0 = Manual, the proven hand-coded sim · 1 = Emergent, needs+utility). The
        /// chosen mode persists on the world; JS surfaces a dev toggle + serialises it in the world blob.
        pub fn set_behavior_mode(&mut self, code: u8) {
            self.world.set_behavior_mode(BehaviorMode::from_code(code));
        }

        /// Mean age (fraction of lifespan, 0..1) per Kind [rabbit,cat,kangaroo,person,lion,dino]; -1 = none alive.
        pub fn age_means(&self) -> Vec<f32> {
            self.world.age_means()
        }

        /// The brain currently running (0 = Manual · 1 = Emergent) — for the HUD readout / persistence.
        pub fn behavior_mode(&self) -> u8 {
            match self.world.behavior_mode() {
                BehaviorMode::Emergent => 1,
                BehaviorMode::Manual => 0,
            }
        }

        /// Spawn an agent from a kind-code (0 rabbit·1 cat·2 kangaroo·3 person·4 lion·5 dinosaur) + a stable
        /// per-individual `seed_id` (its traits/speed key off this). Returns its index = its read-back slot.
        pub fn spawn(&mut self, x: f64, z: f64, kind_code: u8, radius: f64, seed_id: i32) -> usize {
            let kind = crate::eco::kind_from_code(kind_code);
            let agent = Agent::new(x, z, seed_id, &opts_for(kind, seed_id));
            let idx = self.world.spawn(agent, kind, radius, seed_id);
            self.world.randomize_start_age(idx, seed_id); // founder age structure (newborns then get set_breed_cooldown → age 0)
            idx
        }

        /// Spawn into a stable read-back slot recycled by the worker proxy's free-list.
        pub fn spawn_at(&mut self, slot: usize, x: f64, z: f64, kind_code: u8, radius: f64, seed_id: i32) -> usize {
            let kind = crate::eco::kind_from_code(kind_code);
            let agent = Agent::new(x, z, seed_id, &opts_for(kind, seed_id));
            let idx = self.world.spawn_at(slot, agent, kind, radius, seed_id);
            self.world.randomize_start_age(idx, seed_id); // founder age structure (newborns then get set_breed_cooldown → age 0)
            idx
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
        /// Newborns from the last step(): count (each is [kc, x, z, gene, motherFam, fatherFam, g0..g4] — 11 floats).
        pub fn birth_count(&self) -> usize {
            self.world.births().len() / 11
        }
        /// Pointer to the flat births buffer [kc, x, z, gene, motherFam, fatherFam, g0..g4, …] (len = birth_count()*11).
        pub fn births_ptr(&self) -> *const f32 {
            self.world.births().as_ptr()
        }
        /// Stamp a newborn (by index) with its PARENT lineage ids (mother's fam, father's fam) from the births buffer,
        /// so the kinship check refuses a future parent/child/sibling pairing (incest avoidance, all kinds).
        pub fn set_lineage(&mut self, i: usize, pfam_a: u32, pfam_b: u32) {
            self.world.set_lineage(i, pfam_a, pfam_b);
        }
        /// Apply a bred baby's inherited behaviour GENOME (5 utility weights from the births buffer) → emergent
        /// strategies evolve across generations.
        pub fn set_genome(&mut self, i: usize, food: f64, safety: f64, social: f64, rest: f64, industry: f64) {
            self.world.set_genome(i, food, safety, social, rest, industry);
        }
        /// House-build requests from the last step(): count (each is [x, z]).
        pub fn build_count(&self) -> usize {
            self.world.builds().len() / 2
        }
        /// Pointer to the flat builds buffer [x, z, …] (length = build_count()*2) for a zero-copy read.
        pub fn builds_ptr(&self) -> *const f32 {
            self.world.builds().as_ptr()
        }
        /// Well-dig requests from the last step(): count (each is [x, z]).
        pub fn well_count(&self) -> usize {
            self.world.wells().len() / 2
        }
        /// Pointer to the flat wells buffer [x, z, …] (length = well_count()*2) for a zero-copy read.
        pub fn wells_ptr(&self) -> *const f32 {
            self.world.wells().as_ptr()
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

        /// DROUGHT multiplier on thirst (1 = normal). The director/LLM sets this for a drought event; it stacks on
        /// the always-on wet↔dry season cycle. Clamped 0.5‥3.0 internally.
        pub fn set_aridity(&mut self, a: f64) {
            self.world.set_aridity(a);
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

        /// Replace the DRINKABLE water sources (thirst) from a flat [x0,z0,r0,x1,z1,r1,…] buffer (pond centre+radius).
        pub fn set_water(&mut self, xzr: &[f64]) {
            self.world.set_water(xzr);
        }

        /// Replace the REFUGE points (house centres) a threatened woman/child flees toward, flat [x0,z0,x1,z1,…].
        pub fn set_refuges(&mut self, xz: &[f64]) {
            self.world.set_refuges(xz);
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
