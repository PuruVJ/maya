// REGION STREAMING (big-world.md §3) — the near/far sim split that lets the world be huge AND fast. Only regions
// NEAR the player hold INDIVIDUAL creatures (fully simulated by the Rust sim); far regions collapse to a cheap
// aggregate {counts, gene, lastTick} that costs nothing per tick. Crossing a region boundary SLEEPS the regions you
// left (creatures → counts) and WAKES the ones you enter (fast-forward the aggregate via the Rust closed-form
// `ff_targets`, then re-materialise individuals at seeded spots). Static structures (houses/trees/…) are the durable
// delta and are NEVER collapsed — only living creatures stream.
//
// Pure functions over the World (mutate objects + regions); Scene calls `streamRegions` whenever the player crosses
// a region cell. Determinism: positions come from the seeded hash RNG so the same region re-materialises consistently.
import type { World, WorldObject, RegionAggregate } from './world';
import { liveSettlementCount } from './world';
import { math } from './math';
import { heightAt } from './terrain';
import { kindDef } from './kinds';
import { rand } from './rng';
import { packWaterZones, kindStr } from './structpack';

const CREATURE_KINDS = new Set(['rabbit', 'cat', 'kangaroo', 'person', 'lion', 'dinosaur']);
const FF_KINDS = ['rabbit', 'cat', 'kangaroo', 'person', 'lion', 'dinosaur'] as const;
// The BUILDING kinds — the structures that make a region a SETTLEMENT (and so lift its carrying capacity + house its
// people). Mirrors world.ts's BUILDING_KINDS (kept local so streaming has no import cycle). Trees/fences/graves DON'T
// count — only homes develop land.
const BUILDING_KINDS = new Set(['house', 'cabin', 'tower']);
// A region needs at least this many homes to grow + keep a HOUSED population. Below it the region is WILD: its people
// are nomads, clamped to a tiny count (no wild-land person growth, no re-seed). At/above it the region is a settlement
// and its people grow toward the house-built cap. (Other kinds — rabbits etc. — grow on the area scale everywhere:
// wild land SHOULD have wildlife.)
const SETTLEMENT_MIN = 2;
const NOMAD_CAP = 3; // a sub-settlement (wild) region holds at most this many wandering people
// DORMANT SETTLEMENT DEVELOPMENT (self-sustaining world): a far settlement keeps BUILDING over time, not just relaxing
// its population toward a FIXED seeded house count — so EVERY town develops into a city while you're away, not only the
// one you're standing in (user: "only the main settlement grows; make all settlements grow"). Houses track population
// (~PEOPLE_PER_HOUSE settlers per home), bounded by COLONY_HOUSE_CAP (= the live colony cap, so a dormant town stops at
// the same city size — no runaway), grown a few per pulse so the horizon glow thickens gradually, not in one bloom.
const PEOPLE_PER_HOUSE = 2.8; // equilibrium settlers per home (≈ cap_for(Person, 30 houses)/30) → houses+people converge to the cap
const COLONY_HOUSE_CAP = 12; // a dormant colony stops building here (matches the live colony_cap); SMALL towns → more of them, more spread
const GROW_HOUSES_PER_PULSE = 6; // cap on new homes per FF pulse → gradual growth + one cheap wgGrowDormant call
// DORMANT SPREAD — the far world doesn't just FATTEN, it SPREADS. A FULL dormant settlement (at the colony cap) peels
// founders into a NEW satellite town FOUND_GAP away, so an absence grows fresh COLONIES, not one ever-capped blob
// (user: "if I'm not moving, only one settlement grows + no new colonies"). Mirrors the live pioneer spread.
const FOUND_GAP = 160; // min town spacing (matches Rust world::FOUND_GAP) → a satellite lands ≥ this from its parent (LOWERED → towns sit close enough to SEE; user "reduce distance between settlements")
const SPREAD_POP_MIN = 30; // a town spreads once this populous (a full small 12-house town ≈ 56 people still spreads)
const MAX_SETTLEMENTS = 48; // HARD CAP on the GLOBAL town count (dormant regions + live slice). Without it the satellite founding is EXPONENTIAL (895 towns / 50 k people, user "damnnn"). Past this, existing towns GROW but no NEW ones found → CONVERGES, populous but FINITE. MUST match world::MAX_CLUSTERS
const SPREAD_FOUNDERS = 10; // people peeled into each new satellite (its founding stock; grows via FF afterwards)
const GOLDEN = 2.399963229728653; // golden angle → successive satellites ring OUT evenly around the parent

/** Is there a BUILDING within ~1 region of cell (cx,cz)? Scans the LIVE objects AND every dormant aggregate's statics.
 *  Tells a SETTLEMENT's offloaded people (homes live/dormant nearby → CONSERVE them) apart from wild-land overshoot (no
 *  homes near → clamp to nomads). WHY THIS EXISTS: enforceLiveBudget offloads creatures + structures on SEPARATE
 *  budgets, so a busy settlement's PEOPLE can be evicted to a dormant aggregate while its HOUSES stay LIVE — that
 *  aggregate then reads builds=0 and the old NOMAD clamp DELETED the people a few at a time as the player moved ("the
 *  world started with 30+ people and they all died — plucked off"). Guarding the clamp with this check conserves them. */
