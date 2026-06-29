// Canonical world-state types + builders. This is what gets gzip+base64'd into the URL.
import { math } from './math';
import { inWater } from './water';
import { packStructures, packWaterZones, kindStr, OP_STRIDE, OP_ADD } from './structpack';
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
	return math.worldAreaScale(builds); // Rust owns the FORMULA (single source of truth); JS only counts the buildings
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
	// CO-DEVELOPMENT over the away span — the fix for "came back hours later, the world was STUCK at similar numbers".
	// A single pass relaxed people to the CURRENT houses then built a few homes for the CURRENT people: a FIXED POINT, so
	// a town already at its (sparse) equilibrium never grew. Real away-growth is a SPIRAL — more people → more homes →
	// higher carrying capacity (worldAreaScale) → more people → … bounded by the per-town cap, with the surplus founding
	// NEW towns. We CHUNK the elapsed time and run that spiral each chunk (recomputing the scale from the homes built so
	// far), so an absence develops hamlets into cities + spreads fresh towns — matching what the live world-pulse + the
	// far-town development (streaming.ts) would have done over the same hours.
	const CHUNKS = 6;
	const cdt = dt / CHUNKS;
	const FOUND_GAP = 240; // MINIMUM town spacing (mirrors Rust world::FOUND_GAP) — a new anchor must clear this of every town
	const COLONY_R = 75; // a building within this of a town centroid belongs to that town (matches the sim's COLONY_R)
	const PEOPLE_PER_HOUSE = 2.8; // homes a town grows toward per settler — matches the live far-town dev + the colony equilibrium (was a sparse ~13 → towns never developed past a hamlet)
	const PER_CLUSTER_HOUSE_CAP = 30; // a town fills to ~this many homes, then its surplus founds a NEW town (matches worldgen COLONY_MAX*3)
	const GOLDEN = 2.399963229728653; // golden angle (rad) → successive new towns ring OUT evenly, never stacking on one bearing
	let creatures = 0;
	let houses = 0;
	let nid = 0;
	let hid = 0;
	let founded = 0;
	for (let chunk = 0; chunk < CHUNKS; chunk++) {
		// RE-SCAN each chunk — world.objects GREW last chunk, so the scale, the per-kind anchors, and the people target
		// all move UP (the spiral). Bounded cost (objects are bounded) × CHUNKS.
		const count: Record<string, number> = {};
		const byKindPos: Record<string, [number, number][]> = {}; // existing positions per kind → new arrivals cluster WITH their kind
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
				(byKindPos[o.kind] ??= []).push([o.pos[0], o.pos[2]]);
				geneSum += o.gene ?? 1;
				geneN++;
			}
			minX = Math.min(minX, o.pos[0]);
			maxX = Math.max(maxX, o.pos[0]);
			minZ = Math.min(minZ, o.pos[2]);
			maxZ = Math.max(maxZ, o.pos[2]);
		}
		if (!Number.isFinite(minX)) break; // an empty world → nothing to advance
		const avgGene = geneN > 0 ? geneSum / geneN : 1;
		const scale = worldAreaScale(world.objects);
		// The whole relaxation (rates + floors + logistic, prey-before-predators) is one Rust call — single source of truth.
		const adv = math.ffTargets(count.rabbit ?? 0, count.cat ?? 0, count.kangaroo ?? 0, count.person ?? 0, count.lion ?? 0, count.dinosaur ?? 0, scale, cdt);
		if (!adv) return { creatures, houses }; // wasm not loaded → don't guess, leave the rest as-is
		const target: Record<string, number> = {};
		FF_KINDS.forEach((k, i) => {
			target[k] = adv[i];
		});
		// materialise the deltas — add scattered newcomers (evolved vigour) NEAR their kind, or trim the surplus
		for (const k of Object.keys(target)) {
			const have = count[k] ?? 0;
			const want = target[k];
			if (want > have) {
				const anchors = byKindPos[k] ?? [];
				for (let i = 0; i < want - have; i++) {
					// grow each kind NEAR an existing member of its kind (its colony/herd) with a small jitter — so the
					// away-growth appears WHERE that population already lives (your colony fills out, the wild zone thickens).
					let x: number, z: number;
					if (anchors.length) {
						const a = anchors[(Math.random() * anchors.length) | 0];
						x = a[0] + (Math.random() - 0.5) * 24;
						z = a[1] + (Math.random() - 0.5) * 24;
					} else {
						x = minX + Math.random() * (maxX - minX);
						z = minZ + Math.random() * (maxZ - minZ);
					}
					const gene = math.clampGene(avgGene - 0.05 + Math.random() * 0.1);
					world.objects.push({ id: idPrefix + 'c' + nid++, kind: k, pos: [x, 0, z], gene });
					creatures++;
				}
			} else if (want < have) {
				let drop = have - want;
				for (let i = world.objects.length - 1; i >= 0 && drop > 0; i--) {
					if (world.objects[i].kind === k) ((world.objects.splice(i, 1), drop--, creatures--));
				}
			}
		}

		// CITY GROWTH + SPREAD this chunk — raise homes toward people/PEOPLE_PER_HOUSE across the existing towns; once a
		// town hits its cap, the surplus FOUNDS a new town ≥FOUND_GAP out (the populous + spread vision). The homes raised
		// here lift the area scale, so NEXT chunk's people target climbs — the development spiral that grows a hamlet into
		// a city over a long absence. Houses lead, people follow.
		const blds = world.objects.filter((o) => BUILDING_KINDS.has(o.kind));
		const people = Math.max(count.person ?? 0, target.person ?? 0);
		if (blds.length >= 2 && people >= 6) {
			type Cluster = { cx: number; cz: number; n: number };
			const clusters: Cluster[] = [];
			for (const b of blds) {
				const c = clusters.find((cl) => (cl.cx - b.pos[0]) ** 2 + (cl.cz - b.pos[2]) ** 2 < COLONY_R * COLONY_R);
				if (c) {
					c.cx = (c.cx * c.n + b.pos[0]) / (c.n + 1); // running centroid
					c.cz = (c.cz * c.n + b.pos[2]) / (c.n + 1);
					c.n++;
				} else {
					clusters.push({ cx: b.pos[0], cz: b.pos[2], n: 1 });
				}
			}
			// homes toward ~1 per PEOPLE_PER_HOUSE settlers, throttled by THIS chunk's span so the build-out is gradual
			// across the chunks rather than one dump; the per-cluster cap + new-town founding spread it.
			const targetHomes = Math.ceil(people / PEOPLE_PER_HOUSE);
			const deficit = Math.max(0, targetHomes - blds.length);
			let toAdd = Math.min(deficit, Math.round((cdt / 900) * (people / PEOPLE_PER_HOUSE)) + 1, 200 - blds.length, 50);
			let attempts = 0;
			const placeIn = (cl: Cluster): boolean => {
				// build BESIDE the cluster centroid (a ring jitter scaled to its size → blocks aligned on the 8 m grid)
				const ring = 10 + Math.sqrt(cl.n) * 8;
				const a = Math.random() * Math.PI * 2;
				const gx = Math.round((cl.cx + Math.cos(a) * ring * (0.5 + Math.random() * 0.5)) / 8) * 8;
				const gz = Math.round((cl.cz + Math.sin(a) * ring * (0.5 + Math.random() * 0.5)) / 8) * 8;
				if (world.objects.some((o) => BUILDING_KINDS.has(o.kind) && Math.abs(o.pos[0] - gx) < 6 && Math.abs(o.pos[2] - gz) < 6)) return false; // plot taken
				if (inWater(world.zones, gx, gz)) return false; // don't grow a home into a lake while you were away
				world.objects.push({ id: idPrefix + 'h' + hid++, kind: 'house', pos: [gx, groundY(gx, gz), gz] });
				cl.cx = (cl.cx * cl.n + gx) / (cl.n + 1);
				cl.cz = (cl.cz * cl.n + gz) / (cl.n + 1);
				cl.n++;
				houses++;
				return true;
			};
			// FOUND a NEW town anchor ≥FOUND_GAP from every existing town, on a golden-angle ring radiating outward from the
			// densest town's centroid. `founded` persists ACROSS chunks so successive new towns keep ringing out evenly.
			const foundCluster = (): Cluster | null => {
				const seed = clusters.reduce((best, c) => (c.n > best.n ? c : best), clusters[0]); // ring out from the biggest town
				for (let r = FOUND_GAP * 1.1; r <= FOUND_GAP * 2.4; r += FOUND_GAP * 0.4) {
					const a = founded * GOLDEN;
					const ax = Math.round((seed.cx + Math.cos(a) * r) / 8) * 8;
					const az = Math.round((seed.cz + Math.sin(a) * r) / 8) * 8;
					if (inWater(world.zones, ax, az)) continue;
					if (clusters.some((cl) => (cl.cx - ax) ** 2 + (cl.cz - az) ** 2 < FOUND_GAP * FOUND_GAP)) continue; // too close to an existing town
					founded++;
					const nc: Cluster = { cx: ax, cz: az, n: 0 };
					clusters.push(nc);
					return nc;
				}
				return null;
			};
			while (toAdd > 0 && attempts < 600) {
				attempts++;
				// prefer the smallest UNDER-cap cluster (fill towns evenly); if every town is full, FOUND a new one and build there.
				let into: Cluster | null = clusters.filter((cl) => cl.n < PER_CLUSTER_HOUSE_CAP).sort((a, b) => a.n - b.n)[0] ?? null;
				if (!into) {
					into = foundCluster();
					if (!into) break; // nowhere dry to found → stop (don't spin)
				}
				if (placeIn(into)) toAdd--;
			}
		}
	}

	// REFIT THE WALLS after away-growth — new homes change the settlement footprint, so the perimeter must be re-fitted
	// or you return to a STALE fence: a home standing ON an old panel, and the wall not reaching the new edge (user:
	// "one house stands on a fence, one side isn't closed"). Same engine the live sim uses (settlement_ops), an
	// idempotent position-diff: it removes panels a new home overran / that the grown wall no longer needs and adds the
	// new perimeter. Skipped if the wasm math isn't up yet (the Scene's on-load fit then catches it).
	if (houses > 0) {
		// BINARY settlement (jzon-free), via the STATELESS refit — a throwaway store, so it never clobbers the live
		// renderer's persistent fence store. `idBySlot` maps a REMOVE's store slot back to the object id we packed there.
		const idBySlot: string[] = [];
		const ops = math.settlementOpsBin(packStructures(world.objects, idBySlot), packWaterZones(world.zones, (id) => math.waterSeed(id) ?? 0));
		if (ops) {
			let fn = 0;
			for (let i = 0; i + OP_STRIDE <= ops.length; i += OP_STRIDE) {
				if (ops[i] === OP_ADD) {
					const x = ops[i + 2];
					const z = ops[i + 3];
					world.objects.push({ id: idPrefix + 'fc' + fn++, kind: kindStr(ops[i + 1]), pos: [x, groundY(x, z), z], rot: ops[i + 4], scale: [ops[i + 5], ops[i + 6], ops[i + 7]] });
				} else {
					const id = idBySlot[ops[i + 1]];
					if (id !== undefined) {
						const idx = world.objects.findIndex((o) => o.id === id);
						if (idx >= 0) world.objects.splice(idx, 1);
					}
				}
			}
		}
	}

	// GRAVES while away — some of the dead are remembered. A few headstones near the settlement, time-proportional.
	// Recompute the (now grown) homes + people after the development spiral — they were chunk-local inside the loop.
	const gBlds = world.objects.filter((o) => BUILDING_KINDS.has(o.kind));
	const gPeople = world.objects.reduce((s, o) => s + (o.kind === 'person' ? 1 : 0), 0);
	if (gBlds.length >= 2 && gPeople >= 4) {
		const existingGraves = world.objects.reduce((s, o) => s + (o.kind === 'grave' ? 1 : 0), 0);
		let toAdd = Math.min(Math.round(dt / 1200), 14 - existingGraves, 6); // ≤6 per jump, ≤14 total (matches GRAVE_CAP — a small cemetery, not a 70-stone pile)
		const cx = gBlds.reduce((s, b) => s + b.pos[0], 0) / gBlds.length;
		const cz = gBlds.reduce((s, b) => s + b.pos[2], 0) / gBlds.length;
		for (let g = 0; toAdd > 0; g++, toAdd--) {
			const a = Math.random() * Math.PI * 2;
			const r = 8 + Math.random() * 22; // a graveyard on the edge of town
			const gx = cx + Math.cos(a) * r;
			const gz = cz + Math.sin(a) * r;
			world.objects.push({ id: idPrefix + 'g' + g, kind: 'grave', pos: [gx, groundY(gx, gz), gz], rot: Math.random() * Math.PI * 2 });
		}
	}

	return { creatures, houses };
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
