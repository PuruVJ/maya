// Canonical world-state types + builders. This is what gets gzip+base64'd into the URL.
import { math } from './math';
import { inWater } from './water';
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

const CREATURE_KINDS = new Set(['rabbit', 'cat', 'kangaroo', 'person', 'lion', 'dinosaur']);
const BUILDING_KINDS = new Set(['house', 'cabin', 'tower']);
// DEVELOPMENT → a population multiplier. Keyed on the size of the BUILT settlement (houses), NOT the scattered
// ambient trees (which span the whole wilderness → a huge spurious footprint that ballooned the caps). A growing
// city is what should lift the world's carrying capacity: build out → more people/animals → more building (the
// emergent-city feedback). A fresh, cityless world sits at 1. Clamped [1, 3.5]. Fed to the Rust sim's live
// breeding cap (cap_for).
export function worldAreaScale(objects: { kind: string }[]): number {
	let builds = 0;
	for (const o of objects) if (BUILDING_KINDS.has(o.kind)) builds++;
	return Math.max(1, Math.min(3, 1 + builds / 40)); // ~+1 capacity per 40 buildings (softened: a big city hit 300+ agents)
}

// NOTE: there is deliberately NO load-time population trim. A world's population is DURABLE — it accumulates as
// the player + the sim grow it, and reloading must never snap it back (the old `capCreatures` carrying-cap trim
// caused "140 humans → 56 on reload"). VITALITY is Mother Nature's job: the director (nature.svelte.ts) tunes the
// living population over time, and the Rust sim's `cap_for` governs live BREEDING. Persistence just round-trips
// whatever exists.

// Kind index order the Rust `ff_targets` returns: [rabbit, cat, kangaroo, person, lion, dino].
const FF_KINDS = ['rabbit', 'cat', 'kangaroo', 'person', 'lion', 'dinosaur'] as const;

/** DETERMINISTIC AGGREGATE FAST-FORWARD (big-world.md §3). Given how long the player was away (ms), advance the
 *  population to "now" WITHOUT replaying every tick (that would freeze the tab). The relaxation toward carrying
 *  capacity is the closed-form logistic in RUST (`ff_targets`, single source of truth) — O(1) per species, so a
 *  week away costs the same as a minute. JS only materialises the deltas: add/remove creature objects to hit the
 *  advanced counts (new arrivals carry the evolved average vigour). Returns the net population change. */