function settlementNear(world: World, cx: number, cz: number): boolean {
	for (const o of world.objects) {
		if (!BUILDING_KINDS.has(o.kind)) continue;
		const [ox, oz] = regionOf(o.pos[0], o.pos[2]);
		if (Math.abs(ox - cx) <= 1 && Math.abs(oz - cz) <= 1) return true; // a live home in this or an adjacent region
	}
	if (world.regions) {
		for (const k in world.regions) {
			const [rx, rz] = k.split(',').map(Number);
			if (Math.abs(rx - cx) <= 1 && Math.abs(rz - cz) <= 1 && regionBuildCount(world.regions[k]) > 0) return true; // a dormant home nearby
		}
	}
	return false;
}

/** Count the BUILDING structures a dormant region's aggregate holds (its houses → its development level). */
function regionBuildCount(agg: { statics?: WorldObject[] }): number {
	let n = 0;
	for (const s of agg.statics ?? []) if (BUILDING_KINDS.has(s.kind)) n++;
	return n;
}

export const REGION_SIZE = 200; // metres per region tile
const ACTIVE_RING = 1; // regions within this Chebyshev ring of the player's stay LIVE (3×3 → ~600 m live span)
// HARD LIVE BUDGET — the most individual objects (creatures AND structures) that may be live at once. Region
// streaming bounds the live SPAN (~600 m); this bounds the live COUNT, for the case the span is densely packed
// (e.g. you spawn 1000 people in one spot). Excess beyond this — the FARTHEST from the player — is offloaded to the
// dormant region aggregates (still alive there, fast-forwarded, re-materialised as you approach). So only ~this many
// elements ever actually exist live around you, which is what caps the sim + draw cost (and the DRS flicker).
// The budget is SPLIT by class — creatures and static structures have SEPARATE caps and never compete for slots.
// WHY: a single shared pool let a developed town's own structures (its homes + ~60 fence panels + a pile of
// gravestones) fill the nearest-N slots, so distant WILDLIFE got evicted into dormant aggregates to make room — the
// wild emptied and every creature collapsed INTO the settlement (user: "everything's in the settlement, not
// spreading"). With independent budgets the town's structures stay drawn AND the same creatures keep their full
// ~600 m live span, so the world reads populated again. Neither cap usually bites (a 600 m span rarely holds this
// many of either) — they're the ceiling that bounds sim+draw cost when a span is genuinely overcrowded.
export const CREATURE_BUDGET = 240; // live ANIMATED creatures (sim ticks + skinned meshes = the real cost)
export const STRUCT_BUDGET = 250; // live STATIC structures (houses/fences/graves/trees/…), counted independently — STRICT cap (user: "strict budget only, 250") that bounds the binary-SoA live slice so render+sim never see more than this at once
export const LIVE_BUDGET = CREATURE_BUDGET; // back-compat alias (the creature cap is what callers gate the sweep on)
// tick rate comes from the sim clock (math.tickHz(), cached) — no duplicated 30 here

export const regionKey = (cx: number, cz: number): string => `${cx},${cz}`;
export const regionOf = (x: number, z: number): [number, number] => [Math.floor(x / REGION_SIZE), Math.floor(z / REGION_SIZE)];

/** Region keys that should be LIVE for a player at (px,pz) — its cell + the ACTIVE_RING around it. */
export function activeKeys(px: number, pz: number): Set<string> {
	const [pcx, pcz] = regionOf(px, pz);
	const set = new Set<string>();
	for (let dx = -ACTIVE_RING; dx <= ACTIVE_RING; dx++) for (let dz = -ACTIVE_RING; dz <= ACTIVE_RING; dz++) set.add(regionKey(pcx + dx, pcz + dz));
	return set;
}

/** SLEEP a region: tally its live creatures into the aggregate + drop those creature objects. Static objects stay. */
export function collapseRegion(world: World, key: string, tick: number): void {
	if (!world.regions) world.regions = {};
	// If the player crosses OUT of a HALF-WOKEN region, abandon its in-progress wake. NO LOSS: the not-yet-materialised
	// creatures are still counted in `agg.counts` (the pending pool), and the merge below folds the already-materialised
	// (live) creatures back in — so pending + materialised = the original FF'd target, exactly. Just drop the cursor.
	waking.delete(key);
	const counts: Record<string, number> = {};
	let geneSum = 0;
	let geneN = 0;
	const statics: WorldObject[] = [];
	const keep: WorldObject[] = [];
	for (const o of world.objects) {
		const [cx, cz] = regionOf(o.pos[0], o.pos[2]);
		if (regionKey(cx, cz) !== key) {
			keep.push(o); // not this region → stays live
		} else if (CREATURE_KINDS.has(o.kind)) {
			counts[o.kind] = (counts[o.kind] ?? 0) + 1; // creature → lossy aggregate
			geneSum += o.gene ?? 1;
			geneN++;
		} else {
			statics.push(o); // STATIC structure → kept verbatim in the aggregate (durable), dropped from live objects
		}
	}
	if (geneN === 0 && statics.length === 0) return; // empty region → nothing to collapse
	world.objects = keep;
	const prev = world.regions[key]; // merge if it already had an aggregate (slept twice without waking)
	const merged: Record<string, number> = { ...(prev?.counts ?? {}) };
	for (const k in counts) merged[k] = (merged[k] ?? 0) + counts[k];
	world.regions[key] = {
		counts: merged,
		gene: math.clampGene(geneN > 0 ? geneSum / geneN : (prev?.gene ?? 1)),
		statics: [...(prev?.statics ?? []), ...statics],
		lastTick: tick
	};
}

