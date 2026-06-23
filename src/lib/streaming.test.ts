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
		expect(enforceLiveBudget(w, 0, 0, 5, 1)).toBe(1); // budget 1 → keep nearest (rabbit), evict the house
		expect(w.objects.map((o) => o.id)).toEqual(['r0']);
		const key = regionKey(...regionOf(9999, 0));
		expect(w.regions?.[key]?.statics[0]?.id).toBe('h0'); // house preserved verbatim for round-trip
	});

	it('enforceLiveBudget is a no-op under budget', () => {
		const w = withCreatures('rabbit', 10, 0, 0);
		expect(enforceLiveBudget(w, 0, 0, 0, 400)).toBe(0);
		expect(w.objects.length).toBe(10);
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
});
