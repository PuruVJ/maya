import { describe, it, expect } from 'vitest';
import { applyOps, overlaps, type Op } from './engine';
import { emptyWorld, type Player } from './world';
import { heightAt } from './terrain';

function build(ops: Op[], player: Player = { pos: [0, 0, 0], yaw: 0 }) {
	const w = emptyWorld();
	applyOps(w, ops, player);
	return w;
}

describe('egocentric anchors (relative to player yaw)', () => {
	it('front places ahead (-Z at yaw 0)', () => {
		const o = build([{ op: 'add', kind: 'lamp', at: 'front' }]).objects[0];
		expect(o.pos[2]).toBeLessThan(0);
		expect(Math.abs(o.pos[0])).toBeLessThan(1);
	});

	it('behind places behind (+Z at yaw 0)', () => {
		const o = build([{ op: 'add', kind: 'lamp', at: 'behind' }]).objects[0];
		expect(o.pos[2]).toBeGreaterThan(0);
	});

	it('left is -X and right is +X at yaw 0', () => {
		const l = build([{ op: 'add', kind: 'lamp', at: 'left' }]).objects[0];
		const r = build([{ op: 'add', kind: 'lamp', at: 'right' }]).objects[0];
		expect(l.pos[0]).toBeLessThan(0);
		expect(r.pos[0]).toBeGreaterThan(0);
	});

	it('directions rotate with yaw (front → -X when facing -X)', () => {
		const o = build([{ op: 'add', kind: 'lamp', at: 'front' }], { pos: [0, 0, 0], yaw: Math.PI / 2 })
			.objects[0];
		expect(o.pos[0]).toBeLessThan(0);
		expect(Math.abs(o.pos[2])).toBeLessThan(1);
	});

	it('dist controls how far front reaches', () => {
		const near = build([{ op: 'add', kind: 'lamp', at: 'front', dist: 4 }]).objects[0];
		const far = build([{ op: 'add', kind: 'lamp', at: 'front', dist: 30 }]).objects[0];
		expect(Math.abs(far.pos[2])).toBeGreaterThan(Math.abs(near.pos[2]) + 10);
	});

	it('tolerates the model writing "behind:me" instead of "behind"', () => {
		const o = build([{ op: 'add', kind: 'lamp', at: 'behind:me' }]).objects[0];
		expect(o.pos[2]).toBeGreaterThan(0); // still behind, not in front
	});
});

describe('inter-object anchors', () => {
	it('between:a,b places at the midpoint of two objects', () => {
		const w = build(
			[
				{ op: 'add', kind: 'house', pos: [-10, 0, 0] },
				{ op: 'add', kind: 'house', pos: [10, 0, 0] },
				{ op: 'add', kind: 'well', at: 'between:o0,o1' }
			],
			{ pos: [0, 0, 25], yaw: 0 } // player far so avoidance doesn't shift the midpoint
		);
		const well = w.objects[2];
		expect(well.pos[0]).toBeCloseTo(0, 0);
		expect(well.pos[2]).toBeCloseTo(0, 0);
	});

	it('on:<id> places at the object roof height (no overlap with ground placement)', () => {
		const w = build([
			{ op: 'add', kind: 'house', pos: [5, 0, 5] }, // house h = 3
			{ op: 'add', kind: 'lamp', at: 'on:o0' }
		]);
		const lamp = w.objects[1];
		expect(lamp.pos[0]).toBeCloseTo(5, 0);
		expect(lamp.pos[2]).toBeCloseTo(5, 0);
		expect(lamp.pos[1]).toBeCloseTo(3, 5); // exactly on the roof
	});

	it('near:<kind> resolves fuzzily to the nearest object of that kind', () => {
		const w = build([
			{ op: 'add', kind: 'tower', pos: [10, 0, 10] },
			{ op: 'add', kind: 'lamp', at: 'near:tower' }
		]);
		const lamp = w.objects[1];
		expect(Math.hypot(lamp.pos[0] - 10, lamp.pos[2] - 10)).toBeLessThan(6);
	});
});

describe('collision-free placement', () => {
	it('never places objects whose footprints overlap', () => {
		const w = build([
			{ op: 'scatter', kind: 'tree', count: 20, area: 'north' },
			{ op: 'add', kind: 'house', at: 'here' },
			{ op: 'add', kind: 'house', at: 'here' },
			{ op: 'add', kind: 'tower', at: 'front' }
		]);
		expect(overlaps(w)).toHaveLength(0);
	});

	it('shifts placement off the player ("here" never lands on you)', () => {
		const o = build([{ op: 'add', kind: 'house', at: 'here' }]).objects[0];
		expect(Math.hypot(o.pos[0], o.pos[2])).toBeGreaterThan(2);
	});
});

