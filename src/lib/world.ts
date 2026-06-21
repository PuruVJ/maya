// Canonical world-state types + builders. This is what gets gzip+base64'd into the URL.
import { applyOps, type Op } from './engine';

export interface WorldObject {
	id: string;
	kind: string;
	pos: [number, number, number];
	scale?: [number, number, number];
	color?: string;
	rot?: number;
	// live-state snapshot (animals): captured into the share link so a wandered/dead creature reopens that
	// way. `pos` already holds the live position at encode time; these flag its condition.
	dead?: boolean;
	asleep?: boolean;
	juvenile?: boolean; // a Rust-bred newborn → spawns into the sim on a maturation cooldown (can't breed yet)
	gene?: number; // inherited vigor (≈1.0) from its parents → scales its speed in the sim (genetics/evolution)
}

export interface Zone {
	id: string;
	material: string;
	shape: string; // blob | rect | ring
	pos: [number, number, number];
	size: number;
}

export interface Path {
	id: string;
	material: string;
	from: [number, number, number];
	to: [number, number, number];
	width: number;
}

// A contained terrain bump (hill/mountain/dune patch). The world is flat outside all features.
export interface TerrainFeature {
	center: [number, number]; // x, z
	radius: number;
	height: number; // peak height (negative = a valley/depression)
	rough: number; // 0 = smooth mound, >0 = rolling ripple
}

/** Heal duplicate / missing object|zone|path ids in a world loaded from outside (a decoded share link or a
 *  restored cache). Legacy worlds built before the zone/path id-counter fix could carry colliding 'p'/'z' ids
 *  after a remove → Svelte `each_key_duplicate` crash on render. Reassigns any dup/missing id to a fresh unique
 *  one (per-prefix, past the highest existing). Structural type → no World-import ordering. Mutates + returns. */
export function repairIds<T extends { objects: { id: string }[]; zones?: { id: string }[]; paths?: { id: string }[] }>(world: T): T {
	const fix = (items: { id: string }[] | undefined, prefix: string): void => {
		if (!items) return;
		let next = 0;
		for (const it of items) {
			if (it.id && it.id[0] === prefix) {
				const v = parseInt(it.id.slice(1), 36);
				if (Number.isFinite(v) && v >= next) next = v + 1;
			}
		}
		const seen = new Set<string>();
		for (const it of items) {
			if (!it.id || seen.has(it.id)) it.id = prefix + (next++).toString(36);
			seen.add(it.id);
		}
	};
	fix(world.objects, 'o');
	fix(world.zones, 'z');
	fix(world.paths, 'p');
	return world;
}

const CREATURE_KINDS = new Set(['rabbit', 'cat', 'kangaroo', 'person', 'lion', 'dinosaur']);
const BUILDING_KINDS = new Set(['house', 'cabin', 'tower']);
// DEVELOPMENT → a population multiplier. Keyed on the size of the BUILT settlement (houses), NOT the scattered
// ambient trees (which span the whole wilderness → a huge spurious footprint that ballooned the caps). A growing
// city is what should lift the world's carrying capacity: build out → more people/animals → more building (the
// emergent-city feedback). A fresh, cityless world sits at 1. Clamped [1, 3.5]. Fed to the Rust sim AND used by
// capCreatures so the load-trim and the live breeding cap agree.
export function worldAreaScale(objects: { kind: string }[]): number {
	let builds = 0;
	for (const o of objects) if (BUILDING_KINDS.has(o.kind)) builds++;
	return Math.max(1, Math.min(3, 1 + builds / 40)); // ~+1 capacity per 40 buildings (softened: a big city hit 300+ agents)
}

// Live per-kind ceiling — MIRRORS crates/worldsim/src/world.rs effective_cap(): PREY scale with world AREA, each
// PREDATOR tracks a share of the live prey it eats. (Constants must match the Rust ones.)
export function popCaps(count: Record<string, number>, scale: number): Record<string, number> {
	const r = count.rabbit ?? 0;
	const k = count.kangaroo ?? 0;
	const p = count.person ?? 0;
	const c = count.cat ?? 0;
	const l = count.lion ?? 0;
	return {
		rabbit: Math.round(30 * scale), // MIRRORS world.rs PREY_DENSITY_* (trimmed to keep the agent count sane)
		kangaroo: Math.round(20 * scale),
		person: Math.round(22 * scale),
		cat: Math.max(2, Math.round(r * 0.3)),
		lion: Math.max(1, Math.round((r + k + p + c) * 0.07)),
		dinosaur: Math.max(1, Math.round((r + k + p + c + l) * 0.035))
	};
}