// Monotonic id counter for materialised creatures — GUARANTEES globally-unique ids. (The old `${prefix}${made}` was
// only unique within one wake; but enforceLiveBudget can evict creatures back into a region's aggregate, and that
// region may then be re-woken AT THE SAME TICK, so the tick-derived prefix collided → duplicate keyed-each keys →
// `each_key_duplicate` crash on reload. A process-wide counter can never collide.)
let materializeSeq = 0;
// Monotonic id counter for dormant-grown homes (same rationale as materializeSeq — globally-unique keyed-each ids).
let dormantHouseSeq = 0;
let dormantTownSeq = 0; // unique ids for satellite-town starter homes (dormant spread)

// IN-PROGRESS WAKE STATE — transient, MODULE-LEVEL (never serialized onto the saved aggregate). A WAKE-STORM used to
// materialise a dormant region's ENTIRE population into world.objects in ONE frame the instant the player crossed in —
// a visible jitter + "suddenly full of animals". The wake is now INCREMENTAL: `wakeRegion` does the one-time SETUP on
// first call (statics + fast-forward), then materialises up to `batch` creatures per call; this Map remembers, per
// region, the per-kind next seed index so successive calls resume where they left off. Keyed by region key. It holds
// ONLY the resume cursor — the PENDING (not-yet-materialised) count lives in `agg.counts`, which is decremented as we
// materialise, so the aggregate alone is enough to conserve the population if the player crosses back out mid-wake.
const waking = new Map<string, { next: Record<string, number> }>();

/** WAKE a region INCREMENTALLY: on the FIRST call, do the one-time setup — restore the durable statics + fast-forward
 *  the dormant counts/gene toward carrying capacity (Rust closed-form) and write them back onto the aggregate. On
 *  EVERY call (including the first), materialise up to `batch` creatures at seeded spots, decrementing `agg.counts`
 *  (the remaining/PENDING count) as it goes. Clears the aggregate + the waking cursor once every kind is exhausted.
 *  Returns how many it materialised THIS call. No-op (returns 0) if there's no aggregate. `batch = Infinity` (the
 *  default) materialises everything in one call → reproduces the old all-at-once behaviour for non-streaming callers. */
export function wakeRegion(world: World, key: string, tick: number, idPrefix: string, batch = Infinity): number {
	const agg = world.regions?.[key];
	if (!agg) return 0;
	const [cx, cz] = key.split(',').map(Number);
	// ONE-TIME SETUP (first call for this region only): fast-forward + restore statics. After this, agg.counts holds
	// the FF'd targets as the pending pool we drain from, agg.gene holds the evolved vigor, and the statics are live.
	if (!waking.has(key)) {
		const dtSec = Math.max(0, (tick - agg.lastTick) / math.tickHz());
		const c = agg.counts;
		// scale off THIS region's OWN houses (same tie as fastForwardDormant) — a region grows to what its development
		// supports, not the global near-builds count, so a region wakes BALANCED (people ≈ its houses), not at the town's.
		const builds = regionBuildCount(agg);
		const scale = math.worldAreaScale(builds);
		// fast-forward the dormant population toward carrying capacity (Rust). Wasm not loaded / no time → keep the counts.
		const adv = dtSec > 0 ? math.ffTargets(c.rabbit ?? 0, c.cat ?? 0, c.kangaroo ?? 0, c.person ?? 0, c.lion ?? 0, c.dinosaur ?? 0, scale, dtSec) : null;
		const final: Record<string, number> = {};
		if (adv) FF_KINDS.forEach((k, i) => (final[k] = adv[i]));
		else for (const k in c) final[k] = c[k];
		// PEOPLE need HOUSES — an unhoused (wild) region wakes with only its nomads, never a re-seeded crowd (matches
		// fastForwardDormant, so a region's people are consistent whether it's pulsed dormant or woken).
		if (builds < SETTLEMENT_MIN) final.person = Math.min(c.person ?? 0, NOMAD_CAP);
		// EVOLVE the dormant region's vigor over the away span (Rust closed-form) — dormant regions evolve via the clock,
		// they don't freeze. Under predation the mean gene climbs; with no predators it holds.
		agg.gene = dtSec > 0 ? math.ffGene(agg.gene, c, dtSec) : agg.gene;
		agg.counts = final; // FF'd targets become the PENDING pool — decremented as we materialise below (this conserves)
		// restore the durable STATIC structures (exact ids/x,z — the persistent delta), but RE-GROUND each to the live
		// terrain on wake, exactly like the materialised creatures below. A structure's saved Y can be stale (placed
		// against a different terrain state, or an upstream path that didn't ground it) → it would float/sink when the
		// region re-materialises on approach (user: "got close, the houses + wells are all up in the air"). Re-grounding is
		// idempotent when the Y was already right. `?? []` tolerates aggregates persisted before the statics field existed.
		for (const s of agg.statics ?? []) {
			if (s.kind === 'fence') continue; // perimeter fences were ripped out — never restore one stored in an old aggregate
			world.objects.push(s.kind === 'bridge' ? s : { ...s, pos: [s.pos[0], heightAt(s.pos[0], s.pos[2], world.terrain), s.pos[2]] }); // bridges span gaps → keep their exact Y
		}
		agg.statics = []; // statics are now live — drop them from the aggregate so a mid-wake collapse doesn't re-add them
		waking.set(key, { next: {} });
	}
	const w = waking.get(key)!;
	const gene = agg.gene;
	// MATERIALISE up to `batch` creatures, walking FF_KINDS. `agg.counts[kind]` is the PENDING count: each creature we
	// push decrements it (so pending + already-materialised = the FF'd target, always), and `w.next[kind]` is the seed
	// index so the deterministic seeded position is the SAME whether we wake in one shot or dribble it over many frames.
	let made = 0;
	for (const kind of FF_KINDS) {
		while ((agg.counts[kind] ?? 0) > 0 && made < batch) {
			const i = w.next[kind] ?? 0;
			const sx = rand(cx * 73856093 + cz * 19349663, kind.charCodeAt(0), i, 1); // seeded → deterministic re-materialise
			const sz = rand(cx * 19349663 + cz * 83492791, kind.charCodeAt(0), i, 2);
			const x = (cx + sx) * REGION_SIZE;
			const z = (cz + sz) * REGION_SIZE;
			world.objects.push({ id: `${idPrefix}${(materializeSeq++).toString(36)}`, kind, pos: [x, heightAt(x, z, world.terrain), z], gene, scale: [1, 1, 1] });
			agg.counts[kind]--; // one fewer pending
			w.next[kind] = i + 1;
			made++;
		}
		if (made >= batch) break;
	}
	// FULLY WOKEN when no kind has any pending left → clear the aggregate + the cursor.
	let pending = 0;
	for (const kind of FF_KINDS) pending += agg.counts[kind] ?? 0;
	if (pending <= 0) {
		if (world.regions) delete world.regions[key];
		waking.delete(key);
	}
	return made;
}

