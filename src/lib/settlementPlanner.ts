// PROCEDURAL SETTLEMENT PLANNER — thin bridge. The actual layout compute now lives in RUST
// (crates/worldsim/src/worldgen.rs `settlement_plan`), so the world-gen obeys the "Rust owns all compute" rule;
// this file just keeps the size vocabulary + delegates across the wasm boundary. The port is pinned to the
// original JS algorithm by src/lib/worldgen.test.ts. See [[rust-owns-all-compute]].
import type { WorldObject, Path } from './world';
import { math } from './math';
import { kindStr } from './structpack';

export type SettlementSize = 'hamlet' | 'village' | 'town' | 'city';
export const SIZES: SettlementSize[] = ['hamlet', 'village', 'town', 'city'];

/** Plan one settlement (houses/well/towers/fence/lamps + road Paths + footprint radius). Deterministic in
 *  (centre, size, seed). BINARY: Rust returns a packed `[radius, numPaths, numObjects, <paths×4>, <objects×7>]`
 *  stream (no JSON); we rebuild the Path/WorldObject shapes + ids here. Ids share one counter — PATHS first, then
 *  OBJECTS — matching the Rust placement order. Empty until the wasm is loaded (callers place nothing pre-load). */
export function settlementPlan(cx: number, cz: number, size: SettlementSize, seed: number, idPrefix: string): { objects: WorldObject[]; paths: Path[]; radius: number } {
	const arr = math.wgTownPlan(cx, cz, size, seed);
	if (!arr || arr.length < 3) return { objects: [], paths: [], radius: 0 };
	const radius = arr[0];
	const numPaths = arr[1];
	const numObjects = arr[2];
	let n = 0;
	let idx = 3;
	const paths: Path[] = [];
	for (let i = 0; i < numPaths; i++, idx += 4) paths.push({ id: idPrefix + 'p' + n++, material: 'path', from: [arr[idx], 0, arr[idx + 1]], to: [arr[idx + 2], 0, arr[idx + 3]], width: 3 });
	const objects: WorldObject[] = [];
	for (let i = 0; i < numObjects; i++, idx += 7) objects.push({ id: idPrefix + 'o' + n++, kind: kindStr(arr[idx]), pos: [arr[idx + 1], 0, arr[idx + 2]], rot: arr[idx + 3], scale: [arr[idx + 4], arr[idx + 5], arr[idx + 6]], keep: true });
	return { objects, paths, radius };
}
