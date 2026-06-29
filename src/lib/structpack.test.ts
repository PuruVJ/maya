import { describe, it, expect } from 'vitest';
import { kindCode, kindStr, isStructureKind, packStructures, packWaterZones, packPersist, unpackPersist, CODE_KIND, SOA_STRIDE } from './structpack';
import type { WorldObject, Zone } from './world';

describe('structpack', () => {
	it('kindCode round-trips + matches the Rust SK_ order', () => {
		for (const k of CODE_KIND) expect(kindStr(kindCode(k))).toBe(k);
		expect(kindCode('house')).toBe(0);
		expect(kindCode('fence')).toBe(5);
		expect(kindCode('bridge')).toBe(13);
		expect(kindCode('person')).toBe(255); // creatures aren't structures
		expect(isStructureKind('house')).toBe(true);
		expect(isStructureKind('person')).toBe(false);
	});

	it('packs structures into the SoA + idBySlot, skipping creatures', () => {
		const objs: WorldObject[] = [
			{ id: 'h0', kind: 'house', pos: [10, 0, 20], scale: [1.2, 1.2, 1.2] },
			{ id: 'p0', kind: 'person', pos: [0, 0, 0] }, // creature → skipped (not a structure)
			{ id: 'f0', kind: 'fence', pos: [5, 0, 5], rot: 137, scale: [4.6, 1, 1], color: '#7c5230', keep: false }
		];
		const idBySlot: string[] = [];
		const soa = packStructures(objs, idBySlot);
		expect(idBySlot).toEqual(['h0', 'f0']); // the person is skipped
		expect(soa.length).toBe(2 * SOA_STRIDE);
		// house: [code0, x10, z20, rot0, sx1.2, sy1.2, sz1.2, color0, keep0]
		expect(Array.from(soa.slice(0, 9))).toEqual([0, 10, 20, 0, 1.2, 1.2, 1.2, 0, 0]);
		expect(soa[9 + 0]).toBe(5); // fence kind
		expect(soa[9 + 3]).toBe(137); // rot preserved EXACTLY (degrees — never normalised)
		expect(soa[9 + 7]).toBe(0x7c5230); // packed color
	});

	it('persist codec round-trips structures + sidelines creatures verbatim', () => {
		const objs: WorldObject[] = [
			{ id: 'h0', kind: 'house', pos: [10, 5, 20], rot: 0, scale: [1, 1, 1] }, // Y preserved (verbatim restore)
			{ id: 'w0', kind: 'well', pos: [3, 1, 4], rot: 0.5, scale: [1, 1, 1], keep: true },
			{ id: 'f0', kind: 'fence', pos: [5, 0, 5], rot: 137, scale: [4.6, 1, 1], color: '#7c5230' },
			{ id: 'p0', kind: 'person', pos: [0, 0, 0], gene: 1.1, genome: [1, 2, 3, 4, 5] } // creature → rest sidecar
		];
		const { soa, ids, rest } = packPersist(objs);
		expect(ids).toEqual(['h0', 'w0', 'f0']); // 3 structures → SoA
		expect(soa.length).toBe(3 * 10); // PERSIST_STRIDE = 10
		expect(rest).toEqual([objs[3]]); // the creature kept whole (gene/genome survive)
		expect([...unpackPersist(soa, ids), ...rest]).toEqual(objs); // lossless round-trip, structures-first order
	});

	it('packs only water zones, with the supplied seed', () => {
		const zones: Zone[] = [
			{ id: 'lake', material: 'water', shape: 'blob', pos: [100, 0, 0], size: 20 },
			{ id: 'road', material: 'path', shape: 'rect', pos: [0, 0, 0], size: 5 }
		];
		const z = packWaterZones(zones, (id) => (id === 'lake' ? 0.42 : 0));
		expect(Array.from(z)).toEqual([100, 0, 20, 0.42]); // only the water zone, with its seed
	});
});