/** DORMANT IMPOSTORS — visit each creature a dormant (NOT mid-wake) region WOULD materialise, AT THE EXACT (x,z)
 *  `wakeRegion` will give it, so a cheap impostor silhouette can be drawn there and the wake becomes a smooth
 *  impostor→creature swap at the same spot. SINGLE SOURCE OF TRUTH for the seed formula: the (sx,sz)→(x,z) math
 *  here is byte-identical to the materialise loop in `wakeRegion` (same seeds, same per-creature index `i`), so the
 *  impostor and the creature it becomes coincide. Skips `waking.has(key)` regions: their `agg.counts` is the PENDING
 *  remainder being drained live (the near creatures the drainer is materialising), so drawing them would double up —
 *  and that's the one region the player is entering, where the focus is the near creatures anyway. */
export function forEachDormantImpostor(world: World, cb: (x: number, z: number, kind: string) => void): void {
	if (!world.regions) return;
	for (const key in world.regions) {
		if (waking.has(key)) continue; // mid-wake → being materialised live; drawing impostors too would double up
		const [cx, cz] = key.split(',').map(Number);
		const agg = world.regions[key];
		for (const kind of FF_KINDS) {
			const n = Math.round(agg.counts[kind] ?? 0);
			if (!(n > 0)) continue; // guards NaN / negative / 0 (NaN > 0 is false)
			for (let i = 0; i < n; i++) {
				const sx = rand(cx * 73856093 + cz * 19349663, kind.charCodeAt(0), i, 1); // IDENTICAL to wakeRegion
				const sz = rand(cx * 19349663 + cz * 83492791, kind.charCodeAt(0), i, 2);
				cb((cx + sx) * REGION_SIZE, (cz + sz) * REGION_SIZE, kind);
			}
		}
	}
}

/** A cheap fold of the NON-waking dormant set (region keys + their per-kind counts) → one number the impostor
 *  renderer can poll to detect when the dormant population actually changed (a collapse, a world-pulse FF, a
 *  wake start/finish) and rebuild ONLY then — never per frame. Mirrors the structure-fingerprint pattern in Scene. */
export function dormantImpostorSignature(world: World): number {
	if (!world.regions) return 0;
	let sig = 0;
	for (const key in world.regions) {
		if (waking.has(key)) continue; // same set forEachDormantImpostor walks → the signature tracks exactly what's drawn
		let kh = 0;
		for (let i = 0; i < key.length; i++) kh = (Math.imul(kh, 31) + key.charCodeAt(i)) | 0; // cheap string hash of the key
		const agg = world.regions[key];
		for (const kind of FF_KINDS) sig = (Math.imul(sig, 31) + kh + Math.round(agg.counts[kind] ?? 0)) | 0;
	}
	return sig;
}

/** Grow a dormant SETTLEMENT's homes toward what its (just-FF'd) population supports — the far-town DEVELOPMENT that
 *  makes EVERY settlement become a city over time, not only the live one you're standing in. Rust `grow_dormant` places
 *  the new homes water-safe + colony-capped (the same rules as a live settler), and they're appended to `agg.statics`
 *  so they glow on the horizon immediately (SettlementGlows reads statics) and materialise when the region wakes.
 *  Population FOLLOWS on the next pulse (more houses → higher cap_for → ffTargets grows it further), so houses+people
 *  co-develop toward the colony equilibrium. No-op once at the cap (growth stops at city size on its own). */