export function fastForward<T extends { objects: WorldObject[]; zones?: Zone[] }>(
	world: T,
	elapsedMs: number,
	idPrefix: string,
	groundY: (x: number, z: number) => number
): { creatures: number; houses: number } {
	const dt = Math.min(elapsedMs / 1000, 86_400); // model at most ~1 day of effect (the logistic saturates anyway)
	if (dt < 30) return { creatures: 0, houses: 0 }; // a blink away → nothing to do
	const count: Record<string, number> = {};
	let geneSum = 0;
	let geneN = 0;
	let minX = Infinity;
	let maxX = -Infinity;
	let minZ = Infinity;
	let maxZ = -Infinity;
	for (const o of world.objects) {
		if (!CREATURE_KINDS.has(o.kind) && !BUILDING_KINDS.has(o.kind)) continue;
		if (CREATURE_KINDS.has(o.kind)) {
			count[o.kind] = (count[o.kind] ?? 0) + 1;
			geneSum += o.gene ?? 1;
			geneN++;
		}
		minX = Math.min(minX, o.pos[0]);
		maxX = Math.max(maxX, o.pos[0]);
		minZ = Math.min(minZ, o.pos[2]);
		maxZ = Math.max(maxZ, o.pos[2]);
	}
	if (!Number.isFinite(minX)) return { creatures: 0, houses: 0 }; // an empty world → nothing to advance
	const avgGene = geneN > 0 ? geneSum / geneN : 1;
	const scale = worldAreaScale(world.objects);
	// The whole relaxation (rates + floors + logistic, prey-before-predators) is one Rust call — single source of truth.
	const adv = math.ffTargets(count.rabbit ?? 0, count.cat ?? 0, count.kangaroo ?? 0, count.person ?? 0, count.lion ?? 0, count.dinosaur ?? 0, scale, dt);
	if (!adv) return { creatures: 0, houses: 0 }; // wasm not loaded → don't guess, leave the world as-is
	const target: Record<string, number> = {};
	FF_KINDS.forEach((k, i) => {
		target[k] = adv[i];
	});
	// materialise the deltas — add scattered newcomers (evolved vigour) or remove the surplus
	let net = 0;
	let nid = 0;
	for (const k of Object.keys(target)) {
		const have = count[k] ?? 0;
		const want = target[k];
		if (want > have) {
			for (let i = 0; i < want - have; i++) {
				const x = minX + Math.random() * (maxX - minX);
				const z = minZ + Math.random() * (maxZ - minZ);
				const gene = Math.max(0.6, Math.min(1.6, avgGene - 0.05 + Math.random() * 0.1));
				world.objects.push({ id: idPrefix + nid++, kind: k, pos: [x, 0, z], gene });
				net++;
			}
		} else if (want < have) {
			let drop = have - want;
			for (let i = world.objects.length - 1; i >= 0 && drop > 0; i--) {
				if (world.objects[i].kind === k) ((world.objects.splice(i, 1), drop--, net--));
			}
		}
	}

	// CITY GROWTH while away — a populated town keeps raising homes (settlers build between births). Add houses in
	// proportion to the advanced people × time, grid-snapped beside the existing settlement, plot-checked + capped.
	let houses = 0;
	const blds = world.objects.filter((o) => BUILDING_KINDS.has(o.kind));
	const people = target.person ?? count.person ?? 0;
	if (blds.length >= 2 && people >= 6) {
		let toAdd = Math.min(Math.round((dt / 900) * (people / 18)), 140 - blds.length, 30); // ~1 house / 15 min / 18 people; ≤30 per jump, ≤140 total
		let attempts = 0;
		while (toAdd > 0 && attempts < 400) {
			attempts++;
			const b = blds[(Math.random() * blds.length) | 0];
			const gx = Math.round((b.pos[0] + (Math.random() - 0.5) * 26) / 8) * 8; // 8 m grid → aligned blocks
			const gz = Math.round((b.pos[2] + (Math.random() - 0.5) * 26) / 8) * 8;
			if (world.objects.some((o) => BUILDING_KINDS.has(o.kind) && Math.abs(o.pos[0] - gx) < 6 && Math.abs(o.pos[2] - gz) < 6)) continue; // plot taken
			if (inWater(world.zones, gx, gz)) continue; // don't grow a home into a lake while you were away
			const h: WorldObject = { id: idPrefix + 'h' + houses, kind: 'house', pos: [gx, groundY(gx, gz), gz] };
			world.objects.push(h);
			blds.push(h);
			houses++;
			toAdd--;
		}
	}

	// GRAVES while away — some of the dead are remembered. A few headstones near the settlement, time-proportional.
	if (blds.length >= 2 && people >= 4) {
		const existingGraves = world.objects.reduce((s, o) => s + (o.kind === 'grave' ? 1 : 0), 0);
		let toAdd = Math.min(Math.round(dt / 1200), 70 - existingGraves, 15); // ≤15 per jump, ≤70 total
		const cx = blds.reduce((s, b) => s + b.pos[0], 0) / blds.length;
		const cz = blds.reduce((s, b) => s + b.pos[2], 0) / blds.length;
		for (let g = 0; toAdd > 0; g++, toAdd--) {
			const a = Math.random() * Math.PI * 2;
			const r = 8 + Math.random() * 22; // a graveyard on the edge of town
			const gx = cx + Math.cos(a) * r;
			const gz = cz + Math.sin(a) * r;
			world.objects.push({ id: idPrefix + 'g' + g, kind: 'grave', pos: [gx, groundY(gx, gz), gz], rot: Math.random() * Math.PI * 2 });
		}
	}

	return { creatures: net, houses };
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
