import { describe, it, expect, beforeAll } from 'vitest';
import { emptyWorld, type World, type WorldObject } from './world';
import { regionOf, regionKey, activeKeys, collapseRegion, wakeRegion, streamRegions, drainWakes, enforceLiveBudget, fastForwardDormant, fastForwardDormantAway, trimDormantOvershoot, REGION_SIZE } from './streaming';
import { heightAt } from './terrain';
import { math } from './math';

// Build a world with `n` creatures of `kind` clustered around a region's centre.
function withCreatures(kind: string, n: number, cx: number, cz: number, w: World = emptyWorld('t')): World {
	for (let i = 0; i < n; i++) {
		const x = cx * REGION_SIZE + (i % 7) + 1;
		const z = cz * REGION_SIZE + ((i * 3) % 11) + 1;
		w.objects.push({ id: `c${kind}${cx}_${cz}_${i}`, kind, pos: [x, 0, z] } as WorldObject);
	}
	return w;
}
const creatures = (w: World, kind?: string) => w.objects.filter((o) => (kind ? o.kind === kind : ['rabbit', 'cat', 'kangaroo', 'person', 'lion', 'dinosaur'].includes(o.kind)));

describe('region math', () => {
	it('maps positions to region cells', () => {
		expect(regionOf(0, 0)).toEqual([0, 0]);
		expect(regionOf(REGION_SIZE - 1, 1)).toEqual([0, 0]);
		expect(regionOf(REGION_SIZE + 1, -1)).toEqual([1, -1]);
		expect(regionKey(2, -3)).toBe('2,-3');
	});
	it('active set is the 3×3 around the player', () => {
		const a = activeKeys(10, 10); // region 0,0
		expect(a.size).toBe(9);
		expect(a.has('0,0')).toBe(true);
		expect(a.has('1,1')).toBe(true);
		expect(a.has('2,0')).toBe(false);
	});
});

