// Native procedural generators for the typed "make city / forest / lake" commands — handled here, not by the LLM.
// The generation COMPUTE now lives in RUST (crates/worldsim/src/worldgen.rs); this file only matches the command
// word and delegates across the wasm boundary, emitting engine Ops so each build is collision-resolved, undoable
// and shareable like any other. Parity with the original JS is pinned by src/lib/worldgen.test.ts.
// See [[rust-owns-all-compute]] and [[architecture-ops-not-geometry]].
import type { Op } from './engine';
import type { World, Player } from './world';
import { math } from './math';

/** Ops that build (or grow) a concentric, district-zoned city centred on the city you're standing in (else ahead
 *  of you). Empty until the wasm is loaded. */
export function cityOps(world: World, player: Player): Op[] {
	return math.cityOps(JSON.stringify(world), player.pos[0], player.pos[2], player.yaw) ?? [];
}

/** Does this typed instruction mean "make/grow a city"? */
export function isCityCommand(cmd: string): boolean {
	return /^(make|build|grow|add|create|generate|bigger|expand)?\s*(me\s+)?(a\s+|the\s+|my\s+)?(big(ger)?\s+|huge\s+)?(city|town|village)$/.test(cmd);
}

/** Ops that plant (or grow) a forest the same way `make city` grows a city. */
export function forestOps(world: World, player: Player): Op[] {
	return math.forestOps(JSON.stringify(world), player.pos[0], player.pos[2], player.yaw) ?? [];
}

/** Does this typed instruction mean "make/grow a forest"? */
export function isForestCommand(cmd: string): boolean {
	return /^(make|build|grow|add|create|generate|plant|bigger|expand)?\s*(me\s+)?(a\s+|the\s+|my\s+)?(big(ger)?\s+|huge\s+|dense\s+)?(forest|woods?|jungle)$/.test(cmd);
}

/** Ops to dig (or enlarge) a lake ahead of you, or grow the one you're standing at. */
export function lakeOps(world: World, player: Player): Op[] {
	return math.lakeOps(JSON.stringify(world), player.pos[0], player.pos[2], player.yaw) ?? [];
}

/** Does this typed instruction mean "make/grow a lake"? */
export function isLakeCommand(cmd: string): boolean {
	return /^(make|build|dig|grow|add|create|generate|bigger|expand)?\s*(me\s+)?(a\s+|the\s+|my\s+)?(big(ger)?\s+|huge\s+)?(lake|pond)$/.test(cmd);
}