function growDormantSettlement(world: World, agg: RegionAggregate, builds: number, people: number, seed: number): void {
	const target = Math.min(COLONY_HOUSE_CAP, Math.floor(people / PEOPLE_PER_HOUSE));
	const want = Math.min(GROW_HOUSES_PER_PULSE, target - builds);
	if (want <= 0) return; // already developed enough for its population (or at the colony cap) → no new homes
	const houses: number[] = [];
	for (const s of agg.statics ?? []) if (BUILDING_KINDS.has(s.kind)) houses.push(s.pos[0], s.pos[2]);
	if (houses.length < 2) return; // need an existing cluster to grow around (centroid + footprint)
	const zones = packWaterZones(world.zones, (id) => math.waterSeed(id) ?? 0);
	const ops = math.wgGrowDormant(new Float64Array(houses), want, zones, seed);
	if (!ops) return; // wasm not loaded → grow next pulse
	for (let i = 0; i + 8 < ops.length; i += 9) {
		// [OP_ADD(0), kind, x, z, rot, sx, sy, sz, color]. Y=0 — wakeRegion regrounds on materialise; the glow uses heightAt.
		agg.statics.push({ id: `dh${(dormantHouseSeq++).toString(36)}`, kind: kindStr(ops[i + 1]), pos: [ops[i + 2], 0, ops[i + 3]], rot: ops[i + 4], scale: [ops[i + 5], ops[i + 6], ops[i + 7]] });
	}
}

/** Advance ONE dormant region by `dtSec` seconds: relax its populations toward the carrying capacity its OWN homes
 *  support (Rust ffTargets), evolve its vigor (ffGene), and DEVELOP its settlement (build homes → raise capacity).
 *  Shared by the live world-pulse (dt from the sim clock) AND the load-time away-catch-up (dt from real wall-clock).
 *  Does NOT touch agg.lastTick — the caller owns that. */
function advanceDormant(world: World, key: string, agg: RegionAggregate, dtSec: number): void {
	if (dtSec <= 0) return;
	const c = agg.counts;
	// Each region grows on the carrying capacity its OWN development supports — scale off THIS region's houses, not a
	// global near-builds count. So wild land (no homes) stays at the baseline (rabbits etc. still grow), while a settled
	// region's capacity rises with its homes → people ≈ houses, not every roamed region at the near town's scale.
	const builds = regionBuildCount(agg);
	const scale = math.worldAreaScale(builds);
	const adv = math.ffTargets(c.rabbit ?? 0, c.cat ?? 0, c.kangaroo ?? 0, c.person ?? 0, c.lion ?? 0, c.dinosaur ?? 0, scale, dtSec);
	if (!adv) return; // wasm not loaded → leave the region untouched
	const next: Record<string, number> = {};
	FF_KINDS.forEach((k, i) => (next[k] = adv[i]));
	// PEOPLE need HOUSES — but only clamp to nomads on TRULY wild land (no homes near). A settlement whose people were
	// offloaded here while its houses stayed LIVE (separate budgets) must be CONSERVED, not deleted (the "30+ people all
	// died" bug); settlementNear sees those live/dormant homes and skips the clamp. (The FF re-seeds people everywhere;
	// the clamp is what stopped that re-seed from growing a houseless-land crowd — the old 1100 blob.)
	const [fx, fz] = key.split(',').map(Number);
	if (builds < SETTLEMENT_MIN && !settlementNear(world, fx, fz)) next.person = Math.min(c.person ?? 0, NOMAD_CAP);
	// evolve vigor over the span (BEFORE overwriting counts), clamped defensively to the gene band.
	agg.gene = math.clampGene(math.ffGene(agg.gene, c, dtSec));
	agg.counts = next;
	// FAR-TOWN DEVELOPMENT: a settled region keeps BUILDING as its population grows → every town becomes a city over
	// time, not just the live one you're in. Seeded by the region key so it's deterministic. The new homes raise `builds`
	// next call → ffTargets grows the population further (the co-development loop).
	if (builds >= SETTLEMENT_MIN) growDormantSettlement(world, agg, builds, next.person, Math.abs(Math.sin(key.charCodeAt(0) * 12.9898 + key.length * 78.233)));
}

/** WORLD PULSE (user) — fast-forward EVERY dormant region's aggregate to `tick` WITHOUT waking it, so the far
 *  world keeps LIVING (populations relax toward carrying capacity + vigor evolves AND settlements keep BUILDING)
 *  instead of freezing until you visit. Pure closed-form (Rust ff_targets/ff_gene + grow_dormant), O(1) per region →
 *  microseconds for dozens of regions, and it runs on the main thread between frames so it never blocks the worker's
 *  sim ticks. Scene calls it ~every 10 s. */
export function fastForwardDormant(world: World, tick: number): void {
	if (!world.regions) return;
	for (const key in world.regions) {
		if (waking.has(key)) continue; // mid-wake: agg.counts is the PENDING remainder, not the full dormant count — never FF that
		const agg = world.regions[key];
		advanceDormant(world, key, agg, (tick - agg.lastTick) / math.tickHz());
		agg.lastTick = tick;
	}
}

/** LOAD-TIME AWAY CATCH-UP for the DORMANT far world. The world-pulse advances by SIM ticks, which FREEZE while the
 *  app is closed — so on return the far settlements would sit frozen at their saved counts (user: "came back hours
 *  later, the world was stuck at similar numbers"). This advances EVERY dormant region by the real wall-clock `awayMs`
 *  (the same per-region develop logic), so far towns + wildlife catch up to "now" exactly as the live slice does via
 *  world.ts `fastForward`. Leaves agg.lastTick alone — the live pulses resume from the saved tick at ~0 elapsed, so the
 *  away span isn't double-counted. Call once on load, after the live-slice fastForward. */
