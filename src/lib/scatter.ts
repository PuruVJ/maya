// Ambient-forest placement — now a thin adapter over the RUST field (engine.rs `trees_near`/`bushes_near`), so
// Rust is the single source of truth for WHERE scatter trees/bushes stand (render AND collision read the same
// field). One windowed wasm call per query (cheap serialization — not per cell). JS keeps only the render-side
// AVOIDANCE that depends on JS-owned data: `onPath` (player-dug roads) + `treeRadius` (collision). The deterministic
// placement that used to live here moved to Rust (its sin-hash is a hair different there → the forest is reshuffled
// vs the old JS one, but consistent). See the `rust-owns-all-compute` memory.
import type { Path } from './world';
import { math } from './math';

export const SCATTER_STEP = 16; // forest grid cell (m) — mirrors engine.rs SCATTER_STEP (kept for callers' ranges)
export const SCATTER_CLEAR = 70; // spawn/build area kept tree-free (radius from origin)
export const BUSH_STEP = 11;

export interface ScatterTree {
	x: number;
	z: number;
	scale: number; // horizontal scale (trunk + canopy)
	scaleY: number; // vertical scale
	rot: number; // y rotation
	colorHash: number; // [0,1) deterministic per-tree → leaf colour (from Rust)
}

export interface ScatterBush {
	x: number;
	z: number;
	scale: number;
	rot: number;
	colorHash: number;
}

/** Trunk collision radius for a tree of this scale (matches the rendered trunk, a touch generous). */
export function treeRadius(scale: number): number {
	return 0.3 * scale;
}

/** Visit every TREE within `reach` of (x,z) — from the Rust forest field (one wasm call). No-op until wasm loads. */
export function forEachTreeNear(x: number, z: number, reach: number, cb: (t: ScatterTree) => void): void {
	const f = math.treesNear(x, z, reach);
	if (!f) return;
	for (let k = 0; k < f.length; k += 6) {
		cb({ x: f[k], z: f[k + 1], scale: f[k + 2], scaleY: f[k + 3], rot: f[k + 4], colorHash: f[k + 5] });
	}
}

/** Visit every BUSH within `reach` of (x,z) — from the Rust field (one wasm call). No-op until wasm loads. */
export function forEachBushNear(x: number, z: number, reach: number, cb: (b: ScatterBush) => void): void {
	const f = math.bushesNear(x, z, reach);
	if (!f) return;
	for (let k = 0; k < f.length; k += 5) {
		cb({ x: f[k], z: f[k + 1], scale: f[k + 2], rot: f[k + 3], colorHash: f[k + 4] });
	}
}

/** Is (x, z) on/over any path (road or river)? Keeps ambient trees out of streets — matching the grass carve —
 *  shared by AmbientScatter (cull) + Player (don't collide with a culled tree). Half-width plus a small margin. */
export function onPath(paths: Path[] | undefined, x: number, z: number): boolean {
	for (const p of paths ?? []) {
		const ax = p.from[0];
		const az = p.from[2];
		const abx = p.to[0] - ax;
		const abz = p.to[2] - az;
		const t = Math.max(0, Math.min(1, ((x - ax) * abx + (z - az) * abz) / (abx * abx + abz * abz || 1e-4)));
		const dx = x - (ax + abx * t);
		const dz = z - (az + abz * t);
		const r = p.width / 2 + 0.6;
		if (dx * dx + dz * dz < r * r) return true;
	}
	return false;
}
