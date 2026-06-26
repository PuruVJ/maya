import { describe, it, expect } from 'vitest';
import { emptyWorld, type World, type WorldObject } from './world';
import { regionOf, regionKey, activeKeys, collapseRegion, wakeRegion, streamRegions, enforceLiveBudget, REGION_SIZE } from './streaming';

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

	it('a static structure round-trips verbatim through sleep→wake (same id + position)', () => {
		const w = emptyWorld('t');
		w.objects.push({ id: 'h7', kind: 'tower', pos: [8 * REGION_SIZE + 5, 3, 9], keep: true } as WorldObject);
		streamRegions(w, 10, 10, 0); // player at origin → region 8,0 sleeps (the tower offloads)
		expect(w.objects.length).toBe(0);
		streamRegions(w, 8 * REGION_SIZE + 10, 10, 0); // walk there → wakes
		const tower = w.objects.find((o) => o.kind === 'tower');
		expect(tower?.id).toBe('h7'); // exact id preserved (durable)
		expect(tower?.pos).toEqual([8 * REGION_SIZE + 5, 3, 9]); // exact position preserved
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

	it('a settlement FENCE survives the sleep→wake round-trip verbatim (it is a durable static, not regenerated)', () => {
		// The fence is fitted only on a structure add/remove, then STORED — never rebuilt when the player approaches.
		// That only holds if the fence panels travel with their region into dormancy and come back unchanged. Guards
		// against the fence vanishing (or needing a presence-driven rebuild) after you walk away from a town and return.
		const w = emptyWorld('t');
		w.objects.push({ id: 'h0', kind: 'house', pos: [8 * REGION_SIZE + 20, 0, 10] } as WorldObject); // a home in region 8,0
		const fence = { id: 'fc-1', kind: 'fence', pos: [8 * REGION_SIZE + 28, 0, 10], rot: 90, scale: [4, 1, 1] } as WorldObject;
		w.objects.push(fence);
		streamRegions(w, 10, 10, 0); // player at origin → region 8,0 sleeps; home + fence → aggregate statics
		expect(w.objects.find((o) => o.kind === 'fence')).toBeUndefined(); // not live anymore
		expect(w.regions?.['8,0']?.statics.some((s) => s.id === 'fc-1')).toBe(true); // preserved verbatim
		streamRegions(w, 8 * REGION_SIZE + 10, 10, 0); // walk back → region wakes
		const back = w.objects.find((o) => o.id === 'fc-1');
		expect(back).toBeTruthy(); // the SAME fence panel is back…
		expect(back!.pos).toEqual([8 * REGION_SIZE + 28, 0, 10]); // …at the exact same spot (no regeneration, no drift)
		expect(back!.rot).toBe(90);
	});
});
