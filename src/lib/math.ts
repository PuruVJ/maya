// MAIN-THREAD WASM utilities — the Rust core's PURE, stateless math, made callable on the main thread so the
// "all real math lives in Rust" north star holds even for the LOAD-time / BUILD-time computations that don't run
// in the sim worker. This is a SECOND, tiny wasm instance (no Sim state, no per-tick loop) — the same .wasm bytes
// the worker already fetched (browser-cached), just a second WebAssembly instance for stateless calls.
//
// ONE stateful object — `math` — owns the wasm handle + the load lifecycle, with a SINGLE `#call` guard so each
// method is a one-liner (no `if (glue)` repeated per function). `await math.init()` once at startup, then call
// `math.pondsNear(...)`, `math.ffTargets(...)`, etc. Methods return the result, or a permissive sentinel (usually
// null) if the wasm somehow isn't loaded yet (never a duplicated JS formula). Names say WHAT, not the technology.

import type { WorldObject, Path } from './world';
import type { Op } from './engine';

interface MathGlue {
	default: (input?: unknown) => Promise<unknown>;
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
	apply_ops: (world_json: string, ops_json: string, px: number, pz: number, yaw: number) => string;
	settlement_plan: (cx: number, cz: number, size: string, seed: number, id_prefix: string) => string;
	forest_ops: (world_json: string, px: number, pz: number, yaw: number) => string;
	lake_ops: (world_json: string, px: number, pz: number, yaw: number) => string;
	city_ops: (world_json: string, px: number, pz: number, yaw: number) => string;
}

class WorldMath {
	#glue: MathGlue | null = null;
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
				} else {
					// NODE (vitest — the engine runs in Rust now, so even node tests use the wasm): load the bytes from
					// disk; the web-target `default(init)` accepts a BufferSource.
					const fs = await import('node:fs');
					const jsUrl = new URL('../../static/worldsim/worldsim.js', import.meta.url);
					const wasmUrl = new URL('../../static/worldsim/worldsim_bg.wasm', import.meta.url);
					const m = (await import(/* @vite-ignore */ jsUrl.href)) as unknown as MathGlue;
					await m.default({ module_or_path: fs.readFileSync(wasmUrl) } as unknown as undefined);
					this.#glue = m;
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

	/** THE ENGINE — apply `ops` to a world (both JSON strings) for a player at (px,pz,yaw). New world + conflicts. */
	applyOps(worldJson: string, opsJson: string, px: number, pz: number, yaw: number): { world: unknown; conflicts: unknown[] } | null {
		return this.#call((g) => JSON.parse(g.apply_ops(worldJson, opsJson, px, pz, yaw)) as { world: unknown; conflicts: unknown[] });
	}

	/** PROCEDURAL SETTLEMENT PLAN — Rust owns the world-gen. A planned town at (cx,cz) of `size`, deterministic in
	 *  `seed`. Returns the world-objects + road paths + footprint radius (or null pre-wasm-load). */
	settlementPlan(cx: number, cz: number, size: string, seed: number, idPrefix: string): { objects: WorldObject[]; paths: Path[]; radius: number } | null {
		return this.#call((g) => JSON.parse(g.settlement_plan(cx, cz, size, seed, idPrefix)) as { objects: WorldObject[]; paths: Path[]; radius: number });
	}

	/** FOREST generator — engine Ops that plant/grow a wood ahead of the player. Reads the world DOM (JSON). */
	forestOps(worldJson: string, px: number, pz: number, yaw: number): Op[] | null {
		return this.#call((g) => JSON.parse(g.forest_ops(worldJson, px, pz, yaw)) as Op[]);
	}

	/** LAKE generator — engine Ops that dig/grow a pond ahead of the player. Reads the world DOM (JSON). */
	lakeOps(worldJson: string, px: number, pz: number, yaw: number): Op[] | null {
		return this.#call((g) => JSON.parse(g.lake_ops(worldJson, px, pz, yaw)) as Op[]);
	}

	/** CITY generator — engine Ops that build/grow a concentric district-zoned city. Reads the world DOM (JSON). */
	cityOps(worldJson: string, px: number, pz: number, yaw: number): Op[] | null {
		return this.#call((g) => JSON.parse(g.city_ops(worldJson, px, pz, yaw)) as Op[]);
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
