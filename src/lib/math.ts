// MAIN-THREAD WASM utilities — the Rust core's PURE, stateless math, made callable on the main thread so the
// "all real math lives in Rust" north star holds even for the LOAD-time / BUILD-time computations that don't run
// in the sim worker. This is a SECOND, tiny wasm instance (no Sim state, no per-tick loop) — the same .wasm bytes
// the worker already fetched (browser-cached), just a second WebAssembly instance for stateless calls.
//
// ONE stateful object — `math` — owns the wasm handle + the load lifecycle, with a SINGLE `#call` guard so each
// method is a one-liner (no `if (glue)` repeated per function). `await math.init()` once at startup, then call
// `math.pondsNear(...)`, `math.ffTargets(...)`, etc. Methods return the result, or a permissive sentinel (usually
// null) if the wasm somehow isn't loaded yet (never a duplicated JS formula). Names say WHAT, not the technology.


/** The stateful StructureStore bridge (binary worldgen ops) — crates/worldsim/src/lib.rs `WorldGen`. Ops return a
 *  flat Float64Array op stream [op(0=add,1=remove-slot), kind, x, z, rot, sx, sy, sz, color]×n; no JSON either way. */
interface WorldGenWasm {
	seed: (soa: Float64Array) => void;
	well: (reqs: Float64Array, zones: Float64Array) => Float64Array;
	build: (reqs: Float64Array, zones: Float64Array) => Float64Array;
	grow_dormant: (houses: Float64Array, want: number, zones: Float64Array, seed: number) => Float64Array;
	grave: (dx: number, dz: number, zones: Float64Array) => Float64Array;
	veg: (seed: number, zones: Float64Array) => Float64Array;
	settlement: (zones: Float64Array, changed: Float64Array) => Float64Array;
	lake: (zones: Float64Array, px: number, pz: number, yaw: number) => Float64Array;
	forest: (zones: Float64Array, px: number, pz: number, yaw: number) => Float64Array;
	city: (zones: Float64Array, removables: Float64Array, px: number, pz: number, yaw: number) => Float64Array;
	immigration: (counts: Float64Array, px: number, pz: number, globalAvg: number, seed: number) => Float64Array;
	town_plan: (cx: number, cz: number, size: string, seed: number) => Float64Array;
	demo_gallery: () => Float64Array;
	serialize: () => Uint8Array;
	deserialize: (buf: Uint8Array) => void;
	len: () => number;
	free: () => void;
}

/** The `apply_ops_bin` result object (wasm-bindgen getters) — the new world as the SAME parallel arrays we pack in,
 *  plus conflicts. Each getter clones; we read each once into a plain object then `free()` (see `applyOpsBin`). */
interface ApplyResultWasm {
	readonly obj_ids: string[];
	readonly obj_kinds: string[];
	readonly obj_colors: string[];
	readonly obj_num: Float64Array;
	readonly zone_ids: string[];
	readonly zone_materials: string[];
	readonly zone_shapes: string[];
	readonly zone_num: Float64Array;
	readonly path_ids: string[];
	readonly path_materials: string[];
	readonly path_num: Float64Array;
	readonly terrain_num: Float64Array;
	readonly ground: string;
	readonly sky: string;
	readonly conflict_labels: string[];
	readonly conflict_blockers: string[];
	free: () => void;
}

/** What `applyOpsBin` packs IN and reads OUT (engine.ts owns the World↔arrays (un)packing). The `*Strs`/`*Num` split
 *  mirrors the Rust decode/encode: parallel string vecs + a flat f64 SoA, no JSON. */
