// MAIN-THREAD WASM utilities — the Rust core's PURE, stateless math, made callable on the main thread so the
// "all real math lives in Rust" north star holds even for the LOAD-time / BUILD-time computations that don't run
// in the sim worker. This is a SECOND, tiny wasm instance (no Sim state, no per-tick loop) — the same .wasm bytes
// the worker already fetched (browser-cached), just a second WebAssembly instance for stateless calls.
//
// ONE stateful object — `math` — owns the wasm handle + the load lifecycle, with a SINGLE `#call` guard so each
// method is a one-liner (no `if (glue)` repeated per function). `await math.init()` once at startup, then call
// `math.pondsNear(...)`, `math.ffTargets(...)`, etc. Methods return the result, or a permissive sentinel (usually
// null) if the wasm somehow isn't loaded yet (never a duplicated JS formula). Names say WHAT, not the technology.

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
	eco_render: () => Float64Array;
	gene_bounds: () => Float64Array;
	tick_hz: () => number;
	apply_ops: (world_json: string, ops_json: string, px: number, pz: number, yaw: number) => string;
}

class WorldMath {
	#glue: MathGlue | null = null;
	#loading: Promise<void> | null = null;

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

	/** The render slice of the eco table — [rank, speed_lo, speed_hi] per kind, by Kind order (eco.rs is the truth). */
	ecoRender(): Float64Array | null {
		return this.#call((g) => g.eco_render());
	}

	/** The VIGOR gene bounds [min, max] — the sim's source of truth (prefer the cached `clampGene` helper below). */
	geneBounds(): Float64Array | null {
		return this.#call((g) => g.gene_bounds());
	}

	/** Sim ticks per second (1/DT) from the sim's clock — region streaming derives dormant-span seconds from this. */
	tickHz(): number | null {
		return this.#call((g) => g.tick_hz());
	}

	/** THE ENGINE — apply `ops` to a world (both JSON strings) for a player at (px,pz,yaw). New world + conflicts. */
	applyOps(worldJson: string, opsJson: string, px: number, pz: number, yaw: number): { world: unknown; conflicts: unknown[] } | null {
		return this.#call((g) => JSON.parse(g.apply_ops(worldJson, opsJson, px, pz, yaw)) as { world: unknown; conflicts: unknown[] });
	}

	/** Closed-form VIGOR drift for a dormant region over `dtSec` away (Rust). Falls back to the unchanged gene. */
	ffGene(gene: number, c: Record<string, number>, dtSec: number): number {
		return this.#call((g) => g.ff_gene(gene, c.rabbit ?? 0, c.cat ?? 0, c.kangaroo ?? 0, c.person ?? 0, c.lion ?? 0, c.dinosaur ?? 0, dtSec), gene) as number;
	}
}

/** The world's MATH — the single main-thread wasm-math instance. Use `math.pondsNear(...)`, `math.init()`, etc.
 *  (The name says what it is, not how it's built; the wasm is an implementation detail.) */
export const math = new WorldMath();

// ── cached sim constants (read ONCE from the source of truth, no duplicated literals scattered around) ───────────
let geneLo = 0.6; // overwritten from Rust on first use (these literals are only the pre-wasm-load fallback)
let geneHi = 1.6;
let geneSynced = false;
/** Clamp a vigor gene to the sim's [GENE_MIN, GENE_MAX] — bounds come from Rust (gene_bounds), not a copied 0.6/1.6. */
export function clampGene(v: number): number {
	if (!geneSynced) {
		const b = math.geneBounds();
		if (b) ((geneLo = b[0]), (geneHi = b[1]), (geneSynced = true));
	}
	return v < geneLo ? geneLo : v > geneHi ? geneHi : v;
}

let tickHzCache = 0;
/** Sim ticks per second from the sim's clock (cached). 30 only as the pre-wasm-load fallback. */
export function tickHz(): number {
	if (!tickHzCache) {
		const h = math.tickHz();
		if (h) tickHzCache = h;
	}
	return tickHzCache || 30;
}
