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

mod catchup;
mod clock;
mod eco;
mod engine;
mod engine_bin;
mod rng;
mod simrng;
mod spatialhash;
mod world;
mod worldgen;
mod steering;
mod structstore;

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

    /// World-AREA carrying-capacity multiplier from the built count — the SAME formula the sim + fast-forward use, so
    /// the scale JS feeds into `cap_for`/`ff_targets` can never drift. JS counts the buildings; Rust owns the math.
    #[wasm_bindgen]
    pub fn world_area_scale(builds: u32) -> f64 {
        crate::world::world_area_scale(builds as usize)
    }

    /// Female fertile WINDOW (seconds) per kind — maturity → menopause/old-age. The SAME numbers the sim breeds by,
    /// so the HUD's per-species TFR estimate (births ÷ fertile females × this window) never drifts from the sim.
    /// Returns [rabbit, cat, kangaroo, person, lion, dino].
    #[wasm_bindgen]
    pub fn fertile_windows() -> Vec<f64> {
        use crate::eco::Kind;
        [Kind::Rabbit, Kind::Cat, Kind::Kangaroo, Kind::Person, Kind::Lion, Kind::Dinosaur]
            .iter()
            .map(|&k| crate::world::fertile_window(k))
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

    /// AMBIENT TREES near (px,pz) — Rust owns the forest field. Flat [x, z, scale, scaleY, rot, colorHash] × n.
    /// The renderer + collision read this ONCE per rebuild (cheap); JS culls trees on its own paths/lakes.
    #[wasm_bindgen]
    pub fn trees_near(px: f64, pz: f64, reach: f64) -> Vec<f64> {
        crate::engine::trees_near(px, pz, reach)
    }

    /// AMBIENT BUSHES near (px,pz). Flat [x, z, scale, rot, colorHash] × n.
    #[wasm_bindgen]
    pub fn bushes_near(px: f64, pz: f64, reach: f64) -> Vec<f64> {
        crate::engine::bushes_near(px, pz, reach)
    }

    /// The VIGOR gene bounds [GENE_MIN, GENE_MAX] — the sim's source of truth, so the JS clamps that defensively
    /// keep a read-back/aggregate gene in range read it from here instead of hard-coding 0.6/1.6 in six places.
    #[wasm_bindgen]
    pub fn gene_bounds() -> Vec<f64> {
        vec![crate::world::GENE_MIN, crate::world::GENE_MAX]
    }

    /// Sim ticks per second (1 / DT) — the fixed-timestep rate, so JS region-streaming derives dormant-span seconds
    /// from the sim's clock instead of a duplicated `TICK_HZ = 30`.
    #[wasm_bindgen]
    pub fn tick_hz() -> f64 {
        1.0 / crate::clock::DT
    }

    /// Per-kind MIGRATION weight, by Kind order [rabbit, cat, kangaroo, person, lion, dinosaur] — the sim's source
    /// of truth (world::migrate_weight), so the HUD reads it from here instead of hard-coding a duplicate copy.
    #[wasm_bindgen]
    pub fn migrate_weights() -> Vec<f64> {
        use crate::eco::Kind::*;
        [Rabbit, Cat, Kangaroo, Person, Lion, Dinosaur].map(crate::world::migrate_weight).to_vec()
    }

    /// AMBIENT terrain height at (x,z) with no contained features — the deterministic wilderness relief. The render
    /// (terrain.ts heightAt) keeps a native copy (it runs per-frame to ground objects + before the wasm loads), so
    /// this exists to PARITY-TEST that copy against Rust (src/lib/terrain.test.ts). Feature patches blend on top in
    /// both copies; the ambient field is the shared core most likely to drift on a tweak.
    #[wasm_bindgen]
    pub fn terrain_height(x: f64, z: f64) -> f64 {
        crate::engine::height_at(x, z, &[])
    }

    /// Pond per-id SEED (matches the render's waterSeed) — exposed so a parity test pins the JS copy to Rust.
    #[wasm_bindgen]
    pub fn water_seed(id: &str) -> f64 {
        crate::engine::water_seed(id)
    }

    /// Kind FOOTPRINT [radius, height] — engine.rs `kind_rh` is the collision source of truth. The JS `KINDS` table
    /// keeps its own r/h copy (it also carries render geometry), so a parity test (src/lib/kinds.test.ts) pins the JS
    /// numbers to these — a drift would mean placement/collision disagreeing with what's drawn. Unknown → fallback.
    #[wasm_bindgen]
    pub fn kind_rh(kind: &str) -> Vec<f64> {
        let (r, h) = crate::engine::kind_rh(kind);
        vec![r, h]
    }

    /// Pond SHORELINE radius factor at `ang` for a `seed` — the organic-blob edge. The render keeps a native copy
    /// (player wade check runs per frame, pre-wasm-load), so this exists to PARITY-TEST that copy against Rust.
    #[wasm_bindgen]
    pub fn water_edge_factor(seed: f64, ang: f64) -> f64 {
        crate::engine::water_edge_factor(seed, ang)
    }

    /// Per-kind GESTATION seconds, by Kind order [rabbit, cat, kangaroo, person, lion, dinosaur] — the sim's source
    /// of truth (world::gestation), so the renderer paces the pregnancy belly-grow to the REAL delivery time instead
    /// of a duplicated guess (the belly hit full term well before/after delivery when JS hard-coded its own number).
    #[wasm_bindgen]
    pub fn gestation_secs() -> Vec<f64> {
        use crate::eco::Kind::*;
        [Rabbit, Cat, Kangaroo, Person, Lion, Dinosaur].map(crate::world::gestation).to_vec()
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

    /// THE BINARY ENGINE (the jzon-drop path, docs/world-data-architecture.md). Same op→world layer as `apply_ops`
    /// but NO JSON: the world + ops cross as parallel string vecs + a flat f64 SoA (see engine_bin decode fns), the
    /// new world + conflicts ride back in `ApplyResult`. Parity-pinned to `apply_ops` (engine_bin parity test + the
    /// JS vitest). `obj_num` stride 9, `zone_num` 4, `path_num` 7, `terrain_num` 5; `op_num` 19, `op_strs` 11.
    #[wasm_bindgen]
    #[allow(clippy::too_many_arguments)]
    pub fn apply_ops_bin(
        obj_ids: Vec<String>,
        obj_kinds: Vec<String>,
        obj_colors: Vec<String>,
        obj_num: &[f64],
        zone_ids: Vec<String>,
        zone_materials: Vec<String>,
        zone_shapes: Vec<String>,
        zone_num: &[f64],
        path_ids: Vec<String>,
        path_materials: Vec<String>,
        path_num: &[f64],
        terrain_num: &[f64],
        ground: String,
        sky: String,
        op_num: &[f64],
        op_strs: Vec<String>,
        px: f64,
        pz: f64,
        yaw: f64,
    ) -> ApplyResult {
        use crate::engine_bin as eb;
        let mut world = eb::EWorld {
            objects: eb::decode_objs(&obj_ids, &obj_kinds, &obj_colors, obj_num),
            zones: eb::decode_zones(&zone_ids, &zone_materials, &zone_shapes, zone_num),
            paths: eb::decode_paths(&path_ids, &path_materials, path_num),
            terrain: eb::decode_terrain(terrain_num),
            ground,
            sky,
        };
        let ops = eb::decode_ops(op_num, &op_strs);
        let conflicts = eb::apply_ops_bin(&mut world, &ops, px, pz, yaw);
        let (obj_ids, obj_kinds, obj_colors, obj_num) = eb::encode_objs(&world.objects);
        let (zone_ids, zone_materials, zone_shapes, zone_num) = eb::encode_zones(&world.zones);
        let (path_ids, path_materials, path_num) = eb::encode_paths(&world.paths);
        ApplyResult {
            obj_ids,
            obj_kinds,
            obj_colors,
            obj_num,
            zone_ids,
            zone_materials,
            zone_shapes,
            zone_num,
            path_ids,
            path_materials,
            path_num,
            terrain_num: eb::encode_terrain(&world.terrain),
            ground: world.ground,
            sky: world.sky,
            conflict_labels: conflicts.iter().map(|c| c.label.clone()).collect(),
            // each conflict's blocker ids comma-joined (ids are `o<base36>` — no commas); JS splits, "" → no blockers
            conflict_blockers: conflicts.iter().map(|c| c.blockers.join(",")).collect(),
        }
    }

    /// The `apply_ops_bin` result: the new world as the SAME parallel arrays JS packs in, plus conflicts. Read once via
    /// the getters (each clones — this is a cold per-edit call, not a hot path).
    #[wasm_bindgen]
    pub struct ApplyResult {
        obj_ids: Vec<String>,
        obj_kinds: Vec<String>,
        obj_colors: Vec<String>,
        obj_num: Vec<f64>,
        zone_ids: Vec<String>,
        zone_materials: Vec<String>,
        zone_shapes: Vec<String>,
        zone_num: Vec<f64>,
        path_ids: Vec<String>,
        path_materials: Vec<String>,
        path_num: Vec<f64>,
        terrain_num: Vec<f64>,
        ground: String,
        sky: String,
        conflict_labels: Vec<String>,
        conflict_blockers: Vec<String>,
    }

    #[wasm_bindgen]
    impl ApplyResult {
        #[wasm_bindgen(getter)]
        pub fn obj_ids(&self) -> Vec<String> {
            self.obj_ids.clone()
        }
        #[wasm_bindgen(getter)]
        pub fn obj_kinds(&self) -> Vec<String> {
            self.obj_kinds.clone()
        }
        #[wasm_bindgen(getter)]
        pub fn obj_colors(&self) -> Vec<String> {
            self.obj_colors.clone()
        }
        #[wasm_bindgen(getter)]
        pub fn obj_num(&self) -> Vec<f64> {
            self.obj_num.clone()
        }
        #[wasm_bindgen(getter)]
        pub fn zone_ids(&self) -> Vec<String> {
            self.zone_ids.clone()
        }
        #[wasm_bindgen(getter)]
        pub fn zone_materials(&self) -> Vec<String> {
            self.zone_materials.clone()
        }
        #[wasm_bindgen(getter)]
        pub fn zone_shapes(&self) -> Vec<String> {
            self.zone_shapes.clone()
        }
        #[wasm_bindgen(getter)]
        pub fn zone_num(&self) -> Vec<f64> {
            self.zone_num.clone()
        }
        #[wasm_bindgen(getter)]
        pub fn path_ids(&self) -> Vec<String> {
            self.path_ids.clone()
        }
        #[wasm_bindgen(getter)]
        pub fn path_materials(&self) -> Vec<String> {
            self.path_materials.clone()
        }
        #[wasm_bindgen(getter)]
        pub fn path_num(&self) -> Vec<f64> {
            self.path_num.clone()
        }
        #[wasm_bindgen(getter)]
        pub fn terrain_num(&self) -> Vec<f64> {
            self.terrain_num.clone()
        }
        #[wasm_bindgen(getter)]
        pub fn ground(&self) -> String {
            self.ground.clone()
        }
        #[wasm_bindgen(getter)]
        pub fn sky(&self) -> String {
            self.sky.clone()
        }
        #[wasm_bindgen(getter)]
        pub fn conflict_labels(&self) -> Vec<String> {
            self.conflict_labels.clone()
        }
        #[wasm_bindgen(getter)]
        pub fn conflict_blockers(&self) -> Vec<String> {
            self.conflict_blockers.clone()
        }
    }

    /// STATELESS settlement wall refit (the jzon-drop for the away-growth / fast-forward path). Builds a THROWAWAY
    /// `StructureStore` from `soa` (`[kind,x,z,rot,sx,sy,sz,color,keep]×n`, same layout as `WorldGen.seed`), fits every
    /// town's perimeter against `zones` (water `[px,pz,size,seed]×n`), and returns the GEN op stream — WITHOUT touching
    /// the persistent live `WorldGen` store, so the renderer's incremental fence state is never clobbered. A REMOVE
    /// references its target by slot (index in `soa`), which JS maps back to the object id it packed at that index.
    #[wasm_bindgen]
    pub fn settlement_ops_bin(soa: &[f64], zones: &[f64]) -> Vec<f64> {
        let mut store = crate::structstore::StructureStore::new();
        for c in soa.chunks_exact(9) {
            store.add(crate::structstore::Structure { kind: c[0] as u8, x: c[1], z: c[2], rot: c[3], sx: c[4], sy: c[5], sz: c[6], color: c[7] as u32, keep: c[8] != 0.0, region: 0 });
        }
        crate::worldgen::settlement_ops_store(&mut store, zones, &[]) // empty `changed` → fit every town
    }

    // ───────────────────────── the STRUCTURE store bridge (binary worldgen) ─────────────────────────
    // `WorldGen` owns a persistent StructureStore (binary SoA + spatial grid) so the worldgen ops run against an
    // in-wasm structure arena instead of receiving `JSON.stringify(world)` every event — docs/world-data-architecture.md.
    // JS seeds it from the (bounded ≤STRUCT_BUDGET) live structures whenever they change, then calls the ops with small
    // binary inputs and applies the returned Float64Array op stream `[op,kind,x,z,rot,sx,sy,sz,color]×n` (op 0=add,
    // 1=remove-by-slot; slot indexes the SoA order JS seeded, so JS maps it back to the object id). No JSON either way.
    #[wasm_bindgen]
    pub struct WorldGen {
        store: crate::structstore::StructureStore,
    }

    #[wasm_bindgen]
    impl WorldGen {
        #[wasm_bindgen(constructor)]
        pub fn new() -> WorldGen {
            WorldGen { store: crate::structstore::StructureStore::new() }
        }

        /// Replace the store from a flat SoA `[kind, x, z, rot, sx, sy, sz, color, keep]×n`. JS packs world.objects'
        /// structures (in array order) once at load + whenever the structure set changes; the slot of each entry = its
        /// index here, so a returned REMOVE slot maps back to the object id JS packed at that index.
        pub fn seed(&mut self, soa: &[f64]) {
            self.store.clear();
            for c in soa.chunks_exact(9) {
                self.store.add(crate::structstore::Structure {
                    kind: c[0] as u8,
                    x: c[1],
                    z: c[2],
                    rot: c[3],
                    sx: c[4],
                    sy: c[5],
                    sz: c[6],
                    color: c[7] as u32,
                    keep: c[8] != 0.0,
                    region: 0,
                });
            }
        }

        pub fn well(&mut self, reqs: &[f64], zones: &[f64]) -> Vec<f64> {
            crate::worldgen::well_ops_store(&mut self.store, reqs, zones)
        }
        pub fn build(&mut self, reqs: &[f64], zones: &[f64]) -> Vec<f64> {
            crate::worldgen::build_ops_store(&mut self.store, reqs, zones)
        }
        /// DORMANT settlement growth (self-sustaining world): grow a FAR cluster's homes via a throwaway store (does
        /// NOT touch the live `self.store`). `houses` = the cluster's `[x,z]×n`; returns up to `want` new build ops.
        pub fn grow_dormant(&self, houses: &[f64], want: u32, zones: &[f64], seed: f64) -> Vec<f64> {
            crate::worldgen::grow_dormant_houses(houses, want as usize, zones, seed)
        }
        pub fn grave(&mut self, dx: f64, dz: f64, zones: &[f64]) -> Vec<f64> {
            crate::worldgen::grave_site_store(&self.store, dx, dz, zones)
        }
        pub fn veg(&mut self, seed: f64, zones: &[f64]) -> Vec<f64> {
            crate::worldgen::vegetation_ops_store(&mut self.store, seed, zones)
        }
        /// `changed` = positions `[x,z]×n` of structures changed this frame → only those towns' walls re-fit (others
        /// stay put). Empty = fit every town (the one-time load reconcile).
        pub fn settlement(&mut self, zones: &[f64], changed: &[f64]) -> Vec<f64> {
            crate::worldgen::settlement_ops_store(&mut self.store, zones, changed)
        }
        /// LAKE generator (binary) — `zones` = water zones `[px,pz,size,seed]×n`. Returns the GEN op stream (stride 10);
        /// a REMOVE references its target zone by slot (its index in `zones`), which JS maps back to the zone id.
        pub fn lake(&self, zones: &[f64], px: f64, pz: f64, yaw: f64) -> Vec<f64> {
            crate::worldgen::lake_ops_bin(zones, px, pz, yaw)
        }
        /// FOREST generator (binary) — reads trees from the (seeded) store, water from `zones`. Returns the GEN op stream.
        pub fn forest(&self, zones: &[f64], px: f64, pz: f64, yaw: f64) -> Vec<f64> {
            crate::worldgen::forest_ops_bin(&self.store, zones, px, pz, yaw)
        }
        /// CITY generator (binary) — reads buildings from the (seeded) store, water from `zones`, and the removable old
        /// spokes/plaza from `removables` (`[tag,x,z]×n`; a returned REMOVE slot maps back to a path/plaza id JS-side).
        pub fn city(&self, zones: &[f64], removables: &[f64], px: f64, pz: f64, yaw: f64) -> Vec<f64> {
            crate::worldgen::city_ops_bin(&self.store, zones, removables, px, pz, yaw)
        }
        /// IMMIGRATION decision (binary) — `counts` = `[n,geneSum]×5` (FLOORS order rabbit,kangaroo,person,cat,lion).
        /// Returns a flat `[floorIdx,x,z,gene]×n` add-creature stream (no store needed; JS maps floorIdx → kind).
        pub fn immigration(&self, counts: &[f64], px: f64, pz: f64, global_avg: f64, seed: f64) -> Vec<f64> {
            crate::worldgen::immigration_ops_bin(counts, px, pz, global_avg, seed)
        }
        /// SETTLEMENT PLAN (binary) — a deterministic town plan packed as `[radius, numPaths, numObjects, <paths×4>,
        /// <objects×7>]` (paths then objects; JS rebuilds ids + Path/WorldObject shapes). No store needed.
        pub fn town_plan(&self, cx: f64, cz: f64, size: &str, seed: u32) -> Vec<f64> {
            crate::worldgen::settlement_plan_bin(cx, cz, size, seed)
        }
        /// DEMO GALLERY (binary) — Rust owns the whole multi-town layout (spacing/grid/sizes), packed as `[numSites,
        /// numPaths, numObjects, <sites: cx,cz,sizeCode>, <paths×4>, <objects×7>]`. JS just materialises it.
        pub fn demo_gallery(&self) -> Vec<f64> {
            crate::worldgen::demo_gallery_bin()
        }
        /// Binary snapshot of the live structures → IndexedDB stores the bytes (no JSON). Restored via `deserialize`.
        pub fn serialize(&self) -> Vec<u8> {
            self.store.serialize()
        }
        pub fn deserialize(&mut self, buf: &[u8]) {
            self.store.deserialize(buf);
        }
        pub fn len(&self) -> usize {
            self.store.len()
        }
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
            let mut world = World::new();
            world.set_player_immune(true); // the player is not prey (user: "give me immunity, no animals hunt me")
            world.set_natural_water(true); // Rust owns the world's water: an even procedural pond field everywhere
            Sim { world, snap: Snapshot::default() }
        }

        /// Toggle player immunity (1 = no predator hunts/menaces you, danger stays 0 · 0 = you're fair game).
        pub fn set_player_immune(&mut self, immune: u32) {
            self.world.set_player_immune(immune != 0);
        }

        /// Mean age (fraction of lifespan, 0..1) per Kind [rabbit,cat,kangaroo,person,lion,dino]; -1 = none alive.
        pub fn age_means(&self) -> Vec<f32> {
            self.world.age_means()
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
        /// Restore a saved agent's exact age (life fraction 0..1) — reload keeps adults adult, not seeded-young.
        pub fn set_age(&mut self, i: usize, frac: f64) {
            self.world.set_age(i, frac);
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
        pub fn ages_ptr(&self) -> *const f32 {
            self.snap.ages.as_ptr()
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