// Trim each creature kind to its live ceiling AT LOAD — the Rust cap only gates BREEDING, so it can't shrink a
// roster that's ALREADY over (a world saved before the caps, or before the sim starved an apex bloom down). Keeps
// the first `cap` of each kind (established founders sort ahead of later babies/immigrants), drops the surplus.
export function capCreatures<T extends { objects: { kind: string; pos: [number, number, number] }[] }>(world: T): T {
	const scale = worldAreaScale(world.objects);
	const count: Record<string, number> = {};
	for (const o of world.objects) if (CREATURE_KINDS.has(o.kind)) count[o.kind] = (count[o.kind] ?? 0) + 1;
	const cap = popCaps(count, scale);
	const seen: Record<string, number> = {};
	world.objects = world.objects.filter((o) => {
		if (!CREATURE_KINDS.has(o.kind)) return true; // a tree / house / prop — always keep
		seen[o.kind] = (seen[o.kind] ?? 0) + 1;
		return seen[o.kind] <= (cap[o.kind] ?? Infinity);
	});
	return world;
}

export interface World {
	v: number;
	name: string;
	ground: string;
	sky: string;
	spawn: [number, number, number];
	objects: WorldObject[];
	zones: Zone[];
	paths: Path[];
	terrain: TerrainFeature[];
	/** Where the player was when the link was made (decoded from the URL) → reopen standing there. Not part
	 *  of the world proper; set only by share-link decode, read once by Player to place you. */
	start?: { x: number; z: number; yaw: number };
}

export interface Player {
	pos: [number, number, number];
	yaw: number;
}

export function emptyWorld(name = 'Untitled'): World {
	return {
		v: 1,
		name,
		ground: 'grass',
		sky: 'night', // night-only game (user decision 2026-06-21) — perpetual night for atmosphere/simplicity
		spawn: [0, 0, 0],
		objects: [],
		zones: [],
		paths: [],
		terrain: []
	};
}

// A populated scene to walk around in before the LLM is wired up. Also dogfoods the engine.
export function demoWorld(): World {
	const w = emptyWorld('Hello World');
	w.sky = 'night';
	const ops: Op[] = [
		{ op: 'add', kind: 'house', pos: [0, 0, -6] },
		{ op: 'add', kind: 'cabin', pos: [-11, 0, -13] },
		{ op: 'add', kind: 'tower', pos: [10, 0, -13] },
		{ op: 'add', kind: 'well', pos: [-5, 0, -3] },
		{ op: 'add', kind: 'lamp', pos: [2.5, 0, -2] },
		{ op: 'add', kind: 'lamp', pos: [-2.5, 0, -2] },
		{ op: 'scatter', kind: 'tree', count: 20, area: 'north' },
		{ op: 'scatter', kind: 'flower', count: 16, area: 'center' },
		{ op: 'scatter', kind: 'rock', count: 6, area: 'west' },
		{ op: 'addZone', material: 'water', shape: 'blob', at: 'east', size: 20 },
		// a LIVING world on load — wildlife + villagers (the game's core, absent from the old demo): a couple of
		// cats and rabbits wander the hamlet, a kangaroo hops by the lake (it'll come down to drink), two
		// villagers mill about, and a lone dinosaur roams the far treeline → the food chain emerges as you watch.
		{ op: 'scatter', kind: 'cat', count: 2, area: 'center' },
		{ op: 'scatter', kind: 'rabbit', count: 3, area: 'center' },
		{ op: 'add', kind: 'kangaroo', pos: [9, 0, 5] },
		{ op: 'add', kind: 'person', pos: [-4, 0, -5] },
		{ op: 'add', kind: 'person', pos: [5, 0, -8] },
		{ op: 'add', kind: 'dinosaur', pos: [-16, 0, -34] }
	];
	applyOps(w, ops, { pos: [0, 0, 6], yaw: 0 });
	return w;
}