describe('scatter', () => {
	it('spawns the requested count in the right area, no overlaps', () => {
		const w = build([{ op: 'scatter', kind: 'pine', count: 12, area: 'north' }]);
		expect(w.objects).toHaveLength(12);
		expect(w.objects.every((o) => o.kind === 'pine')).toBe(true);
		expect(w.objects.every((o) => o.pos[2] < 0)).toBe(true); // north = -Z
		expect(overlaps(w)).toHaveLength(0);
	});
});

describe('mutation ops + robustness', () => {
	it('paint / setGround / setSky / remove mutate as expected', () => {
		const w = build([
			{ op: 'add', kind: 'house', pos: [0, 0, -6] }, // o0
			{ op: 'paint', id: 'o0', color: '#ffffff' },
			{ op: 'setGround', value: 'snow' },
			{ op: 'setSky', value: 'night' }
		]);
		expect(w.objects[0].color).toBe('#ffffff');
		expect(w.ground).toBe('snow');
		expect(w.sky).toBe('night');
		applyOps(w, [{ op: 'remove', id: 'o0' }]);
		expect(w.objects).toHaveLength(0);
	});

	it('ignores ops referencing a nonexistent id (no crash, no-op)', () => {
		const w = build([
			{ op: 'remove', id: 'nope' },
			{ op: 'paint', id: 'ghost', color: '#fff' }
		]);
		expect(w.objects).toHaveLength(0);
	});
});

describe('zones & paths', () => {
	it('addZone stores a zone at the resolved area anchor', () => {
		const w = build([{ op: 'addZone', material: 'water', shape: 'blob', at: 'east', size: 12 }]);
		expect(w.zones).toHaveLength(1);
		expect(w.zones[0].material).toBe('water');
		expect(w.zones[0].pos[0]).toBeCloseTo(30, 0); // east = +X
		expect(w.zones[0].size).toBe(12);
	});

	it('extends a degenerate path (both ends collapse) instead of a zero-length stub', () => {
		const w = build([{ op: 'addPath', material: 'path', from: 'here', to: 'here', width: 3 }]);
		expect(w.paths).toHaveLength(1);
		const { from, to } = w.paths[0];
		expect(Math.hypot(to[0] - from[0], to[2] - from[2])).toBeGreaterThan(8);
	});

	it('a path "behind" runs backward (+Z), not forward (regression: "road behind me drew in front")', () => {
		const { from, to } = build([
			{ op: 'addPath', material: 'path', from: 'here', to: 'behind:me', width: 3 }
		]).paths[0];
		expect(to[2]).toBeGreaterThan(from[2]);
	});

	it('a lake steers clear of objects (finds open space, no conflict)', () => {
		const w = build([{ op: 'add', kind: 'house', pos: [0, 0, 0] }]);
		const out = { conflicts: [] as { label: string; blockers: string[] }[] };
		applyOps(w, [{ op: 'addZone', material: 'water', shape: 'blob', at: 'center', size: 8 }], { pos: [0, 0, 0], yaw: 0 }, out);
		expect(out.conflicts.length).toBe(0); // moved to clear ground → no demolish needed
		expect(Math.hypot(w.zones[0].pos[0], w.zones[0].pos[2])).toBeGreaterThan(0); // not on the house
	});

	it('a bare "dig a lake" lands ahead of the player, never on their feet', () => {
		const w = emptyWorld();
		const out = { conflicts: [] as { label: string; blockers: string[] }[] };
		applyOps(w, [{ op: 'addZone', material: 'water', shape: 'blob', size: 10 }], { pos: [0, 0, 0], yaw: 0 }, out);
		expect(w.zones[0].pos[2]).toBeLessThan(-1); // facing -Z → lake is ahead, not underfoot
	});
});

describe('determinism (shareable worlds reproduce exactly)', () => {
	it('same ops + player → identical layout', () => {
		const ops: Op[] = [
			{ op: 'scatter', kind: 'tree', count: 15, area: 'north' },
			{ op: 'add', kind: 'house', at: 'front' },
			{ op: 'add', kind: 'well', at: 'left' }
		];
		const a = build(ops);
		const b = build(ops);
		expect(JSON.stringify(a.objects)).toBe(JSON.stringify(b.objects));
	});
});

