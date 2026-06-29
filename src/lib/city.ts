// Native procedural generators for the typed "make city / forest / lake" commands — handled here, not by the LLM.
// The generation COMPUTE now lives in RUST (crates/worldsim/src/worldgen.rs); this file only matches the command
// word and delegates across the wasm boundary, emitting engine Ops so each build is collision-resolved, undoable
// and shareable like any other. Parity with the original JS is pinned by src/lib/worldgen.test.ts.
// See [[rust-owns-all-compute]] and [[architecture-ops-not-geometry]].
import type { Op } from './engine';
import type { World, Player } from './world';
import { math } from './math';
import { packStructures, packWaterZones, decodeGenOps } from './structpack';

/** Water zones as the binary `[px,pz,size,seed]×n` the generators read + the parallel id list (same order) so a
 *  returned REMOVE slot maps back to its zone id. NO JSON crosses the boundary (docs/world-data-architecture.md). */
function waterZones(world: World): { bin: Float64Array; ids: string[] } {
	const ids: string[] = [];
	for (const z of world.zones ?? []) if (z.material === 'water') ids.push(z.id);
	return { bin: packWaterZones(world.zones, (id) => math.waterSeed(id) ?? 0), ids };
}

/** The city's removable old features as `[tag(0=path-from, 1=plaza), x, z]×n` + a parallel id list (same order) — a
 *  returned REMOVE slot maps back to the path/plaza id. Paths first, then plaza zones (matches the jzon emit order). */
function cityRemovables(world: World): { bin: Float64Array; ids: string[] } {
	const ids: string[] = [];
	const lanes: number[] = [];
	for (const p of world.paths ?? []) {
		ids.push(p.id);
		lanes.push(0, p.from[0], p.from[2]);
	}
	for (const z of world.zones ?? []) if (z.material === 'plaza') {
		ids.push(z.id);
		lanes.push(1, z.pos[0], z.pos[2]);
	}
	return { bin: Float64Array.from(lanes), ids };
}

/** Ops that build (or grow) a concentric, district-zoned city centred on the city you're standing in (else ahead
 *  of you). BINARY: buildings read from the seeded store, water + removable spokes/plaza pass as typed arrays. */
export function cityOps(world: World, player: Player): Op[] {
	math.seedStructures(packStructures(world.objects, []));
	const { bin: water } = waterZones(world);
	const { bin: removables, ids } = cityRemovables(world);
	return decodeGenOps(math.wgCity(water, removables, player.pos[0], player.pos[2], player.yaw), ids);
}

/** Does this typed instruction mean "make/grow a city"? */
export function isCityCommand(cmd: string): boolean {
	return /^(make|build|grow|add|create|generate|bigger|expand)?\s*(me\s+)?(a\s+|the\s+|my\s+)?(big(ger)?\s+|huge\s+)?(city|town|village)$/.test(cmd);
}

/** Ops that plant (or grow) a forest the same way `make city` grows a city. BINARY: the store is seeded from the
 *  current structures (forest reads existing trees), water zones pass as a typed array — no JSON crossing. */
export function forestOps(world: World, player: Player): Op[] {
	math.seedStructures(packStructures(world.objects, []));
	return decodeGenOps(math.wgForest(waterZones(world).bin, player.pos[0], player.pos[2], player.yaw), []);
}

/** Does this typed instruction mean "make/grow a forest"? */
export function isForestCommand(cmd: string): boolean {
	return /^(make|build|grow|add|create|generate|plant|bigger|expand)?\s*(me\s+)?(a\s+|the\s+|my\s+)?(big(ger)?\s+|huge\s+|dense\s+)?(forest|woods?|jungle)$/.test(cmd);
}

/** Ops to dig (or enlarge) a lake ahead of you, or grow the one you're standing at. BINARY: reads only water zones
 *  (a typed array); a REMOVE returns the chosen zone's slot, mapped back to its id here. No JSON crossing. */
export function lakeOps(world: World, player: Player): Op[] {
	const { bin, ids } = waterZones(world);
	return decodeGenOps(math.wgLake(bin, player.pos[0], player.pos[2], player.yaw), ids);
}

/** Does this typed instruction mean "make/grow a lake"? */
export function isLakeCommand(cmd: string): boolean {
	return /^(make|build|dig|grow|add|create|generate|bigger|expand)?\s*(me\s+)?(a\s+|the\s+|my\s+)?(big(ger)?\s+|huge\s+)?(lake|pond)$/.test(cmd);
}
