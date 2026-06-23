// PROCEDURAL SETTLEMENT PLANNER — thin bridge. The actual layout compute now lives in RUST
// (crates/worldsim/src/worldgen.rs `settlement_plan`), so the world-gen obeys the "Rust owns all compute" rule;
// this file just keeps the size vocabulary + delegates across the wasm boundary. The port is pinned to the
// original JS algorithm by src/lib/worldgen.test.ts. See [[rust-owns-all-compute]].
import type { WorldObject, Path } from '$lib/world';
import { math } from '$lib/math';

export type SettlementSize = 'hamlet' | 'village' | 'town' | 'city';
export const SIZES: SettlementSize[] = ['hamlet', 'village', 'town', 'city'];

/** Plan one settlement (houses/well/towers/fence/lamps + road Paths + footprint radius). Deterministic in
 *  (centre, size, seed). Returns empty until the wasm is loaded (callers place nothing pre-load). */
export function settlementPlan(cx: number, cz: number, size: SettlementSize, seed: number, idPrefix: string): { objects: WorldObject[]; paths: Path[]; radius: number } {
	return math.settlementPlan(cx, cz, size, seed, idPrefix) ?? { objects: [], paths: [], radius: 0 };
}