describe('sleep / wake', () => {
	it('collapseRegion offloads BOTH creatures (aggregated) and statics (verbatim) out of live objects', () => {
		const w = withCreatures('rabbit', 30, 5, 0);
		w.objects.push({ id: 'h0', kind: 'house', pos: [5 * REGION_SIZE + 2, 0, 2], keep: true } as WorldObject);
		collapseRegion(w, '5,0', 100);
		expect(w.objects.length).toBe(0); // EVERYTHING in the region left live objects (creatures + the house)
		expect(w.regions?.['5,0']?.counts.rabbit).toBe(30); // rabbits → lossy aggregate
		expect(w.regions?.['5,0']?.statics.length).toBe(1); // house → kept verbatim in the aggregate
		expect(w.regions?.['5,0']?.statics[0].id).toBe('h0');
		expect(w.regions?.['5,0']?.lastTick).toBe(100);
	});

	it('a static structure round-trips through sleep→wake (same id + x/z), RE-GROUNDED to the terrain on wake', () => {
		const w = emptyWorld('t');
		w.objects.push({ id: 'h7', kind: 'tower', pos: [8 * REGION_SIZE + 5, 3, 9], keep: true } as WorldObject); // Y=3 is a stale/wrong height here
		streamRegions(w, 10, 10, 0); // player at origin → region 8,0 sleeps (the tower offloads)
		expect(w.objects.length).toBe(0);
		streamRegions(w, 8 * REGION_SIZE + 10, 10, 0); // walk there → wakes
		const tower = w.objects.find((o) => o.kind === 'tower');
		expect(tower?.id).toBe('h7'); // exact id preserved (durable)
		expect([tower?.pos[0], tower?.pos[2]]).toEqual([8 * REGION_SIZE + 5, 9]); // exact x/z preserved
		expect(tower?.pos[1]).toBeCloseTo(heightAt(8 * REGION_SIZE + 5, 9, w.terrain)); // Y re-grounded on wake (no float, even from a stale saved Y)
	});

	it('wakeRegion re-materialises the population (count conserved without fast-forward)', () => {
		const w = emptyWorld('t');
		w.regions = { '5,0': { counts: { rabbit: 18, lion: 2 }, gene: 1.1, statics: [], lastTick: 0 } };
		const made = wakeRegion(w, '5,0', 0, 'rg-'); // dt=0 → no fast-forward → exact counts
		expect(made).toBe(20);
		expect(creatures(w, 'rabbit').length).toBe(18);
		expect(creatures(w, 'lion').length).toBe(2);
		expect(w.regions?.['5,0']).toBeUndefined(); // aggregate cleared
		// materialised inside the region bounds + carrying the aggregate's gene
		for (const o of creatures(w)) {
			expect(regionKey(...regionOf(o.pos[0], o.pos[2]))).toBe('5,0');
			expect(o.gene).toBeCloseTo(1.1);
		}
	});

	it('streamRegions sleeps far creatures and keeps near ones', () => {
		let w = withCreatures('rabbit', 20, 0, 0); // near the player (region 0,0)
		w = withCreatures('kangaroo', 15, 6, 6, w); // far away (region 6,6)
		const r = streamRegions(w, 10, 10, 200); // player in region 0,0
		expect(r.slept).toBe(1); // the far region collapsed
		expect(creatures(w, 'rabbit').length).toBe(20); // near rabbits untouched
		expect(creatures(w, 'kangaroo').length).toBe(0); // far kangaroos collapsed
		expect(w.regions?.['6,6']?.counts.kangaroo).toBe(15);
	});

	it('enforceLiveBudget keeps the nearest `budget` live and offloads the farthest to dormant aggregates', () => {
		const w = emptyWorld('t');
		for (let i = 0; i < 100; i++) w.objects.push({ id: 'r' + i, kind: 'rabbit', pos: [i * 5, 0, 0], gene: 1 } as WorldObject);
		const evicted = enforceLiveBudget(w, 0, 0, 10, 40); // player at origin → nearest 40 (smallest x) stay
		expect(evicted).toBe(60);
		expect(w.objects.length).toBe(40);
		expect(Math.max(...w.objects.map((o) => o.pos[0]))).toBe(39 * 5); // the 40 kept are the 40 nearest
		let dormant = 0; // the 60 offloaded are still alive in region aggregates
		for (const k in w.regions) for (const kind in w.regions![k].counts) dormant += w.regions![k].counts[kind];
		expect(dormant).toBe(60);
	});

	it('enforceLiveBudget offloads STRUCTURES verbatim too (not only creatures)', () => {
		const w = emptyWorld('t');
		w.objects.push({ id: 'r0', kind: 'rabbit', pos: [1, 0, 1], gene: 1 } as WorldObject); // near
		w.objects.push({ id: 'h0', kind: 'house', pos: [9999, 0, 0] } as WorldObject); // far
		expect(enforceLiveBudget(w, 0, 0, 5, 1, 0)).toBe(1); // creatureBudget 1 keeps the rabbit; structBudget 0 evicts the house
		expect(w.objects.map((o) => o.id)).toEqual(['r0']);
		const key = regionKey(...regionOf(9999, 0));
		expect(w.regions?.[key]?.statics[0]?.id).toBe('h0'); // house preserved verbatim for round-trip
	});

	it('enforceLiveBudget budgets CREATURES and STRUCTURES independently — structures never evict wildlife', () => {
		// THE FIX: a structure-dense town must not push distant wildlife out of the live set (that collapsed every
		// creature into the settlement). 300 structures around the player + 50 far creatures, creatureBudget 240,
		// structBudget 100 → the structures cap themselves (200 evicted) but EVERY creature stays live (50 ≤ 240).
		const w = emptyWorld('t');
		for (let i = 0; i < 300; i++) w.objects.push({ id: 'g' + i, kind: 'grave', pos: [i % 10, 0, (i / 10) | 0] } as WorldObject); // dense, near
		for (let i = 0; i < 50; i++) w.objects.push({ id: 'r' + i, kind: 'rabbit', pos: [500 + i * 5, 0, 0], gene: 1 } as WorldObject); // far
		const evicted = enforceLiveBudget(w, 0, 0, 10, 240, 100);
		expect(evicted).toBe(200); // only the farthest 200 STRUCTURES offloaded
		expect(w.objects.filter((o) => o.kind === 'rabbit').length).toBe(50); // all 50 far creatures STILL LIVE — not starved
		expect(w.objects.filter((o) => o.kind === 'grave').length).toBe(100);
	});

	it('enforceLiveBudget is a no-op under budget', () => {
		const w = withCreatures('rabbit', 10, 0, 0);
		expect(enforceLiveBudget(w, 0, 0, 0, 400)).toBe(0);
		expect(w.objects.length).toBe(10);
	});

	it('enforceLiveBudget keeps survivors in ORIGINAL array order (no keyed-each reorder → no DOM-move thrash)', () => {
		// REGRESSION GUARD: an earlier version emitted survivors in DISTANCE order. Scene renders objects with a keyed
		// {#each}, so reordering forced insertBefore() DOM moves every frame the budget bit — `before` hit 41.7% of CPU
		// and the worst frame was 2.2 s. Survivors MUST keep their original relative order regardless of distance.
		const w = emptyWorld('t');
		// interleave near (DECREASING distance) with far objects, so a distance sort would REVERSE the survivors' order
		const layout = [5, 999, 4, 998, 3, 997, 2, 996, 1]; // r0..r8 ; even idx = near, odd idx = far
		layout.forEach((x, i) => w.objects.push({ id: 'r' + i, kind: 'rabbit', pos: [x, 0, 0], gene: 1 } as WorldObject));
		enforceLiveBudget(w, 0, 0, 10, 5); // keep nearest 5 (x = 1..5 → ids r8,r6,r4,r2,r0 by distance)
		expect(w.objects.map((o) => o.id)).toEqual(['r0', 'r2', 'r4', 'r6', 'r8']); // ORIGINAL order — NOT distance order
	});

	it('a dense over-budget region survives offload → sleep → wake with its FULL population (no loss, no dupes)', () => {
		// 500 rabbits packed into one region (0,0) — well over a 200 budget. Exercises the new live cap together with
		// the existing region sleep/wake, the seam most likely to drop or duplicate creatures.
		const w = emptyWorld('t');
		for (let i = 0; i < 500; i++) w.objects.push({ id: 'r' + i, kind: 'rabbit', pos: [i % 13, 0, (i * 2) % 17], gene: 1 } as WorldObject);
		const alive = (ww: World) => {
			let n = ww.objects.filter((o) => o.kind === 'rabbit').length;
			if (ww.regions) for (const k in ww.regions) n += ww.regions[k].counts.rabbit ?? 0;
			return n;
		};
		enforceLiveBudget(w, 0, 0, 2, 200); // budget 200 → 200 nearest live, 300 offloaded to region 0,0's aggregate
		expect(w.objects.length).toBe(200);
		expect(alive(w)).toBe(500); // nothing lost in the offload
		streamRegions(w, 6 * REGION_SIZE + 10, 10, 2); // walk far → region 0,0 sleeps (its 200 live merge into the 300 dormant)
		expect(w.objects.length).toBe(0);
		expect(alive(w)).toBe(500); // still all 500, now fully dormant
		streamRegions(w, 10, 10, 2); // walk back (same tick → dt 0 → no fast-forward drift) → 0,0 wakes with everyone
		expect(w.objects.filter((o) => o.kind === 'rabbit').length).toBe(500);
		expect(new Set(w.objects.map((o) => o.id)).size).toBe(w.objects.length); // ids unique → no duplicate materialise
		expect(alive(w)).toBe(500);
	});

	it('round-trip conserves the population: sleep far, then walk there to wake it', () => {
		const w = withCreatures('rabbit', 24, 8, 0); // far region 8,0
		streamRegions(w, 10, 10, 0); // player at origin → region 8,0 sleeps
		expect(creatures(w).length).toBe(0);
		expect(w.regions?.['8,0']).toBeTruthy();
		// walk into region 8,0 → it wakes (dt=0 here → no fast-forward, exact count back)
		const r = streamRegions(w, 8 * REGION_SIZE + 10, 10, 0);
		expect(r.woken).toBe(1);
		expect(creatures(w, 'rabbit').length).toBe(24);
	});

	it('an INCREMENTAL (batched) wake conserves the population even when the player crosses back OUT mid-wake', () => {
		// THE WAKE-STORM FIX GUARD. The whole population must NOT pop in on one frame, AND no creature may be lost if the
		// player leaves a half-woken region. Conservation invariant throughout: (live creatures here) + (pending in agg) = 36.
		const w = emptyWorld('t');
		w.regions = { '5,0': { counts: { rabbit: 30, lion: 6 }, gene: 1.2, statics: [], lastTick: 0 } };
		const total = 36;
		const liveHere = () => creatures(w).filter((o) => regionKey(...regionOf(o.pos[0], o.pos[2])) === '5,0').length;
		const pending = () => {
			const c = w.regions?.['5,0']?.counts ?? {};
			let n = 0;
			for (const k in c) n += c[k];
			return n;
		};
		// drain in batches of ≤5 a FEW times — each call materialises at most 5, the running live total never exceeds 36,
		// and live + pending stays exactly 36 the whole way (nothing is created or destroyed, only moved pending → live).
		let prevLive = 0;
		for (let step = 0; step < 3; step++) {
			const made = wakeRegion(w, '5,0', 0, 'rg', 5);
			expect(made).toBeLessThanOrEqual(5); // batched, never the whole population at once
			expect(liveHere()).toBeLessThanOrEqual(total);
			expect(liveHere()).toBe(prevLive + made); // monotonic — only adds, never drops
			expect(liveHere() + pending()).toBe(total); // CONSERVATION invariant holds at every step
			prevLive = liveHere();
		}
		expect(liveHere()).toBeGreaterThan(0);
		expect(liveHere()).toBeLessThan(total); // genuinely mid-wake — not yet fully materialised
		// PLAYER CROSSES BACK OUT mid-wake → collapse the region. The live (materialised) creatures must merge back into
		// the aggregate's PENDING, so the grand total is conserved with ZERO loss.
		collapseRegion(w, '5,0', 0);
		expect(liveHere()).toBe(0); // all live creatures left the world (collapsed into the aggregate)
		expect(pending()).toBe(total); // …and the aggregate now holds EVERYONE again — nothing lost across the cross-out
		// walk back in + drain fully → the original 36 all materialise, with unique ids (no dupes).
		while (w.regions?.['5,0']) wakeRegion(w, '5,0', 0, 'rg', 5);
		expect(liveHere()).toBe(total); // full population restored
		expect(w.regions?.['5,0']).toBeUndefined(); // aggregate cleared once fully awake
		expect(new Set(creatures(w).map((o) => o.id)).size).toBe(creatures(w).length); // ids unique → no duplicate materialise
	});

	it('drainWakes materialises a few creatures/frame from active regions (the per-frame WAKE-STORM drainer)', () => {
		// streamRegions with wakeBatch 0 does SETUP only (no creatures); drainWakes then dribbles them in over frames.
		const w = withCreatures('rabbit', 40, 8, 0); // far region 8,0
		streamRegions(w, 10, 10, 0); // player at origin → region 8,0 sleeps (full wake on the default-Infinity path)
		expect(creatures(w).length).toBe(0);
		expect(w.regions?.['8,0']?.counts.rabbit).toBe(40);
		// SETUP-only crossing into 8,0: fast-forward/statics happen, but ZERO creatures materialise this frame.
		streamRegions(w, 8 * REGION_SIZE + 10, 10, 0, 'rg', 0);
		expect(creatures(w).length).toBe(0); // no WAKE-STORM — nothing popped in on the crossing frame
		expect(w.regions?.['8,0']).toBeTruthy(); // still dormant (now mid-wake), counts are the pending pool
		// drain a few frames at batch 5 → ~5/frame, smoothly, until the region is fully awake.
		let total = 0;
		for (let frame = 0; frame < 8; frame++) total += drainWakes(w, 8 * REGION_SIZE + 10, 10, 0, 'rg', 5);
		expect(total).toBe(40); // all 40 materialised across the frames
		expect(creatures(w, 'rabbit').length).toBe(40);
		expect(w.regions?.['8,0']).toBeUndefined(); // fully awake → aggregate cleared
	});

	it('perimeter FENCES are never restored on wake (they were ripped out — cosmetic; colony-fear excludes animals)', () => {
		// Automatic fencing was removed: a wall around a growing/streaming cluster churned endlessly for zero gameplay
		// (animals never collided with it). A fence baked into an OLD aggregate must NOT come back when its region wakes —
		// the home does, the fence stays gone. Guards the wakeRegion fence-skip + the +page load strip.
		const w = emptyWorld('t');
		w.objects.push({ id: 'h0', kind: 'house', pos: [8 * REGION_SIZE + 20, 0, 10] } as WorldObject); // a home in region 8,0
		const fence = { id: 'fc-1', kind: 'fence', pos: [8 * REGION_SIZE + 28, 0, 10], rot: 90, scale: [4, 1, 1] } as WorldObject;
		w.objects.push(fence);
		streamRegions(w, 10, 10, 0); // player at origin → region 8,0 sleeps; home + (legacy) fence → aggregate statics
		expect(w.objects.find((o) => o.kind === 'fence')).toBeUndefined(); // not live anymore
		streamRegions(w, 8 * REGION_SIZE + 10, 10, 0); // walk back → region wakes
		expect(w.objects.find((o) => o.id === 'fc-1')).toBeUndefined(); // fence STAYS gone (skipped on wake)
		expect(w.objects.find((o) => o.id === 'h0')).toBeTruthy(); // …but the home is restored as normal
	});
});