export function fastForwardDormantAway(world: World, awayMs: number): void {
	if (!world.regions) return;
	const dtSec = Math.min(awayMs / 1000, 86_400); // cap at ~1 day (the logistic saturates anyway) — matches world.ts fastForward
	if (dtSec < 30) return; // a blink away → nothing to do
	// CHUNK the away span (like world.ts fastForward) so the co-development spiral runs: each chunk relaxes population +
	// builds a few homes (growDormantSettlement caps homes/call), and the risen home count lifts NEXT chunk's capacity.
	// A single call would only add ~6 homes to a dormant town; chunking lets a far town develop into a city on return.
	const CHUNKS = 6;
	const cdt = dtSec / CHUNKS;
	for (let chunk = 0; chunk < CHUNKS; chunk++) {
		for (const key in world.regions) {
			if (waking.has(key)) continue;
			advanceDormant(world, key, world.regions[key], cdt);
		}
		spreadDormantSettlements(world, chunk); // FULL towns peel founders into NEW satellites → the far world SPREADS
	}
}

/** DORMANT SPREAD — a FULL, populous dormant settlement founds a SATELLITE town FOUND_GAP away (peeling founders + a
 *  couple of starter homes into a fresh/empty region), so the far world keeps SPREADING into new colonies while you're
 *  away, not just fattening one capped blob. Seeded by (region key, chunk) → deterministic, successive satellites ring
 *  out evenly; skips a target region that's already a town (no overlap, no re-founding the same spot). The satellite
 *  then DEVELOPS on later chunks via advanceDormant (its founders relax toward its own carrying capacity + it builds). */
function spreadDormantSettlements(world: World, chunk: number): void {
	if (!world.regions) return;
	// HARD CAP: stop founding NEW satellites once the world already has MAX_SETTLEMENTS towns. Without this the founding
	// compounds — each full town spawns a satellite that itself fills + spawns more → EXPONENTIAL (895 towns / 50 k people).
	// Past the cap the existing towns keep growing toward carrying capacity, so the world stays populous but FINITE.
	let settled = 0;
	for (const k in world.regions) if (regionBuildCount(world.regions[k]) >= SETTLEMENT_MIN) settled++;
	// count the LIVE slice too — the live + dormant towns share ONE global cap, otherwise the live founder (world.ts)
	// and this dormant spreader each found up to MAX independently and the world overshoots (user: "3×+1d → 84 towns").
	if (settled + liveSettlementCount(world.objects) >= MAX_SETTLEMENTS) return;
	for (const key of Object.keys(world.regions)) {
		// snapshot the keys above — never spread FROM a satellite created this same pass (would chain-found in one go)
		if (waking.has(key)) continue;
		const agg = world.regions[key];
		const people = agg.counts.person ?? 0;
		if (regionBuildCount(agg) < COLONY_HOUSE_CAP || people < SPREAD_POP_MIN) continue; // only a FULL, populous town spreads
		// centroid of the parent's homes
		let cx = 0;
		let cz = 0;
		let nh = 0;
		for (const s of agg.statics ?? []) if (BUILDING_KINDS.has(s.kind)) ((cx += s.pos[0]), (cz += s.pos[2]), nh++);
		if (nh === 0) continue;
		cx /= nh;
		cz /= nh;
		// satellite site: a golden-angle ring ≥ FOUND_GAP out, seeded by (key hash, chunk) → deterministic + rings out
		let kh = 0;
		for (let i = 0; i < key.length; i++) kh = (Math.imul(kh, 31) + key.charCodeAt(i)) | 0;
		const ang = (Math.abs(kh % 1000) / 1000) * Math.PI * 2 + chunk * GOLDEN;
		const sx = cx + Math.cos(ang) * FOUND_GAP * 1.3;
		const sz = cz + Math.sin(ang) * FOUND_GAP * 1.3;
		const skey = regionKey(...regionOf(sx, sz));
		if (skey === key) continue; // landed in the parent's own region → no spread
		if (world.regions[skey] && regionBuildCount(world.regions[skey]) >= SETTLEMENT_MIN) continue; // already a town there
		// PEEL founders from the parent into the satellite (conserves population) + 2 starter homes → it's a settlement
		agg.counts.person = people - SPREAD_FOUNDERS;
		const sat = (world.regions[skey] ??= { counts: {}, gene: agg.gene, statics: [], lastTick: agg.lastTick });
		sat.counts.person = (sat.counts.person ?? 0) + SPREAD_FOUNDERS;
		if (regionBuildCount(sat) < SETTLEMENT_MIN) {
			for (let h = 0; h < 2; h++) sat.statics.push({ id: `ds${(dormantTownSeq++).toString(36)}`, kind: 'house', pos: [sx + h * 8, 0, sz], keep: true } as WorldObject);
		}
	}
}

