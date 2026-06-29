import { describe, it, expect } from 'vitest';
import { splitWorld, mergeWorld, regionSig } from './worldStore';
import type { World, RegionAggregate, WorldObject } from './world';

// structures carry explicit rot/scale so the binary codec round-trips key-exact (packPersist normalises absent rot→0,
// scale→[1,1,1] — semantically identical, but the test asserts deep equality).
const house = (id = 'h0', pos: [number, number, number] = [10, 5, 20]): WorldObject => ({ id, kind: 'house', pos, rot: 0, scale: [1, 1, 1] });
const person = (id = 'p0'): WorldObject => ({ id, kind: 'person', pos: [3, 0, 4], gene: 1.1, genome: [1, 2, 3, 4, 5] });

const region = (over: Partial<RegionAggregate> = {}): RegionAggregate => ({
	counts: { rabbit: 3, cat: 1 },
	gene: 0.5,
	statics: [house('rh0', [12, 3, 22])],
	lastTick: 100,
	...over
});

const world = (over: Partial<World> = {}): World => ({
	v: 1,
	name: 'Test',
	ground: '#3a5',
	sky: '#9cf',
	spawn: [0, 0, 0],
	objects: [house(), person()], // a structure (→ binary SoA) + a creature (→ verbatim rest sidecar)
	zones: [],
	paths: [],
	terrain: [],
	regions: { '0,0': region(), '1,-1': region({ lastTick: 200 }) },
	...over
});

const storedRegion = (over: Partial<RegionAggregate> = {}) => splitWorld(world({ regions: { k: region(over) } })).regions.k;

describe('worldStore split/merge (binary-encoded)', () => {
	it('binary-encodes structures, keeps regions in their own keys', () => {
		const { meta, regions } = splitWorld(world());
		expect((meta as { regions?: unknown }).regions).toBeUndefined(); // regions never ride inside meta
		expect(meta.regionKeys.sort()).toEqual(['0,0', '1,-1']);
		expect(meta.ids).toEqual(['h0']); // the house → SoA + id list
		expect(meta.soa).toBeInstanceOf(Float64Array);
		expect(meta.rest).toEqual([person()]); // the creature → verbatim sidecar (all fields survive)
		expect(regions['0,0'].soa).toBeInstanceOf(Float64Array);
		expect(regions['0,0'].ids).toEqual(['rh0']);
	});

	it('round-trips a world through split → merge unchanged', () => {
		const w = world();
		const { meta, regions } = splitWorld(w);
		expect(mergeWorld(meta, regions)).toEqual(w);
	});

	it('a world with no regions merges back without a regions field', () => {
		const w = world({ regions: undefined });
		const { meta, regions } = splitWorld(w);
		expect(meta.regionKeys).toEqual([]);
		expect(regions).toEqual({});
		expect('regions' in mergeWorld(meta, {})).toBe(false);
	});

	it('merge tolerates a region key whose blob failed to load (dropped silently)', () => {
		const { meta, regions } = splitWorld(world());
		const back = mergeWorld(meta, { '0,0': regions['0,0'] }); // only one of the two region blobs came back
		expect(Object.keys(back.regions!)).toEqual(['0,0']);
	});

	it('tolerates a legacy (B-step-1) structured-clone blob — objects/_statics, no SoA', () => {
		// a meta/region written before the binary codec: objects & statics are plain arrays, no soa/ids/rest.
		const legacyMeta = { v: 1, name: 'L', ground: '#000', sky: '#000', spawn: [0, 0, 0], zones: [], paths: [], terrain: [], regionKeys: ['0,0'], objects: [house()] } as unknown as Parameters<typeof mergeWorld>[0];
		const legacyRegion = { counts: { cat: 1 }, gene: 0.5, lastTick: 100, statics: [house('rh0')] } as unknown as Parameters<typeof mergeWorld>[1][string];
		const back = mergeWorld(legacyMeta, { '0,0': legacyRegion });
		expect(back.objects).toEqual([house()]);
		expect(back.regions!['0,0'].statics).toEqual([house('rh0')]);
	});
});

describe('worldStore regionSig', () => {
	it('is stable for an unchanged aggregate (→ skip the rewrite)', () => {
		expect(regionSig(storedRegion())).toBe(regionSig(storedRegion()));
	});

	it('changes when the region re-collapses (lastTick), a static is added, or a count moves', () => {
		const base = regionSig(storedRegion());
		expect(regionSig(storedRegion({ lastTick: 101 }))).not.toBe(base);
		expect(regionSig(storedRegion({ statics: [house('a'), house('b')] }))).not.toBe(base);
		expect(regionSig(storedRegion({ counts: { rabbit: 4, cat: 1 } }))).not.toBe(base);
	});

	it('is order-independent in the counts map', () => {
		const a = regionSig(storedRegion({ counts: { rabbit: 3, cat: 1 } }));
		const b = regionSig(storedRegion({ counts: { cat: 1, rabbit: 3 } }));
		expect(a).toBe(b);
	});
});