// PEOPLE ↔ HOUSES coupling: a region's people are tied to ITS OWN houses, so a grown/returned world is BALANCED
// (people ≈ what houses support) + SPREAD, not 1100 people crammed into a few towns with ~20 houses.
describe('people ↔ houses coupling', () => {
	beforeAll(async () => {
		await math.init(); // pop_caps / ff_targets / world_area_scale come from the wasm math
	});

	// Build a dormant region aggregate in cell (cx,cz) with a given person count + house count.
	const dormant = (cx: number, cz: number, person: number, houses: number): World => {
		const w = emptyWorld('t');
		const statics: WorldObject[] = [];
		for (let i = 0; i < houses; i++) statics.push({ id: `h${cx}_${cz}_${i}`, kind: 'house', pos: [cx * REGION_SIZE + i, 0, cz * REGION_SIZE] } as WorldObject);
		w.regions = { [regionKey(cx, cz)]: { counts: { person, rabbit: 10 }, gene: 1, statics, lastTick: 0 } };
		return w;
	};

	it('B: trimDormantOvershoot drops an UNHOUSED region to a tiny nomad count', () => {
		const w = dormant(5, 5, 200, 0); // 200 people, ZERO houses → wild land, not a settlement
		const cut = trimDormantOvershoot(w);
		expect(cut).toBeGreaterThan(150);
		expect(w.regions!['5,5'].counts.person).toBeLessThanOrEqual(3); // clamped to nomads
		expect(w.regions!['5,5'].counts.rabbit).toBe(10); // wildlife untouched — only people are trimmed
	});

	it('B: trimDormantOvershoot caps a SETTLED region to its house-built person cap (not the 1100 overshoot)', () => {
		const w = dormant(5, 5, 600, 10); // 600 people but only 10 houses → must drop toward the 10-house cap
		const cut = trimDormantOvershoot(w);
		const left = w.regions!['5,5'].counts.person;
		expect(cut).toBeGreaterThan(0);
		expect(left).toBeLessThan(600); // overshoot trimmed
		expect(left).toBeGreaterThan(3); // …but it IS a settlement, so it keeps a real (housed) population, not nomads
	});

	it('B: only trims DOWN — a region already within its house cap is left alone (idempotent)', () => {
		const w = dormant(5, 5, 5, 10); // a small, well-housed population (under cap)
		expect(trimDormantOvershoot(w)).toBe(0);
		expect(w.regions!['5,5'].counts.person).toBe(5);
		expect(trimDormantOvershoot(w)).toBe(0); // second pass is a no-op (already balanced)
	});

	it('B: a FULL dormant town SPREADS — founds satellite colonies in NEW regions while away (not one capped blob)', () => {
		// "if I'm not moving, only one settlement grows + no new colonies." A full dormant town (at the colony cap, lots
		// of people) must peel founders into NEW satellite towns in OTHER regions over a long absence — real spread.
		const w = emptyWorld('t');
		const statics: WorldObject[] = [];
		for (let i = 0; i < 30; i++) statics.push({ id: `h${i}`, kind: 'house', pos: [(i % 6) * 8, 0, ((i / 6) | 0) * 8] } as WorldObject);
		w.regions = { '0,0': { counts: { person: 84 }, gene: 1, statics, lastTick: 0 } };
		const before = Object.keys(w.regions).length;
		fastForwardDormantAway(w, 12 * 3600 * 1000); // 12 h away
		const homesIn = (k: string) => w.regions![k].statics.filter((s) => s.kind === 'house' || s.kind === 'cabin').length;
		const keys = Object.keys(w.regions!);
		const satellites = keys.filter((k) => k !== '0,0' && homesIn(k) >= 2 && (w.regions![k].counts.person ?? 0) > 0);
		expect(keys.length).toBeGreaterThan(before); // the world SPREAD into new regions
		expect(satellites.length).toBeGreaterThan(0); // …and the satellites are REAL colonies (homes + people), not ghosts
	});

	it('B: fastForwardDormantAway catches up the DORMANT far world on load (frozen-while-closed fix)', () => {
		// "came back hours later, the world was STUCK" — the dormant pulse advances by SIM ticks, frozen while the app is
		// closed, so far settlements sat frozen on return. The load-time away catch-up must develop them by wall-clock.
		const w = emptyWorld('t');
		const statics: WorldObject[] = [];
		for (let i = 0; i < 3; i++) statics.push({ id: `h${i}`, kind: 'house', pos: [i * 8, 0, 0] } as WorldObject);
		w.regions = { '0,0': { counts: { person: 8, rabbit: 5 }, gene: 1, statics, lastTick: 0 } };
		const homes = () => w.regions!['0,0'].statics.filter((s) => s.kind === 'house' || s.kind === 'cabin').length;
		const h0 = homes();
		const p0 = w.regions['0,0'].counts.person;
		fastForwardDormantAway(w, 8 * 3600 * 1000); // 8 hours away
		expect(w.regions['0,0'].counts.person).toBeGreaterThan(p0); // far population caught up toward capacity
		expect(homes()).toBeGreaterThan(h0 + 4); // …and the far town DEVELOPED (the spiral, not just +6 once)
	});

	it('REPRO: offloaded settlement people must NOT be NOMAD-trimmed when their houses are still live', () => {
		// THE "30+ people all died" BUG. enforceLiveBudget offloads the FARTHEST people to a dormant aggregate, but the
		// settlement's HOUSES (a SEPARATE budget) stay live → that aggregate has people + ZERO statics → the dormant FF
		// saw builds=0 ("wild land") and NOMAD-clamped the people to 3, deleting the settlement's offloaded population a
		// few at a time as the player moved ("plucked off"). The FF must not DESTROY existing people, only refuse to GROW
		// them on truly houseless land.
		const w = emptyWorld('t');
		for (let i = 0; i < 30; i++) w.objects.push({ id: `p${i}`, kind: 'person', pos: [10 + (i % 5), 0, 10], gene: 1 } as WorldObject);
		for (let i = 0; i < 6; i++) w.objects.push({ id: `h${i}`, kind: 'house', pos: [10 + i, 0, 12] } as WorldObject);
		enforceLiveBudget(w, 0, 0, 0, 0, 50); // creatureBudget 0 → people offload; structBudget 50 → houses stay live
		expect(w.regions!['0,0'].counts.person).toBe(30);
		fastForwardDormant(w, 600); // the world pulse must CONSERVE them, not clamp to 3
		const left = (w.regions?.['0,0']?.counts.person ?? 0) + w.objects.filter((o) => o.kind === 'person').length;
		expect(left).toBeGreaterThanOrEqual(30); // the settlement's people survive (conserved), not plucked to a nomad count
	});

	it('B: a dormant SETTLEMENT develops — builds new homes as its population grows (far-town growth)', () => {
		// a 3-home hamlet with a population that outstrips its houses → fast-forward should BUILD more homes (not just
		// relax population), so a far town becomes a city over time, not only the live one you're standing in.
		const w = emptyWorld('t');
		const statics: WorldObject[] = [];
		for (let i = 0; i < 3; i++) statics.push({ id: `h${i}`, kind: 'house', pos: [i * 8, 0, 0] } as WorldObject);
		w.regions = { '0,0': { counts: { person: 40, rabbit: 5 }, gene: 1, statics, lastTick: 0 } };
		const homes = (r: World) => r.regions!['0,0'].statics.filter((s) => s.kind === 'house' || s.kind === 'cabin').length;
		const before = homes(w);
		fastForwardDormant(w, 600); // ~20 s later
		expect(homes(w)).toBeGreaterThan(before); // the far town built new homes on its own
		// a WILD region (1 house, below the settlement threshold) does NOT develop — only real settlements build out
		const wild = emptyWorld('t');
		wild.regions = { '0,0': { counts: { person: 40 }, gene: 1, statics: [{ id: 'h0', kind: 'house', pos: [0, 0, 0] } as WorldObject], lastTick: 0 } };
		fastForwardDormant(wild, 600);
		expect(wild.regions!['0,0'].statics.filter((s) => s.kind === 'house').length).toBe(1); // unchanged
	});

	it('B: snaps an OVERSHOT wildlife base down to the current density caps (calm-vicinity reload)', () => {
		// A world saved at the OLD dense base — a wild tile packed far past the new carrying capacity. Reload must snap
		// it to capacity so the near vicinity loads calm, not at the stale ~110-per-tile pack.
		const w = emptyWorld('t');
		w.regions = { '0,0': { counts: { rabbit: 300, kangaroo: 200, cat: 150, person: 0 }, gene: 1, statics: [], lastTick: 0 } };
		trimDormantOvershoot(w);
		const c = w.regions['0,0'].counts;
		expect(c.rabbit).toBeLessThan(300); // prey clamped to its density cap
		expect(c.kangaroo).toBeLessThan(200);
		expect(c.cat).toBeLessThanOrEqual(Math.round(c.rabbit * 0.3) + 1); // predator shares off the CLAMPED prey, not the old 300
		expect(c.rabbit).toBeGreaterThan(0); // …but never wiped — it relaxes to capacity, doesn't go extinct
	});

	it('A: fastForwardDormant does NOT grow/re-seed people on UNHOUSED land (no wild-region person boom)', () => {
		const w = dormant(5, 5, 1, 0); // one lone wanderer, no houses
		fastForwardDormant(w, 30 * 60 * math.tickHz()); // 30 sim-minutes of dormant pulse
		expect(w.regions!['5,5'].counts.person).toBeLessThanOrEqual(3); // stayed nomadic — no re-seed to ~38
		expect(w.regions!['5,5'].counts.rabbit).toBeGreaterThan(10); // …but rabbits DID grow (wildlife belongs in the wild)
	});

	it('A: fastForwardDormant DOES grow people in a HOUSED region (toward its house cap)', () => {
		const w = dormant(5, 5, 4, 20); // a real settlement, room to grow
		fastForwardDormant(w, 6 * 3600 * math.tickHz()); // 6 sim-hours away
		expect(w.regions!['5,5'].counts.person).toBeGreaterThan(4); // the housed region's people grew
	});
});