describe('terrain (localized features)', () => {
	it('a flat world keeps objects at y=0', () => {
		const w = build([{ op: 'add', kind: 'house', pos: [10, 0, 10] }]);
		expect(w.objects[0].pos[1]).toBe(0);
	});

	it('setTerrain adds ONE contained feature and re-grounds objects', () => {
		const w = build([
			{ op: 'add', kind: 'house', pos: [20, 0, -5] },
			{ op: 'setTerrain', preset: 'mountains' }
		]);
		expect(w.terrain.length).toBe(1);
		const f = w.terrain[0];
		expect(Math.abs(heightAt(f.center[0], f.center[1], w.terrain))).toBeGreaterThan(2); // peak raised
		expect(w.objects[0].pos[1]).toBeCloseTo(heightAt(20, -5, w.terrain), 5); // re-grounded
	});

	it('a feature is contained: raised at its centre, adds nothing far away', () => {
		const feats = [{ center: [0, 0] as [number, number], radius: 18, height: 6, rough: 1 }];
		expect(Math.abs(heightAt(0, 0, feats))).toBeGreaterThan(3);
		// far from the feature it contributes nothing (ambient relief aside)
		expect(heightAt(100, 100, feats)).toBeCloseTo(heightAt(100, 100, []), 5);
	});

	it('the flat preset clears all terrain', () => {
		const w = build([
			{ op: 'setTerrain', preset: 'hills' },
			{ op: 'setTerrain', preset: 'flat' }
		]);
		expect(w.terrain.length).toBe(0);
	});
});

describe('counts & combos', () => {
	it('add with count places N non-overlapping objects near the anchor', () => {
		const w = build([{ op: 'add', kind: 'cabin', at: 'front', count: 3 }]);
		expect(w.objects.length).toBe(3);
		expect(overlaps(w)).toEqual([]);
	});

	it('add defaults to a single object when count is omitted', () => {
		const w = build([{ op: 'add', kind: 'house', at: 'here' }]);
		expect(w.objects.length).toBe(1);
	});

	it('scatter spawns many of any kind (e.g. 40 cats) without overlaps', () => {
		const w = build([{ op: 'scatter', kind: 'cat', count: 40, area: 'everywhere' }]);
		expect(w.objects.length).toBe(40);
		expect(w.objects.every((o) => o.kind === 'cat')).toBe(true);
		expect(overlaps(w)).toEqual([]);
	});

	it('runaway counts are clamped (add 9999 → MAX_COUNT)', () => {
		const w = build([{ op: 'add', kind: 'flower', at: 'here', count: 9999 }]);
		expect(w.objects.length).toBe(120);
	});
});

describe('composite & CRUD prompts (cross-references in one batch)', () => {
	it('a later op references an earlier one by kind (add house, then tree near:house)', () => {
		const w = build([
			{ op: 'add', kind: 'house', at: 'here' },
			{ op: 'add', kind: 'tree', at: 'near:house' }
		]);
		const house = w.objects.find((o) => o.kind === 'house')!;
		const tree = w.objects.find((o) => o.kind === 'tree')!;
		const d = Math.hypot(tree.pos[0] - house.pos[0], tree.pos[2] - house.pos[2]);
		expect(d).toBeLessThan(8); // dropped right beside the house, not at a default spot
	});

	it('on:last stacks onto the most-recently-added object', () => {
		const w = build([
			{ op: 'add', kind: 'tower', at: 'front' },
			{ op: 'add', kind: 'lamp', at: 'on:last' }
		]);
		const tower = w.objects.find((o) => o.kind === 'tower')!;
		const lamp = w.objects.find((o) => o.kind === 'lamp')!;
		expect(Math.hypot(lamp.pos[0] - tower.pos[0], lamp.pos[2] - tower.pos[2])).toBeLessThan(0.5);
		expect(lamp.pos[1]).toBeGreaterThan(1); // up on the roof
	});

	it('paint/remove resolve targets by kind, not just exact id', () => {
		const w = build([
			{ op: 'add', kind: 'house', at: 'here' },
			{ op: 'paint', id: 'house', color: '#b22222' },
			{ op: 'add', kind: 'well', at: 'left' },
			{ op: 'remove', id: 'well' }
		]);
		expect(w.objects.find((o) => o.kind === 'house')!.color).toBe('#b22222');
		expect(w.objects.some((o) => o.kind === 'well')).toBe(false);
	});

	it('a garbage remove id is a no-op (never nukes a random object)', () => {
		const w = build([
			{ op: 'add', kind: 'house', at: 'here' },
			{ op: 'remove', id: 'dragon' }
		]);
		expect(w.objects.length).toBe(1);
	});
});

