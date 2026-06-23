// MAIN-THREAD WASM utilities — the Rust core's PURE, stateless math, made callable on the main thread so the
// "all real math lives in Rust" north star holds even for the LOAD-time / BUILD-time computations that don't run
// in the sim worker. This is a SECOND, tiny wasm instance (no Sim state, no per-tick loop) — the same .wasm bytes
// the worker already fetched (browser-cached), just a second WebAssembly instance for stateless calls.
//
// Lazy: call `initRustMath()` once at startup and await it before any cap/load math. The pure getters return the
// Rust result, or a permissive sentinel if it somehow isn't loaded (never a duplicated JS formula).

interface MathGlue {
	default: (input?: unknown) => Promise<unknown>;
	pop_caps: (rabbit: number, cat: number, kangaroo: number, person: number, lion: number, dino: number, scale: number) => Uint32Array;
	ff_targets: (rabbit: number, cat: number, kangaroo: number, person: number, lion: number, dino: number, scale: number, dt: number) => Uint32Array;
	ff_gene: (gene: number, rabbit: number, cat: number, kangaroo: number, person: number, lion: number, dino: number, dt: number) => number;
	band_spread: (count: number, ax: number, az: number, r: number) => Float64Array;
	ponds_near: (px: number, pz: number, reach: number) => Float64Array;
	apply_ops: (world_json: string, ops_json: string, px: number, pz: number, yaw: number) => string;
}

let glue: MathGlue | null = null;
let loading: Promise<void> | null = null;

/** Load the main-thread wasm math instance (idempotent). Resolves once the pure functions are callable. */
export function initRustMath(): Promise<void> {
	if (glue) return Promise.resolve();
	if (!loading) {
		loading = (async () => {
			if (typeof location !== 'undefined' && typeof location.origin === 'string') {
				// BROWSER: same glue the worker loads; @vite-ignore so Vite doesn't bundle the static wasm pkg
				const m = (await import(/* @vite-ignore */ `${location.origin}/worldsim/worldsim.js`)) as unknown as MathGlue;
				await m.default();
				glue = m;
			} else {
				// NODE (vitest — the engine runs in Rust now, so even node tests use the wasm): load the bytes from
				// disk; the web-target `default(init)` accepts a BufferSource.
				const fs = await import('node:fs');
				const jsUrl = new URL('../../static/worldsim/worldsim.js', import.meta.url);
				const wasmUrl = new URL('../../static/worldsim/worldsim_bg.wasm', import.meta.url);
				const m = (await import(/* @vite-ignore */ jsUrl.href)) as unknown as MathGlue;
				await m.default({ module_or_path: fs.readFileSync(wasmUrl) } as unknown as undefined);
				glue = m;
			}
		})().catch((e) => {
			console.error('[rustMath] failed to load wasm math', e);
		});
	}
	return loading;
}

export const rustMathReady = (): boolean => glue !== null;

/** Carrying caps [rabbit, cat, kangaroo, person, lion, dinosaur] from the Rust `cap_for` — the single source of
 *  truth. Returns null if the wasm isn't loaded yet (caller decides a safe default). */
export function rustPopCaps(rabbit: number, cat: number, kangaroo: number, person: number, lion: number, dino: number, scale: number): Uint32Array | null {
	return glue ? glue.pop_caps(rabbit, cat, kangaroo, person, lion, dino, scale) : null;
}

/** Aggregate fast-forward target headcounts [rabbit, cat, kangaroo, person, lion, dino] after `dt` seconds away —
 *  the closed-form logistic relaxation toward carrying capacity, from Rust. Null if the wasm isn't loaded yet. */
export function rustFfTargets(rabbit: number, cat: number, kangaroo: number, person: number, lion: number, dino: number, scale: number, dt: number): Uint32Array | null {
	return glue ? glue.ff_targets(rabbit, cat, kangaroo, person, lion, dino, scale, dt) : null;
}

/** Spawn-spread band layout [x,z,…] for a big creature batch — the golden-spiral placement math, from Rust (the
 *  deterministic op→placement compute lives in the crate, not the JS engine). Null if the wasm isn't loaded yet. */
export function rustBandSpread(count: number, ax: number, az: number, r: number): Float64Array | null {
	return glue ? glue.band_spread(count, ax, az, r) : null;
}

/** NATURAL PONDS near (px,pz) within `reach` — Rust owns the world's water (an even, infinite, deterministic pond
 *  field); the renderer calls this once per area to draw them. Flat [x, z, radius, …], or null if wasm isn't loaded. */
export function rustPondsNear(px: number, pz: number, reach: number): Float64Array | null {
	return glue ? glue.ponds_near(px, pz, reach) : null;
}

/** THE ENGINE — apply `ops` to a world (both JSON strings) for a player at (px,pz,yaw). Returns the new world +
 *  any placement conflicts, or null if the wasm isn't loaded yet. The op→world compute lives in Rust (crate::engine);
 *  JS only (de)serializes + renders. */
export function rustApplyOps(worldJson: string, opsJson: string, px: number, pz: number, yaw: number): { world: unknown; conflicts: unknown[] } | null {
	return glue ? (JSON.parse(glue.apply_ops(worldJson, opsJson, px, pz, yaw)) as { world: unknown; conflicts: unknown[] }) : null;
}

/** Closed-form VIGOR drift for a dormant region over `dtSec` away (Rust) — evolves the offloaded population's mean
 *  gene under predation pressure so dormant regions evolve via the clock, not freeze. Falls back to unchanged gene. */
export function rustFfGene(gene: number, c: Record<string, number>, dtSec: number): number {
	return glue ? glue.ff_gene(gene, c.rabbit ?? 0, c.cat ?? 0, c.kangaroo ?? 0, c.person ?? 0, c.lion ?? 0, c.dinosaur ?? 0, dtSec) : gene;
}
