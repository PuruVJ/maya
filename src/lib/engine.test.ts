// BOUNDARY + behavior guard for the binary engine (engine.ts applyOps → wasm apply_ops_bin). The op→world logic
// itself is proven in Rust (crates/worldsim engine_bin tests); this pins the JS side: the world+ops pack/unpack round
// trip, and — critically — that unpackInto's MERGE-BY-ID preserves per-object live-state (genome/asleep/…) that never
// crosses the wasm boundary. (The old jzon path this used to be compared against has been deleted.)
import { describe, it, expect, beforeAll } from 'vitest';
import { applyOps, type Op, type PlacementConflict } from './engine';
import { math } from './math';
import type { World, Player } from './world';

function makeWorld(): World {
	return {
		objects: [
			{ id: 'o5', kind: 'house', pos: [10, 0, 10], scale: [1, 1, 1], rot: 0, keep: true },
			// a live creature with snapshot fields that ONLY exist JS-side — must survive the round-trip untouched
			{ id: 'o6', kind: 'cat', pos: [12, 0, 11], gene: 0.7, genome: [0.1, 0.2, 0.3, 0.4, 0.5], dead: false, asleep: true },
		],
		zones: [{ id: 'z2', material: 'water', shape: 'blob', pos: [-30, 0, -30], size: 8 }],
		paths: [],
		terrain: [{ center: [0, 0], radius: 20, height: 5, rough: 0.5 }],
		ground: 'grass',
		sky: 'night',
	} as unknown as World;
}

const OPS: Op[] = [
	{ op: 'add', kind: 'house', at: 'front' },
	{ op: 'add', kind: 'rabbit', at: 'around:house', count: 6 },
	{ op: 'add', kind: 'kangaroo', count: 12 }, // >8 creatures → band_spread
	{ op: 'add', kind: 'cat', at: 'on:o5', color: '#0f0' },
	{ op: 'scatter', kind: 'flower', area: 'center', count: 20 },
	{ op: 'move', id: 'o6', at: 'north' },
	{ op: 'paint', id: 'o5', color: '#f00' },
	{ op: 'addZone', material: 'plaza', shape: 'round', size: 10 },
	{ op: 'addZone', material: 'water', shape: 'blob', pos: [10, 0, 10], size: 14 },
	{ op: 'addPath', material: 'path', from: 'here', to: 'north', width: 3 },
	{ op: 'setTerrain', preset: 'mountains' },
	{ op: 'remove', id: 'rabbit' },
	{ op: 'setGround', value: 'sand' },
	{ op: 'setSky', value: 'night' },
	{ op: 'note', text: 'hi' },
];

describe('binary engine boundary (applyOps → apply_ops_bin)', () => {
	beforeAll(async () => {
		await math.init();
	});

	it('applies every op branch end-to-end through the binary boundary', () => {
		const out: { conflicts: PlacementConflict[] } = { conflicts: [] };
		const w = applyOps(makeWorld(), OPS, { pos: [3, 0, -2], yaw: 0.6 }, out);

		// the ops built a populous world (house + 6-ring rabbits + 12 band kangaroos + on-top cat + 20 flowers − 1 rabbit)
		expect(w.objects.length).toBeGreaterThan(40);
		// CRUD on existing objects: o5 painted + still kept; the on-top cat got its colour
		const o5 = w.objects.find((o) => o.id === 'o5')!;
		expect([o5.color, o5.keep]).toEqual(['#f00', true]);
		expect(w.objects.some((o) => o.kind === 'cat' && o.color === '#0f0')).toBe(true);
		// scenery + world fields round-tripped
		expect(w.zones.some((z) => z.material === 'plaza')).toBe(true);
		expect(w.zones.filter((z) => z.material === 'water').length).toBe(2); // the original + the added lake
		expect(w.paths.length).toBe(1);
		expect(w.terrain.length).toBe(2); // original feature + the mountains preset
		expect([w.ground, w.sky]).toEqual(['sand', 'night']);
		expect(Array.isArray(out.conflicts)).toBe(true);
	});

	it('MERGE-BY-ID preserves a moved creature snapshot (genome/asleep/gene) the boundary never carries', () => {
		const w = applyOps(makeWorld(), [{ op: 'move', id: 'o6', at: 'north' }], { pos: [0, 0, 0], yaw: 0 });
		const cat = w.objects.find((o) => o.id === 'o6')!;
		expect(cat.genome).toEqual([0.1, 0.2, 0.3, 0.4, 0.5]); // unpackInto kept the JS-only fields
		expect(cat.asleep).toBe(true);
		expect(cat.gene).toBe(0.7);
		expect(cat.pos[2]).toBeLessThan(0); // actually moved north (−Z area)
	});
});
