// Deterministic ambient-forest placement — the SINGLE source of truth for WHERE scatter trees stand, so
// the renderer (AmbientScatter.svelte) and collision (Player / agents) agree exactly. Pure analytic
// function of the world cell (same world-stable hashing as Grass/Birds), so anyone can ask "is there a
// tree here?" without the renderer. See docs/ + [[game-work-queue]] item 5.
import type { Path } from './world';

export const SCATTER_STEP = 16; // forest grid cell (m)
export const SCATTER_CLEAR = 70; // spawn/build area kept tree-free (radius from origin)

const hash = (i: number, j: number, s: number): number => {
	const v = Math.sin(i * 127.1 + j * 311.7 + s * 74.7) * 43758.5453;
	return v - Math.floor(v);
};
// clumps trees into forests rather than an even spread
const forest = (x: number, z: number): number =>
	Math.sin(x * 0.018 + 2) * Math.cos(z * 0.016 - 1) + 0.4 * Math.sin(x * 0.05) * Math.cos(z * 0.045);

export interface ScatterTree {
	x: number;
	z: number;
	scale: number; // horizontal scale (trunk + canopy)
	scaleY: number; // vertical scale
	rot: number; // y rotation
}

/** The tree standing in cell (ci, cj), or null if that cell has none (clearing / sparse forest). */
export function treeAt(ci: number, cj: number): ScatterTree | null {
	const cellX = ci * SCATTER_STEP;
	const cellZ = cj * SCATTER_STEP;
	if (cellX * cellX + cellZ * cellZ < SCATTER_CLEAR * SCATTER_CLEAR) return null; // keep spawn clear
	if (forest(cellX, cellZ) + (hash(ci, cj, 1) - 0.5) < 0.35) return null; // forest clumps only
	return {
		x: cellX + (hash(ci, cj, 2) - 0.5) * SCATTER_STEP,
		z: cellZ + (hash(ci, cj, 3) - 0.5) * SCATTER_STEP,
		scale: 1.3 + hash(ci, cj, 4) * 1.6,
		scaleY: 1.3 + hash(ci, cj, 4) * 1.6 + hash(ci, cj, 6) * 0.8,
		rot: hash(ci, cj, 5) * 6.283
	};
}

/** Trunk collision radius for a tree of this scale (matches the rendered trunk, a touch generous). */
export function treeRadius(scale: number): number {
	return 0.3 * scale;
}

// Ambient BUSHES — low shrubs sprinkled across the open grassland (not clumped like the forest). Soft, so
// there's no collision (you brush through them); render-only, world-stable on their own coarse grid.
export const BUSH_STEP = 11;
const bhash = (i: number, j: number, s: number): number => {
	const v = Math.sin(i * 157.3 + j * 271.9 + s * 53.1) * 43758.5453;
	return v - Math.floor(v);
};
export interface ScatterBush {
	x: number;
	z: number;
	scale: number;
	rot: number;
}
/** The bush in cell (ci, cj), or null — ~1 in 5 cells, jittered, so bushes dot the meadow sparsely. */
export function bushAt(ci: number, cj: number): ScatterBush | null {
	const cellX = ci * BUSH_STEP;
	const cellZ = cj * BUSH_STEP;
	if (cellX * cellX + cellZ * cellZ < SCATTER_CLEAR * SCATTER_CLEAR) return null; // keep spawn clear
	if (bhash(ci, cj, 1) > 0.2) return null;
	return {
		x: cellX + (bhash(ci, cj, 2) - 0.5) * BUSH_STEP,
		z: cellZ + (bhash(ci, cj, 3) - 0.5) * BUSH_STEP,
		scale: 0.55 + bhash(ci, cj, 4) * 0.75,
		rot: bhash(ci, cj, 5) * 6.283
	};
}

/** Is (x, z) on/over any path (road or river)? Used to keep ambient trees out of streets — matching the
 *  grass carve — and shared by AmbientScatter (cull) + Player (don't collide with a culled tree). Half-width
 *  plus a small margin so trunks don't poke the verge. */
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

/** Visit every tree whose cell could place it within `reach` metres of (x, z) — for collision push-out. */
export function forEachTreeNear(x: number, z: number, reach: number, cb: (t: ScatterTree) => void): void {
	// a tree is jittered up to ±STEP/2 from its cell centre, so widen the cell search by that margin
	const span = Math.ceil((reach + SCATTER_STEP / 2) / SCATTER_STEP);
	const cx = Math.round(x / SCATTER_STEP);
	const cz = Math.round(z / SCATTER_STEP);
	for (let ci = cx - span; ci <= cx + span; ci++) {
		for (let cj = cz - span; cj <= cz + span; cj++) {
			const t = treeAt(ci, cj);
			if (t) cb(t);
		}
	}
}