export interface PackedApply {
	objIds: string[];
	objKinds: string[];
	objColors: string[];
	objNum: Float64Array;
	zoneIds: string[];
	zoneMaterials: string[];
	zoneShapes: string[];
	zoneNum: Float64Array;
	pathIds: string[];
	pathMaterials: string[];
	pathNum: Float64Array;
	terrainNum: Float64Array;
	ground: string;
	sky: string;
	opNum: Float64Array;
	opStrs: string[];
	px: number;
	pz: number;
	yaw: number;
}
export interface RawApplyResult {
	objIds: string[];
	objKinds: string[];
	objColors: string[];
	objNum: Float64Array;
	zoneIds: string[];
	zoneMaterials: string[];
	zoneShapes: string[];
	zoneNum: Float64Array;
	pathIds: string[];
	pathMaterials: string[];
	pathNum: Float64Array;
	terrainNum: Float64Array;
	ground: string;
	sky: string;
	conflictLabels: string[];
	conflictBlockers: string[];
}

interface MathGlue {
	default: (input?: unknown) => Promise<unknown>;
	WorldGen: new () => WorldGenWasm;
	pop_caps: (rabbit: number, cat: number, kangaroo: number, person: number, lion: number, dino: number, scale: number) => Uint32Array;
	ff_targets: (rabbit: number, cat: number, kangaroo: number, person: number, lion: number, dino: number, scale: number, dt: number) => Uint32Array;
	ff_gene: (gene: number, rabbit: number, cat: number, kangaroo: number, person: number, lion: number, dino: number, dt: number) => number;
	band_spread: (count: number, ax: number, az: number, r: number) => Float64Array;
	ponds_near: (px: number, pz: number, reach: number) => Float64Array;
	trees_near: (px: number, pz: number, reach: number) => Float64Array;
	bushes_near: (px: number, pz: number, reach: number) => Float64Array;
	migrate_weights: () => Float64Array;
	fertile_windows: () => Float64Array;
	world_area_scale: (builds: number) => number;
	gestation_secs: () => Float64Array;
	kind_rh: (kind: string) => Float64Array;
	terrain_height: (x: number, z: number) => number;
	rng_hash: (position: number, seed: number) => number;
	rng_hash_keys: (seed: number, keys: Int32Array) => number;
	rng_rand: (seed: number, keys: Int32Array) => number;
	rng_seed_from: (s: string) => number;
	water_seed: (id: string) => number;
	water_edge_factor: (seed: number, ang: number) => number;
	eco_render: () => Float64Array;
	gene_bounds: () => Float64Array;
	tick_hz: () => number;
	apply_ops_bin: (
		obj_ids: string[],
		obj_kinds: string[],
		obj_colors: string[],
		obj_num: Float64Array,
		zone_ids: string[],
		zone_materials: string[],
		zone_shapes: string[],
		zone_num: Float64Array,
		path_ids: string[],
		path_materials: string[],
		path_num: Float64Array,
		terrain_num: Float64Array,
		ground: string,
		sky: string,
		op_num: Float64Array,
		op_strs: string[],
		px: number,
		pz: number,
		yaw: number,
	) => ApplyResultWasm;
	settlement_ops_bin: (soa: Float64Array, zones: Float64Array) => Float64Array;
}

class WorldMath {
	#glue: MathGlue | null = null;
	#worldgen: WorldGenWasm | null = null; // the stateful binary structure store (instantiated on load)
	#loading: Promise<void> | null = null;

	// cached sim constants — read ONCE from the source of truth (Rust) on first use, then memoised. The literals
	// are ONLY the pre-wasm-load fallback (they match the Rust defaults). All this state lives on the instance.
	#geneLo = 0.6;
	#geneHi = 1.6;
	#geneSynced = false;
	#tickHzCache = 0;
	#personGestationCache = 0;

	/** True once the wasm math instance is callable. */
	get ready(): boolean {
		return this.#glue !== null;
	}