describe('things land near the player (not the world origin)', () => {
	it('scatter clusters around the player', () => {
		const w = build([{ op: 'scatter', kind: 'cat', count: 12, area: 'everywhere' }], { pos: [40, 0, 40], yaw: 0 });
		const ax = w.objects.reduce((s, o) => s + o.pos[0], 0) / w.objects.length;
		const az = w.objects.reduce((s, o) => s + o.pos[2], 0) / w.objects.length;
		expect(Math.abs(ax - 40)).toBeLessThan(30);
		expect(Math.abs(az - 40)).toBeLessThan(30);
	});

	it('"near:here" places by the player, not a far existing object', () => {
		const w = emptyWorld();
		applyOps(w, [{ op: 'add', kind: 'tower', pos: [80, 0, 80] }], { pos: [0, 0, 0], yaw: 0 });
		applyOps(w, [{ op: 'add', kind: 'lamp', at: 'near:here' }], { pos: [0, 0, 0], yaw: 0 });
		const lamp = w.objects.find((o) => o.kind === 'lamp')!;
		expect(Math.hypot(lamp.pos[0], lamp.pos[2])).toBeLessThan(12);
	});
});

describe('object ids never collide (no Svelte each_key_duplicate)', () => {
	it('reuses no id after a remove, across multiple applyOps calls', () => {
		const p: Player = { pos: [0, 0, 0], yaw: 0 };
		const w = emptyWorld();
		applyOps(w, [{ op: 'add', kind: 'tree', at: 'here' }, { op: 'add', kind: 'tree', at: 'left' }, { op: 'add', kind: 'tree', at: 'right' }], p);
		applyOps(w, [{ op: 'remove', id: 'o1' }], p); // remove the middle one → array shrinks
		applyOps(w, [{ op: 'add', kind: 'rock', at: 'front' }, { op: 'add', kind: 'rock', at: 'behind' }], p);
		const ids = w.objects.map((o) => o.id);
		expect(new Set(ids).size).toBe(ids.length); // every id unique
	});
});

describe('object-relative anchors (in front of / around a thing)', () => {
	const P: Player = { pos: [0, 0, 0], yaw: 0 }; // facing -Z; "front" = -Z

	it('"front:<id>" places in front of that OBJECT, not the player', () => {
		const w = emptyWorld();
		applyOps(w, [{ op: 'add', kind: 'house', pos: [20, 0, 0] }], P); // house off to the side
		applyOps(w, [{ op: 'add', kind: 'lamp', at: 'front:o0' }], P);
		const lamp = w.objects.find((o) => o.kind === 'lamp')!;
		// in front of the house (near x=20), NOT in front of the player (near x=0)
		expect(Math.abs(lamp.pos[0] - 20)).toBeLessThan(6);
		expect(lamp.pos[2]).toBeLessThan(0); // -Z = "front" at yaw 0
	});

	it('"around:<id>" rings several objects around the target (none on its centre)', () => {
		const w = emptyWorld();
		applyOps(w, [{ op: 'add', kind: 'house', at: 'here' }], P);
		const house = w.objects[0];
		applyOps(w, [{ op: 'add', kind: 'fence', at: 'around:o0', count: 8 }], P);
		const fences = w.objects.filter((o) => o.kind === 'fence');
		expect(fences.length).toBeGreaterThanOrEqual(8);
		// every fence sits out at a similar radius from the house centre (a ring, not a pile)
		for (const f of fences) {
			const d = Math.hypot(f.pos[0] - house.pos[0], f.pos[2] - house.pos[2]);
			expect(d).toBeGreaterThan(1.5);
			expect(d).toBeLessThan(12);
		}
	});

	it('placing near a thing never removes it', () => {
		const w = emptyWorld();
		applyOps(w, [{ op: 'add', kind: 'house', at: 'here' }], P);
		applyOps(w, [{ op: 'add', kind: 'tower', at: 'near:house' }], P);
		expect(w.objects.some((o) => o.kind === 'house')).toBe(true); // house survived
		expect(w.objects.some((o) => o.kind === 'tower')).toBe(true);
	});
});
