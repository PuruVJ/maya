// Canonical world-state types + builders. This is what gets gzip+base64'd into the URL.
import { math } from './math';
import DEMO_SNAPSHOT from './demoWorld.json';

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
	pfamA?: number; // mother's lineage id (from the Rust birth) → set on the sim agent at spawn for incest avoidance
	pfamB?: number; // father's lineage id
	genome?: number[]; // inherited behaviour genome (5 weights, from the Rust birth) → set on the sim agent at spawn
	ageFrac?: number; // 0..1 life fraction — saved into the share link so a reload restores exact age (adults stay adult)
	keep?: boolean; // PLAYER/LLM-placed → never reclaimed by habitation decay (only emergent NPC homes can rot away)
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

const BUILDING_KINDS = new Set(['house', 'cabin', 'tower']);
// DEVELOPMENT → a population multiplier. Keyed on the size of the BUILT settlement (houses), NOT the scattered
// ambient trees (which span the whole wilderness → a huge spurious footprint that ballooned the caps). A growing
// city is what should lift the world's carrying capacity: build out → more people/animals → more building (the
// emergent-city feedback). A fresh, cityless world sits at 1. Clamped [1, 3.5]. Fed to the Rust sim's live
// breeding cap (cap_for).
export function worldAreaScale(objects: { kind: string }[]): number {
	let builds = 0;
	for (const o of objects) if (BUILDING_KINDS.has(o.kind)) builds++;
	return math.worldAreaScale(builds); // Rust owns the FORMULA (single source of truth); JS only counts the buildings
}

// NOTE: there is deliberately NO load-time population trim. A world's population is DURABLE — it accumulates as
// the player + the sim grow it, and reloading must never snap it back (the old `capCreatures` carrying-cap trim
// caused "140 humans → 56 on reload"). VITALITY is Mother Nature's job: the director (nature.svelte.ts) tunes the
// living population over time, and the Rust sim's `cap_for` governs live BREEDING. Persistence just round-trips
// whatever exists.


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
	start?: { x: number; z: number; yaw: number; y?: number }; // y persists the player's HEIGHT (reload mid-air → resume mid-air)
	/** Wall-clock ms when this world was last persisted. The seam for the time-based fast-forward (big-world.md
	 *  §3): on load we know how long you were away, so the world can deterministically advance to "now". */
	savedAt?: number;
	/** DORMANT-region aggregates (big-world.md §3 streaming, see streaming.ts): a far region's creatures collapse to
	 *  a cheap per-kind headcount + lastTick instead of being individually simulated. Keyed by region cell "cx,cz".
	 *  Absent → the world has never streamed (everything is live objects). */
	regions?: Record<string, RegionAggregate>;
}

/** A dormant region's collapsed content — what `streaming.ts` stores instead of LIVE objects, so a far region costs
 *  ~nothing (and isn't in `world.objects`) until the player returns. Creatures collapse to a lossy aggregate (counts
 *  + avg gene, fast-forwarded on wake); STATIC structures are kept verbatim (durable delta, restored exactly). This
 *  is what bounds the LIVE object count to the near regions, systemically (no hard cap). */
export interface RegionAggregate {
	counts: Record<string, number>; // live count per creature kind at sleep time
	gene: number; // average vigour of the collapsed creatures (re-seeded into materialised ones)
	statics: WorldObject[]; // the region's non-creature objects (houses/trees/…), kept verbatim → restored on wake
	lastTick: number; // sim tick when it went dormant → fast-forward span on wake
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

// A populated scene to walk around in before the LLM is wired up. PRE-GENERATED snapshot (src/lib/demoWorld.json,
// produced once from the now-Rust engine's ops) so building the demo needs NO engine call at init — the engine is
// wasm (loaded async) and `demoWorld()` runs at component construction, before the wasm is ready. structuredClone
// → a fresh, independently-mutable world each call. To regenerate after changing the recipe, see scripts.
export function demoWorld(): World {
	return structuredClone(DEMO_SNAPSHOT as unknown as World);
}

/** The world's brand title (single source of truth = the demo snapshot). The local world's name is app branding,
 *  not user content — there's no rename UI — so a cached world from a previous name should adopt the current one. */
export const WORLD_NAME = (DEMO_SNAPSHOT as unknown as World).name;
/** Names this world has shipped under; a cached local world bearing one of these is migrated to WORLD_NAME on load. */
export const LEGACY_WORLD_NAMES = ['Hello World'];