	/** Load the main-thread wasm math instance (idempotent). Resolves once the pure functions are callable. */
	init(): Promise<void> {
		if (this.#glue) return Promise.resolve();
		if (!this.#loading) {
			this.#loading = (async () => {
				if (typeof location !== 'undefined' && typeof location.origin === 'string') {
					// BROWSER: same glue the worker loads; @vite-ignore so Vite doesn't bundle the static wasm pkg
					const m = (await import(/* @vite-ignore */ `${location.origin}/worldsim/worldsim.js`)) as unknown as MathGlue;
					await m.default();
					this.#glue = m;
					this.#worldgen = new m.WorldGen(); // the persistent binary structure store (docs/world-data-architecture.md)
				} else {
					// NODE (vitest — the engine runs in Rust now, so even node tests use the wasm): load the bytes from
					// disk; the web-target `default(init)` accepts a BufferSource.
					const fs = await import('node:fs');
					const jsUrl = new URL('../../static/worldsim/worldsim.js', import.meta.url);
					const wasmUrl = new URL('../../static/worldsim/worldsim_bg.wasm', import.meta.url);
					const m = (await import(/* @vite-ignore */ jsUrl.href)) as unknown as MathGlue;
					await m.default({ module_or_path: fs.readFileSync(wasmUrl) } as unknown as undefined);
					this.#glue = m;
					this.#worldgen = new m.WorldGen();
				}
			})().catch((e) => {
				console.error('[rustMath] failed to load wasm math', e);
			});
		}
		return this.#loading;
	}

	/** THE single guard — run `fn` against the loaded wasm, or return `fallback` (default null) if it isn't ready. */
	#call<T>(fn: (g: MathGlue) => T, fallback: T | null = null): T | null {
		return this.#glue ? fn(this.#glue) : fallback;
	}

	/** Carrying caps [rabbit, cat, kangaroo, person, lion, dinosaur] from the Rust `cap_for` — single source of truth. */
	popCaps(rabbit: number, cat: number, kangaroo: number, person: number, lion: number, dino: number, scale: number): Uint32Array | null {
		return this.#call((g) => g.pop_caps(rabbit, cat, kangaroo, person, lion, dino, scale));
	}

	/** Aggregate fast-forward target headcounts after `dt` seconds away — closed-form logistic relaxation, from Rust. */
	ffTargets(rabbit: number, cat: number, kangaroo: number, person: number, lion: number, dino: number, scale: number, dt: number): Uint32Array | null {
		return this.#call((g) => g.ff_targets(rabbit, cat, kangaroo, person, lion, dino, scale, dt));
	}

	/** Spawn-spread band layout [x,z,…] for a big creature batch — the golden-spiral placement math, from Rust. */
	bandSpread(count: number, ax: number, az: number, r: number): Float64Array | null {
		return this.#call((g) => g.band_spread(count, ax, az, r));
	}

	/** NATURAL PONDS near (px,pz) within `reach` — Rust owns the world's water. Flat [x, z, radius, …]. */
	pondsNear(px: number, pz: number, reach: number): Float64Array | null {
		return this.#call((g) => g.ponds_near(px, pz, reach));
	}

	/** Ambient TREES near (px,pz) — Rust owns the forest field. Flat [x,z,scale,scaleY,rot,colorHash]×n. */
	treesNear(px: number, pz: number, reach: number): Float64Array | null {
		return this.#call((g) => g.trees_near(px, pz, reach));
	}

	/** Ambient BUSHES near (px,pz). Flat [x,z,scale,rot,colorHash]×n. */
	bushesNear(px: number, pz: number, reach: number): Float64Array | null {
		return this.#call((g) => g.bushes_near(px, pz, reach));
	}

	/** Per-kind MIGRATION weight from the sim, by Kind order [rabbit,cat,kangaroo,person,lion,dinosaur]. */
	migrateWeights(): Float64Array | null {
		return this.#call((g) => g.migrate_weights());
	}

	/** Per-kind female FERTILE WINDOW (seconds): maturity → menopause/old-age. Kind order
	 *  [rabbit,cat,kangaroo,person,lion,dinosaur]. The HUD's TFR estimate multiplies the live per-female birth rate
	 *  by this — using the sim's own breeding numbers, so the readout can't drift from the simulation. */
	fertileWindows(): Float64Array | null {
		return this.#call((g) => g.fertile_windows());
	}

	/** World-AREA carrying-capacity multiplier from the BUILT count — Rust owns the formula (single source of truth);
	 *  JS just counts the buildings and calls this. Pre-wasm-load → neutral 1 (every real call site has it loaded). */
	worldAreaScale(builds: number): number {
		return this.#call((g) => g.world_area_scale(builds)) ?? 1;
	}

	/** Per-kind GESTATION seconds by Kind order [rabbit,cat,kangaroo,person,lion,dinosaur] (prefer `personGestation`). */
	gestationSecs(): Float64Array | null {
		return this.#call((g) => g.gestation_secs());
	}

	/** Pond per-id SEED — Rust's source of truth for the shoreline (the render keeps a native copy; a test pins it). */
	waterSeed(id: string): number | null {
		return this.#call((g) => g.water_seed(id));
	}

	/** Kind FOOTPRINT [radius, height] — the collision source of truth (engine.rs kind_rh); a test pins the JS KINDS
	 *  copy to this. Unknown kind → the fallback [1, 2]. */
	kindRh(kind: string): Float64Array | null {
		return this.#call((g) => g.kind_rh(kind));
	}

	/** AMBIENT terrain height at (x,z), no features — Rust's copy of the render's heightAt (a test pins the JS one). */
	terrainHeight(x: number, z: number): number | null {
		return this.#call((g) => g.terrain_height(x, z));
	}

	/** Rust's addressed-RNG primitives (rng.rs) — the JS rng.ts keeps a native copy for render-side seeding; a test
	 *  pins it to these. `hash`/`hashKeys` return a uint32; `rand` a float in [0,1); `seedFrom` a string→uint32. */
	rngHash(position: number, seed: number): number | null {
		return this.#call((g) => g.rng_hash(position, seed));
	}
	rngHashKeys(seed: number, keys: number[]): number | null {
		return this.#call((g) => g.rng_hash_keys(seed, Int32Array.from(keys)));
	}
	rngRand(seed: number, keys: number[]): number | null {
		return this.#call((g) => g.rng_rand(seed, Int32Array.from(keys)));
	}
	rngSeedFrom(s: string): number | null {
		return this.#call((g) => g.rng_seed_from(s));
	}

	/** Pond SHORELINE radius factor at `ang` for `seed` — Rust's source of truth (render copy pinned by a test). */
	waterEdgeFactor(seed: number, ang: number): number | null {
		return this.#call((g) => g.water_edge_factor(seed, ang));
	}

	/** The render slice of the eco table — [rank, speed_lo, speed_hi] per kind, by Kind order (eco.rs is the truth). */
	ecoRender(): Float64Array | null {
		return this.#call((g) => g.eco_render());
	}

	/** The VIGOR gene bounds [min, max] — the sim's source of truth (prefer the cached `clampGene` helper below). */
	geneBounds(): Float64Array | null {
		return this.#call((g) => g.gene_bounds());
	}

	/** Sim ticks per second (1/DT) from the sim's clock (cached) — region streaming derives dormant-span seconds from
	 *  this, no duplicated `30`. 30 only as the pre-wasm-load fallback. */
	tickHz(): number {
		if (!this.#tickHzCache) {
			const h = this.#call((g) => g.tick_hz());
			if (h) this.#tickHzCache = h;
		}
		return this.#tickHzCache || 30;
	}

	/** THE BINARY ENGINE (no JSON) — the op→world layer. `p` is the world+ops packed as parallel string vecs +
	 *  flat f64 SoA (engine.ts owns pack/unpack). Reads the wasm getters once into a plain object, then frees it. */
	applyOpsBin(p: PackedApply): RawApplyResult | null {
		return this.#call((g) => {
			const r = g.apply_ops_bin(
				p.objIds,
				p.objKinds,
				p.objColors,
				p.objNum,
				p.zoneIds,
				p.zoneMaterials,
				p.zoneShapes,
				p.zoneNum,
				p.pathIds,
				p.pathMaterials,
				p.pathNum,
				p.terrainNum,
				p.ground,
				p.sky,
				p.opNum,
				p.opStrs,
				p.px,
				p.pz,
				p.yaw,
			);
			const out: RawApplyResult = {
				objIds: r.obj_ids,
				objKinds: r.obj_kinds,
				objColors: r.obj_colors,
				objNum: r.obj_num,
				zoneIds: r.zone_ids,
				zoneMaterials: r.zone_materials,
				zoneShapes: r.zone_shapes,
				zoneNum: r.zone_num,
				pathIds: r.path_ids,
				pathMaterials: r.path_materials,
				pathNum: r.path_num,
				terrainNum: r.terrain_num,
				ground: r.ground,
				sky: r.sky,
				conflictLabels: r.conflict_labels,
				conflictBlockers: r.conflict_blockers,
			};
			r.free();
			return out;
		});
	}

	/** STATELESS settlement wall refit (binary, jzon-free) — for the fast-forward/away-growth path, which can't use the
	 *  persistent `wgSettlement` store (the live renderer owns its incremental state). `soa` = packStructures(world),
	 *  `zones` = packWaterZones(world). Returns the GEN op stream (decode like applyStructOps; REMOVE slot = `soa` index). */
	settlementOpsBin(soa: Float64Array, zones: Float64Array): Float64Array | null {
		return this.#call((g) => g.settlement_ops_bin(soa, zones));
	}

	// ── BINARY worldgen (the StructureStore path, docs/world-data-architecture.md) — no JSON. Seed the store from the
	//    bounded live structures whenever they change, then call the ops with small typed-array inputs; each returns a
	//    flat Float64Array op stream [op(0=add,1=remove-slot), kind, x, z, rot, sx, sy, sz, color]×n. ────────────────
	/** True once the binary structure store is instantiated (load complete). */
	get hasStore(): boolean {
		return this.#worldgen !== null;
	}
	/** Replace the store from a packed SoA `[kind,x,z,rot,sx,sy,sz,color,keep]×n` (world.objects' structures, in order). */
	seedStructures(soa: Float64Array): void {
		this.#worldgen?.seed(soa);
	}
	wgWell(reqs: Float64Array, zones: Float64Array): Float64Array | null {
		return this.#worldgen ? this.#worldgen.well(reqs, zones) : null;
	}
	wgBuild(reqs: Float64Array, zones: Float64Array): Float64Array | null {
		return this.#worldgen ? this.#worldgen.build(reqs, zones) : null;
	}
	/** DORMANT settlement growth — grow a FAR cluster's homes. `houses` = `[x,z]×n`; returns up to `want` build ops
	 *  `[OP_ADD, kind, x, z, rot, sx, sy, sz, color]×k` (a throwaway store; the live structures are untouched). */
	wgGrowDormant(houses: Float64Array, want: number, zones: Float64Array, seed: number): Float64Array | null {
		return this.#worldgen ? this.#worldgen.grow_dormant(houses, want, zones, seed) : null;
	}
	wgGrave(dx: number, dz: number, zones: Float64Array): Float64Array | null {
		return this.#worldgen ? this.#worldgen.grave(dx, dz, zones) : null;
	}
	wgVeg(seed: number, zones: Float64Array): Float64Array | null {
		return this.#worldgen ? this.#worldgen.veg(seed, zones) : null;
	}
	/** `changed` = `[x,z]×n` positions of structures changed this frame → only those towns' walls re-fit; empty = all. */
	wgSettlement(zones: Float64Array, changed: Float64Array): Float64Array | null {
		return this.#worldgen ? this.#worldgen.settlement(zones, changed) : null;
	}
	/** LAKE generator (binary) — `zones` = water zones `[px,pz,size,seed]×n`. Returns the GEN op stream (decodeGenOps). */
	wgLake(zones: Float64Array, px: number, pz: number, yaw: number): Float64Array | null {
		return this.#worldgen ? this.#worldgen.lake(zones, px, pz, yaw) : null;
	}
	/** FOREST generator (binary) — reads trees from the seeded store + water `zones`. Returns the GEN op stream. */
	wgForest(zones: Float64Array, px: number, pz: number, yaw: number): Float64Array | null {
		return this.#worldgen ? this.#worldgen.forest(zones, px, pz, yaw) : null;
	}
	/** CITY generator (binary) — reads buildings from the seeded store, water `zones`, removable spokes/plaza
	 *  (`removables` = `[tag,x,z]×n`). Returns the GEN op stream (a REMOVE's slot → path/plaza id via decodeGenOps). */
	wgCity(zones: Float64Array, removables: Float64Array, px: number, pz: number, yaw: number): Float64Array | null {
		return this.#worldgen ? this.#worldgen.city(zones, removables, px, pz, yaw) : null;
	}
	/** IMMIGRATION (binary) — `counts` = `[n,geneSum]×5` (FLOORS order). Returns a flat `[floorIdx,x,z,gene]×n` stream. */
	wgImmigration(counts: Float64Array, px: number, pz: number, globalAvg: number, seed: number): Float64Array | null {
		return this.#worldgen ? this.#worldgen.immigration(counts, px, pz, globalAvg, seed) : null;
	}
	/** SETTLEMENT PLAN (binary) — packed `[radius, numPaths, numObjects, <paths×4>, <objects×7>]` (settlementPlanner decodes). */
	wgTownPlan(cx: number, cz: number, size: string, seed: number): Float64Array | null {
		return this.#worldgen ? this.#worldgen.town_plan(cx, cz, size, seed) : null;
	}
	/** DEMO GALLERY (binary) — Rust owns the gallery layout; returns `[numSites, numPaths, numObjects, <sites×3>, <paths×4>, <objects×7>]`. */
	wgDemoGallery(): Float64Array | null {
		return this.#worldgen ? this.#worldgen.demo_gallery() : null;
	}
	/** Binary snapshot of the live structures (→ IndexedDB, no JSON) / restore. */
	wgSerialize(): Uint8Array | null {
		return this.#worldgen ? this.#worldgen.serialize() : null;
	}
	wgDeserialize(buf: Uint8Array): void {
		this.#worldgen?.deserialize(buf);
	}
	wgLen(): number {
		return this.#worldgen ? this.#worldgen.len() : 0;
	}

	/** Closed-form VIGOR drift for a dormant region over `dtSec` away (Rust). Falls back to the unchanged gene. */
	ffGene(gene: number, c: Record<string, number>, dtSec: number): number {
		return this.#call((g) => g.ff_gene(gene, c.rabbit ?? 0, c.cat ?? 0, c.kangaroo ?? 0, c.person ?? 0, c.lion ?? 0, c.dinosaur ?? 0, dtSec), gene) as number;
	}

	/** Clamp a vigor gene to the sim's [GENE_MIN, GENE_MAX] — bounds come from Rust (gene_bounds, cached), not a
	 *  copied 0.6/1.6 in six places. */
	clampGene(v: number): number {
		if (!this.#geneSynced) {
			const b = this.geneBounds();
			if (b) ((this.#geneLo = b[0]), (this.#geneHi = b[1]), (this.#geneSynced = true));
		}
		return v < this.#geneLo ? this.#geneLo : v > this.#geneHi ? this.#geneHi : v;
	}

	/** A person's gestation in seconds from the sim (cached) — so the pregnancy belly-grow lands exactly at delivery.
	 *  72 only as the pre-wasm-load fallback (matches world::gestation(Person)); person is index 3 in Kind order. */
	personGestation(): number {
		if (!this.#personGestationCache) {
			const g = this.gestationSecs();
			if (g && g.length > 3) this.#personGestationCache = g[3];
		}
		return this.#personGestationCache || 72;
	}
}

/** The world's MATH — the single main-thread wasm-math instance, owning the wasm handle, the load lifecycle, AND
 *  the cached sim constants. Use `math.pondsNear(...)`, `math.clampGene(...)`, `math.tickHz()`, `math.init()`, etc.
 *  (The name says what it is, not how it's built; the wasm is an implementation detail.) */
export const math = new WorldMath();