/** ONE-TIME OVERSHOOT TRIM (load-time). The per-region FF tie (fastForwardDormant / wakeRegion) stops FUTURE growth
 *  from outrunning a region's carrying capacity, but a world saved BEFORE a tuning change has the old overshoot already
 *  BANKED in the dormant aggregates — and the FF relaxes it DOWN only slowly (rate ~0.0016/s), so on reload you'd still
 *  see the old crammed world for minutes. This snaps each dormant region's counts to that region's CURRENT carrying
 *  capacity immediately (Rust `pop_caps`/`cap_for`, single source of truth), so the world reads right the instant you
 *  load it, not gradually as regions are re-roamed. Two banked overshoots get corrected:
 *    • PEOPLE — the old global ~38·scale grown into EVERY roamed region (the 1100-in-3-towns blob): a region with no
 *      homes drops to the nomad count, a settled region holds at its house-built cap. (Reported → the welcome-back line.)
 *    • WILDLIFE — the old denser prey base (rabbits/kangaroos, and the cats/lions/dino that share off them): clamped to
 *      the current density caps so the near vicinity loads CALM, not at the stale ~110-per-tile pack. (Silent — a
 *      "12000 animals vanished" toast would just alarm; the FF would have relaxed them down anyway, only slower.)
 *  Predators clamp off the ALREADY-CLAMPED prey (trophic order) so the pyramid stays consistent. ONLY trims down (never
 *  adds); idempotent → a re-saved world stays balanced. Returns how many PEOPLE it trimmed (for the welcome-back readout). */
export function trimDormantOvershoot(world: World): number {
	if (!world.regions) return 0;
	let trimmed = 0;
	for (const key in world.regions) {
		const agg = world.regions[key];
		const c = agg.counts;
		if (!c) continue;
		const builds = regionBuildCount(agg);
		const scale = math.worldAreaScale(builds);
		// WILDLIFE (silent) — clamp to the current density caps. PREY caps (rabbit/kangaroo) are independent of other
		// counts, so clamp them first; PREDATOR caps (cat/lion/dino) share off the prey, so recompute from the clamped
		// prey before clamping them. pop_caps order = [rabbit, cat, kangaroo, person, lion, dino].
		const preyCaps = math.popCaps(c.rabbit ?? 0, c.cat ?? 0, c.kangaroo ?? 0, c.person ?? 0, c.lion ?? 0, c.dinosaur ?? 0, scale);
		if (preyCaps) {
			c.rabbit = Math.min(c.rabbit ?? 0, preyCaps[0]);
			c.kangaroo = Math.min(c.kangaroo ?? 0, preyCaps[2]);
			const predCaps = math.popCaps(c.rabbit, c.cat ?? 0, c.kangaroo, c.person ?? 0, c.lion ?? 0, c.dinosaur ?? 0, scale);
			if (predCaps) {
				c.cat = Math.min(c.cat ?? 0, predCaps[1]);
				c.lion = Math.min(c.lion ?? 0, predCaps[4]);
				c.dinosaur = Math.min(c.dinosaur ?? 0, predCaps[5]);
			}
		}
		// PEOPLE (reported) — below the settlement threshold a WILD region clamps to the tiny nomad count (no housing →
		// no crowd); at/above, cap to the region's OWN house-built person carrying capacity (index 3 = Person). A houseless
		// region whose people belong to a NEARBY settlement (homes live/dormant adjacent) is treated as settled, so an
		// offloaded settlement isn't nomad-trimmed to nothing on load (the "30+ people died" bug, load-time variant).
		const have = Math.round(c.person ?? 0);
		if (have <= 0) continue;
		const [tx, tz] = key.split(',').map(Number);
		let cap: number;
		if (builds < SETTLEMENT_MIN && !settlementNear(world, tx, tz)) {
			cap = NOMAD_CAP;
		} else {
			const caps = math.popCaps(c.rabbit ?? 0, c.cat ?? 0, c.kangaroo ?? 0, have, c.lion ?? 0, c.dinosaur ?? 0, scale);
			cap = caps ? caps[3] : have; // wasm not loaded → don't guess, leave it
		}
		if (have > cap) {
			c.person = cap;
			trimmed += have - cap;
		}
	}
	return trimmed;
}

/** Per-cell streaming step (call when the player crosses a region): WAKE regions that just entered the active set,
 *  SLEEP regions with live creatures that just left it. Returns counts for diagnostics (0/0 if nothing changed).
 *  `wakeBatch` caps how many creatures each entering region materialises THIS call — pass 0 to do only the one-time
 *  SETUP (statics + fast-forward) on a crossing and leave the creatures for the per-frame `drainWakes` to dribble in,
 *  so a crossing never WAKE-STORMs. Default Infinity → fully wake in one shot (the old behaviour; what tests rely on). */
export function streamRegions(world: World, px: number, pz: number, tick: number, idPrefix = 'rg', wakeBatch = Infinity): { slept: number; woken: number } {
	const active = activeKeys(px, pz);
	let slept = 0;
	let woken = 0;
	if (world.regions) {
		for (const key of Object.keys(world.regions)) {
			if (active.has(key)) {
				wakeRegion(world, key, tick, `${idPrefix}${key.replace(',', '_')}-${tick.toString(36)}-`, wakeBatch);
				woken++;
			}
		}
	}
	const live = new Set<string>();
	for (const o of world.objects) {
		const [cx, cz] = regionOf(o.pos[0], o.pos[2]);
		const k = regionKey(cx, cz);
		if (!active.has(k)) live.add(k); // ANY object (creature OR static) in a non-active region → that region sleeps
	}
	for (const k of live) {
		collapseRegion(world, k, tick);
		slept++;
	}
	return { slept, woken };
}

/** PER-FRAME WAKE DRAINER — call every frame (NOT just on a crossing). For each ACTIVE region that still has a dormant
 *  aggregate (i.e. a crossing did its SETUP with wakeBatch 0, or a mid-wake region the player re-entered), materialise
 *  another `batch` creatures via the incremental `wakeRegion`. This spreads a region's population in over several
 *  frames (~12/frame) instead of all at once — the WAKE-STORM fix. Returns how many it materialised this frame (0 when
 *  every active region is already fully awake → the common, allocation-light case). */
export function drainWakes(world: World, px: number, pz: number, tick: number, idPrefix = 'rg', batch = 12): number {
	if (!world.regions) return 0;
	// RESPECT THE LIVE BUDGET. Without this, a dense/overpopulated world THRASHES: every frame enforceLiveBudget
	// offloads the farthest creatures and drainWakes instantly re-materialises them → an offload↔materialise LOOP
	// (200 ms frames; creatures visibly spawning under you each frame; "100 cats appear where I stand"). Only wake up
	// to the HEADROOM the budget will keep; the surplus stays dormant until a slot frees (a death), so the two passes
	// converge to a stable live set instead of fighting every frame.
	let live = 0;
	for (const o of world.objects) if (CREATURE_KINDS.has(o.kind)) live++;
	let made = 0;
	for (const key of activeKeys(px, pz)) {
		const headroom = CREATURE_BUDGET - live - made;
		if (headroom <= 0) break; // at budget → leave the rest dormant (no re-materialise → no loop)
		if (!world.regions[key]) continue; // no aggregate → already fully awake (or never dormant)
		made += wakeRegion(world, key, tick, `${idPrefix}${key.replace(',', '_')}-${tick.toString(36)}-`, Math.min(batch, headroom));
	}
	return made;
}

/** HARD LIVE-COUNT CAP. Keep only the nearest `budget` objects live (creatures + structures alike); offload the
 *  FARTHEST excess into their region's dormant aggregate — creatures lossily (counts + a count-weighted gene mean),
 *  statics verbatim — exactly as collapseRegion does, so they round-trip back when you approach. This is what makes
 *  "only ~budget elements actually exist live around you" true even when the live SPAN is densely packed (region
 *  streaming alone can't help if 1000 are crammed into one 200 m tile). Returns how many it offloaded. Cheap: a
 *  single distance sort, and the caller only invokes it while over budget. */
export function enforceLiveBudget(world: World, px: number, pz: number, tick: number, creatureBudget = CREATURE_BUDGET, structBudget = STRUCT_BUDGET): number {
	const objs = world.objects;
	// COUNT each class + early-out when both are under budget — the common case, and it ALLOCATES NOTHING (this runs
	// every frame, so the no-op path must stay free of garbage).
	let nCre = 0;
	let nStr = 0;
	for (let i = 0; i < objs.length; i++) CREATURE_KINDS.has(objs[i].kind) ? nCre++ : nStr++;
	if (nCre <= creatureBudget && nStr <= structBudget) return 0;
	if (!world.regions) world.regions = {};
	const creIdx: number[] = [];
	const strIdx: number[] = [];
	for (let i = 0; i < objs.length; i++) (CREATURE_KINDS.has(objs[i].kind) ? creIdx : strIdx).push(i);
	// rank EACH class by distance² INDEPENDENTLY and keep the nearest `budget` of each — so a structure-dense town can
	// never evict distant wildlife to make room for its own homes/fences/graves (that starvation collapsed the world's
	// creatures into the settlement). Emit `kept` in the ORIGINAL array order: world.objects feeds the keyed {#each},
	// so re-sorting it (nearest-first) made Svelte REORDER the DOM nodes (reconcile→move→insertBefore) every frame the
	// player moved — ~40% of the main thread during movement. Insertion order keeps the keyed list append/remove-only.
	const d2 = objs.map((o) => (o.pos[0] - px) ** 2 + (o.pos[2] - pz) ** 2);
	creIdx.sort((a, b) => d2[a] - d2[b]);
	strIdx.sort((a, b) => d2[a] - d2[b]);
	const keep = new Set([...creIdx.slice(0, creatureBudget), ...strIdx.slice(0, structBudget)]); // nearest of EACH class
	const kept: WorldObject[] = [];
	let evicted = 0;
	for (let i = 0; i < objs.length; i++) {
		const o = objs[i];
		if (keep.has(i)) {
			kept.push(o); // among the nearest `budget` → stays live, in ORIGINAL order (no keyed-each reorder)
			continue;
		}
		const [cx, cz] = regionOf(o.pos[0], o.pos[2]);
		const key = regionKey(cx, cz);
		const agg = (world.regions[key] ??= { counts: {}, gene: 1, statics: [], lastTick: tick });
		if (CREATURE_KINDS.has(o.kind)) {
			// count-weighted running gene mean (matches the spirit of collapseRegion's averaged gene)
			let n = 0;
			for (const k in agg.counts) n += agg.counts[k];
			agg.gene = math.clampGene((agg.gene * n + (o.gene ?? 1)) / (n + 1));
			agg.counts[o.kind] = (agg.counts[o.kind] ?? 0) + 1;
		} else {
			agg.statics.push(o); // durable structure → kept verbatim (restored on wake)
		}
		agg.lastTick = tick;
		evicted++;
	}
	world.objects = kept;
	return evicted;
}
