// PARITY GUARD for the JS→Rust world-gen port. The procedural generators (settlement planner; later city/forest/
// lake) moved into Rust (crates/worldsim/src/worldgen.rs) so Rust owns all world-gen compute. This pins the Rust
// output to the original JS algorithm so the port can't silently change a generated town. The reference JS impl is
// embedded here as the spec-of-record (the production copy is now a thin wasm bridge), so the guard stays meaningful
// after the production JS is reduced to delegation.
import { describe, it, expect, beforeAll } from 'vitest';
import { math } from './math';
import type { WorldObject, Path, World, Player } from './world';
import type { Op } from './engine';
import { inWater } from './water';
import { forestOps as binForestOps, lakeOps as binLakeOps, cityOps as binCityOps } from './city';
import { settlementPlan as binTownPlan } from './settlementPlanner';

// ── REFERENCE algorithm (verbatim from the pre-port settlementPlanner.ts) — the spec the Rust port must match ──
type Size = 'hamlet' | 'village' | 'town' | 'city';
const PLAN: Record<Size, { blocks: number; houses: number; towers: number; fenced: boolean }> = {
	hamlet: { blocks: 1, houses: 4, towers: 0, fenced: false },
	village: { blocks: 2, houses: 10, towers: 1, fenced: false },
	town: { blocks: 3, houses: 20, towers: 1, fenced: true },
	city: { blocks: 4, houses: 34, towers: 2, fenced: true }
};
const GAP = 18, SETBACK = 6, HOUSE_SPACING = 7.5;
function prng(seed: number): () => number {
	let a = seed >>> 0;
	return () => {
		a = (a + 0x6d2b79f5) | 0;
		let t = Math.imul(a ^ (a >>> 15), 1 | a);
		t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
		return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
	};
}
function refPlan(cx: number, cz: number, size: Size, seed: number, idPrefix: string): { objects: WorldObject[]; paths: Path[]; radius: number } {
	const rng = prng(seed);
	const p = PLAN[size];
	const objects: WorldObject[] = [];
	const paths: Path[] = [];
	let n = 0;
	const oid = () => `${idPrefix}o${n++}`;
	const pid = () => `${idPrefix}p${n++}`;
	const half = (p.blocks * GAP) / 2;
	const lines: number[] = [];
	for (let i = 0; i <= p.blocks; i++) lines.push(-half + i * GAP);
	for (const off of lines) {
		paths.push({ id: pid(), material: 'path', from: [cx - half, 0, cz + off], to: [cx + half, 0, cz + off], width: 3 });
		paths.push({ id: pid(), material: 'path', from: [cx + off, 0, cz - half], to: [cx + off, 0, cz + half], width: 3 });
	}
	const kinds = size === 'hamlet' ? ['cabin', 'cabin', 'house'] : size === 'city' ? ['house', 'house', 'cabin', 'manor'] : ['house', 'cabin', 'house'];
	let placed = 0;
	outer: for (const off of lines) {
		const cols = Math.max(1, Math.floor((p.blocks * GAP) / HOUSE_SPACING));
		for (let c = 0; c <= cols; c++) {
			for (const sideZ of [-SETBACK, SETBACK]) {
				if (placed >= p.houses) break outer;
				if (rng() < 0.18) continue;
				const hx = cx - half + 4 + c * HOUSE_SPACING + (rng() - 0.5) * 1.4;
				const hz = cz + off + sideZ + (rng() - 0.5) * 1.2;
				const kind = kinds[(rng() * kinds.length) | 0];
				const s = 0.9 + rng() * 0.5;
				objects.push({ id: oid(), kind, pos: [hx, 0, hz], rot: sideZ < 0 ? 0 : 180, scale: [s, s, s], keep: true });
				placed++;
			}
		}
	}
	objects.push({ id: oid(), kind: 'well', pos: [cx, 0, cz], keep: true });
	objects.push({ id: oid(), kind: 'lamp', pos: [cx + 2.5, 0, cz + 2.5], keep: true });
	for (let t = 0; t < p.towers; t++) {
		const corner = t === 0 ? [-half, -half] : [half, half];
		objects.push({ id: oid(), kind: 'tower', pos: [cx + corner[0], 0, cz + corner[1]], scale: [1, 1.3, 1], keep: true });
	}
	for (const ox of lines) for (const oz of lines) {
		if (ox === 0 && oz === 0) continue;
		if (rng() < 0.5) objects.push({ id: oid(), kind: 'lamp', pos: [cx + ox, 0, cz + oz], keep: true });
	}
	if (p.fenced) {
		const R = half + 6;
		const segLen = 1.4;
		const per = 2 * Math.PI * R;
		const segs = Math.max(8, Math.floor(per / segLen));
		for (let i = 0; i < segs; i++) {
			const ang = (i / segs) * Math.PI * 2;
			if (Math.abs(ang) < 0.18 || Math.abs(ang - Math.PI * 2) < 0.18) continue;
			const fx = cx + Math.cos(ang) * R;
			const fz = cz + Math.sin(ang) * R;
			objects.push({ id: oid(), kind: 'fence', pos: [fx, 0, fz], rot: (ang * 180) / Math.PI + 90, keep: true });
		}
	}
	return { objects, paths, radius: half + (p.fenced ? 8 : 4) };
}

const POS_EPS = 1e-9; // fence positions use cos/sin → last-ULP f64 trig diffs across the boundary; everything else is exact

function expectObjEqual(a: WorldObject, b: WorldObject, where: string) {
	expect(a.id, `${where} id`).toBe(b.id);
	expect(a.kind, `${where} kind`).toBe(b.kind);
	for (let k = 0; k < 3; k++) expect(Math.abs(a.pos[k] - b.pos[k]), `${where} pos[${k}]`).toBeLessThan(POS_EPS);
	expect(a.rot ?? 0, `${where} rot`).toBeCloseTo(b.rot ?? 0, 9);
	expect(a.scale?.[0] ?? 1, `${where} scale`).toBeCloseTo(b.scale?.[0] ?? 1, 9);
	expect(a.keep ?? false, `${where} keep`).toBe(b.keep ?? false);
}

describe('world-gen parity (JS reference ↔ Rust port)', () => {
	beforeAll(async () => {
		await math.init();
	});

	it('loaded the wasm (otherwise the guard is vacuous)', () => {
		expect(math.ready).toBe(true);
	});

	const cases: { size: Size; cx: number; cz: number; seed: number }[] = [
		{ size: 'hamlet', cx: 0, cz: 0, seed: 7 },
		{ size: 'village', cx: 160, cz: -240, seed: 1007 },
		{ size: 'town', cx: -80, cz: 400, seed: 2014 },
		{ size: 'city', cx: 640, cz: 240, seed: 99991 }
	];

	// INTEGRATION: the production BINARY settlementPlanner (wgTownPlan → unpack [radius,P,O,...] → rebuild ids/shapes)
	// must reconstruct the SAME town as the reference — ids, kinds, positions, rot, scale, paths, radius.
	for (const { size, cx, cz, seed } of cases) {
		it(`binary settlementPlan matches the reference for a ${size}`, () => {
			const ref = refPlan(cx, cz, size, seed, `t_`);
			const got = binTownPlan(cx, cz, size, seed, `t_`);
			expect(got.objects.length, 'object count').toBe(ref.objects.length);
			expect(got.paths.length, 'path count').toBe(ref.paths.length);
			expect(Math.abs(got.radius - ref.radius), 'radius').toBeLessThan(POS_EPS);
			ref.objects.forEach((o, i) => expectObjEqual(got.objects[i], o, `${size} bin obj#${i}`));
			ref.paths.forEach((pp, i) => expect(got.paths[i].id, `${size} bin path#${i} id`).toBe(pp.id));
		});
	}
});

// ── REFERENCE forest/lake (verbatim from the pre-port city.ts) — the spec the Rust port must match ──
const HASH1 = (i: number) => {
	const v = Math.sin(i * 12.9898 + 4.13) * 43758.5453;
	return v - Math.floor(v);
};
const FOREST_KINDS = ['tree', 'tree', 'pine'];
const isTree = (k: string) => k === 'tree' || k === 'pine';
function refForest(world: World, player: Player): Op[] {
	const ops: Op[] = [];
	const fx = Math.sin(player.yaw), fz = -Math.cos(player.yaw);
	const tx = player.pos[0] + fx * 14, tz = player.pos[2] + fz * 14;
	const near = world.objects.filter((o) => isTree(o.kind) && Math.hypot(o.pos[0] - tx, o.pos[2] - tz) < 40);
	let cx: number, cz: number;
	if (near.length) {
		cx = near.reduce((s, o) => s + o.pos[0], 0) / near.length;
		cz = near.reduce((s, o) => s + o.pos[2], 0) / near.length;
	} else { cx = tx; cz = tz; }
	let innerR = 0;
	for (const o of near) { const d = Math.hypot(o.pos[0] - cx, o.pos[2] - cz); if (d > innerR) innerR = d; }
	const outerR = innerR + (near.length ? 16 : 14);
	const area = Math.PI * (outerR * outerR - innerR * innerR);
	const count = Math.max(8, Math.min(32, Math.round(area / 16)));
	const GA = Math.PI * (3 - Math.sqrt(5));
	for (let i = 0; i < count; i++) {
		const t = (i + 0.5) / count;
		const r = Math.sqrt(innerR * innerR + t * (outerR * outerR - innerR * innerR));
		const a = i * GA + HASH1(i) * 0.6;
		const jr = 1 + (HASH1(i + 99) - 0.5) * 4;
		const x = cx + Math.cos(a) * (r + jr), z = cz + Math.sin(a) * (r + jr);
		if (inWater(world.zones, x, z)) continue;
		const kind = FOREST_KINDS[Math.floor(HASH1(i + 7) * FOREST_KINDS.length)];
		const s = 0.8 + HASH1(i + 31) * 0.7;
		ops.push({ op: 'add', kind, pos: [x, 0, z], scale: [s, s, s], rot: HASH1(i + 51) * 360 });
	}
	return ops;
}
function refLake(world: World, player: Player): Op[] {
	const fx = Math.sin(player.yaw), fz = -Math.cos(player.yaw);
	const tx = player.pos[0] + fx * 18, tz = player.pos[2] + fz * 18;
	let best: { id: string; pos: [number, number, number]; size: number } | null = null;
	let bd = Infinity;
	for (const z of world.zones ?? []) {
		if (z.material !== 'water') continue;
		const d = Math.hypot(z.pos[0] - tx, z.pos[2] - tz);
		if (d < z.size + 16 && d < bd) ((bd = d), (best = z));
	}
	if (best) return [{ op: 'remove', id: best.id }, { op: 'addZone', material: 'water', shape: 'blob', pos: [best.pos[0], 0, best.pos[2]], size: best.size + 6 }];
	return [{ op: 'addZone', material: 'water', shape: 'blob', pos: [tx, 0, tz], size: 13 }];
}

const GEN_EPS = 1e-6; // forest positions/rot run through the GLSL hash (sin × 43758) → looser than the linear settlement math

function mkWorld(objects: WorldObject[] = [], zones: World['zones'] = []): World {
	return { name: 't', seed: 1, objects, zones, paths: [], ground: 'grass', sky: 'day' } as unknown as World;
}
const opAddEq = (g: Op, r: Op, where: string) => {
	if (g.op !== 'add' || r.op !== 'add') { expect(g.op, `${where} op`).toBe(r.op); return; }
	expect(g.kind, `${where} kind`).toBe(r.kind);
	for (let k = 0; k < 3; k++) expect(Math.abs((g.pos![k]) - (r.pos![k])), `${where} pos[${k}]`).toBeLessThan(GEN_EPS);
	expect(g.scale?.[0] ?? 1, `${where} scale`).toBeCloseTo(r.scale?.[0] ?? 1, 6);
	expect(g.rot ?? 0, `${where} rot`).toBeCloseTo(r.rot ?? 0, 4);
};

describe('forest/lake generator parity (JS reference ↔ Rust port)', () => {
	beforeAll(async () => { await math.init(); });

	const worlds: { name: string; world: World; player: Player }[] = [
		{ name: 'fresh wood ahead of player', world: mkWorld(), player: { pos: [10, 0, -5], yaw: 0.9 } },
		{
			name: 'grow an existing wood',
			world: mkWorld([
				{ id: 'a', kind: 'tree', pos: [0, 0, -14] }, { id: 'b', kind: 'tree', pos: [3, 0, -12] }, { id: 'c', kind: 'pine', pos: [-2, 0, -16] }
			] as WorldObject[]),
			player: { pos: [0, 0, 0], yaw: 0 }
		},
		{
			name: 'wood beside a lake (water cull)',
			world: mkWorld(
				[{ id: 'a', kind: 'tree', pos: [0, 0, -14] }] as WorldObject[],
				[{ id: 'z0', material: 'water', pos: [0, 0, -14], size: 10 }] as World['zones']
			),
			player: { pos: [0, 0, 0], yaw: 0 }
		}
	];

	// INTEGRATION: the production binary city.ts wrappers (seed store → wgForest/wgLake → decodeGenOps) must produce
	// the SAME engine Op[] as the JS REFERENCE algorithm — proves the whole no-JSON wiring end-to-end (real wasm, in node).
	it('binary city.forestOps matches the reference forest path', () => {
		for (const { name, world, player } of worlds) {
			const ref = refForest(world, player);
			const got = binForestOps(world, player);
			expect(got.length, `${name} count`).toBe(ref.length);
			ref.forEach((r, i) => opAddEq(got[i], r, `${name} bin #${i}`));
		}
	});

	it('binary city.lakeOps matches the reference lake path (grow + fresh)', () => {
		const withLake = mkWorld([], [{ id: 'z3', material: 'water', pos: [5, 0, -10], size: 8 }] as World['zones']);
		const p2: Player = { pos: [0, 0, 0], yaw: 0 };
		const ref = refLake(withLake, p2);
		const got = binLakeOps(withLake, p2);
		expect(got.length).toBe(ref.length);
		expect(got[0]).toEqual(ref[0]); // REMOVE: the slot→id mapping recovers the same zone id 'z3'
		const a = got[1] as Extract<Op, { op: 'addZone' }>, b = ref[1] as Extract<Op, { op: 'addZone' }>;
		expect(a.material).toBe(b.material);
		expect(a.size).toBeCloseTo(b.size!, 9);
		for (let k = 0; k < 3; k++) expect(a.pos![k]).toBeCloseTo(b.pos![k], 9);

		const fresh = mkWorld();
		const pf: Player = { pos: [5, 0, 5], yaw: 1.2 };
		expect(binLakeOps(fresh, pf).length).toBe(refLake(fresh, pf).length);
	});
});

// ── REFERENCE city (verbatim from the pre-port city.ts cityOps) — the spec the Rust port must match ──
const BUILDINGS = ['house', 'cabin', 'tower'];
const WALL_TONES = ['#d2b48c', '#c9a978', '#be9d72', '#cdb389', '#b89a86', '#c2a15f', '#a98c63'];
const STONE_TONES = ['#b7b2a8', '#adb0b3', '#c1bcb0', '#a8a59c'];
const DISTRICTS = [
	{ towerChance: 0.3, h: [1.5, 2.2], w: [1.0, 1.25], tones: STONE_TONES },
	{ towerChance: 0.1, h: [1.1, 1.6], w: [0.95, 1.2], tones: WALL_TONES },
	{ towerChance: 0.03, h: [0.85, 1.15], w: [0.85, 1.05], tones: WALL_TONES }
];
const TAU = Math.PI * 2;
const districtFor = (ring: number) => DISTRICTS[Math.min(ring, DISTRICTS.length - 1)];
const lerp = (r: number[], t: number) => r[0] + (r[1] - r[0]) * t;
const isBuilding = (k: string) => k === 'house' || k === 'cabin' || k === 'tower';
function refCity(world: World, player: Player): Op[] {
	const ops: Op[] = [];
	const fx = Math.sin(player.yaw), fz = -Math.cos(player.yaw);
	const tx = player.pos[0] + fx * 16, tz = player.pos[2] + fz * 16;
	const near = world.objects.filter((o) => isBuilding(o.kind) && Math.hypot(o.pos[0] - tx, o.pos[2] - tz) < 45);
	let cx: number, cz: number;
	if (near.length) {
		cx = near.reduce((s, o) => s + o.pos[0], 0) / near.length;
		cz = near.reduce((s, o) => s + o.pos[2], 0) / near.length;
	} else { cx = Math.round(tx / 2) * 2; cz = Math.round(tz / 2) * 2; }
	let maxR = 0;
	for (const o of near) { const d = Math.hypot(o.pos[0] - cx, o.pos[2] - cz); if (d > maxR) maxR = d; }
	const RING_GAP = 16;
	const ringR = near.length ? maxR + RING_GAP : 16;
	const ring = near.length ? Math.round(maxR / RING_GAP) : 0;
	const SPOKES = 6, ROAD_W = 4;
	const edge = ringR + 8;
	for (const p of world.paths ?? []) if (Math.hypot(p.from[0] - cx, p.from[2] - cz) < 6) ops.push({ op: 'remove', id: p.id });
	for (const z of world.zones ?? []) if (z.material === 'plaza' && Math.hypot(z.pos[0] - cx, z.pos[2] - cz) < 10) ops.push({ op: 'remove', id: z.id });
	if (!inWater(world.zones, cx, cz)) ops.push({ op: 'addZone', material: 'plaza', shape: 'rect', pos: [cx, 0, cz], size: Math.min(15, 6 + ring * 2) });
	const spokeAng: number[] = [];
	for (let s = 0; s < SPOKES; s++) {
		const ang = (s / SPOKES) * TAU + 0.26;
		spokeAng.push(ang);
		ops.push({ op: 'addPath', material: 'path', fromPos: [cx, 0, cz], toPos: [cx + Math.cos(ang) * edge, 0, cz + Math.sin(ang) * edge], width: ROAD_W });
		const off = ROAD_W / 2 + 0.6;
		const lx = cx + Math.cos(ang) * ringR - Math.sin(ang) * off;
		const lz = cz + Math.sin(ang) * ringR + Math.cos(ang) * off;
		if (!inWater(world.zones, lx, lz)) ops.push({ op: 'add', kind: 'lamp', pos: [lx, 0, lz] });
	}
	const spacing = 13 + ring * 3;
	const count = Math.max(5, Math.min(30, Math.round((TAU * ringR) / spacing)));
	const district = districtFor(ring);
	const clearAng = Math.min(0.26, (ROAD_W / 2 + 2) / ringR);
	const SECTOR = TAU / SPOKES;
	for (let i = 0; i < count; i++) {
		const a = (i / count) * TAU + ring * 0.4 + 0.13;
		let onRoad = false;
		for (const sa of spokeAng) { const da = Math.abs(((((a - sa) % TAU) + TAU + Math.PI) % TAU) - Math.PI); if (da < clearAng) onRoad = true; }
		if (onRoad) continue;
		const jr = ringR + (HASH1(ring * 31 + i * 7) - 0.5) * RING_GAP * 0.4;
		const x = cx + Math.cos(a) * jr, z = cz + Math.sin(a) * jr;
		if (inWater(world.zones, x, z)) continue;
		const sector = Math.floor(((((a - 0.26) % TAU) + TAU) % TAU) / SECTOR);
		const bSeed = ring * 23 + sector * 7;
		const towerBlock = HASH1(bSeed + 11) < district.towerChance;
		const blockTone = district.tones[Math.floor(HASH1(bSeed + 3) * district.tones.length)];
		const wBase = lerp(district.w, HASH1(bSeed + 5));
		const hBase = lerp(district.h, HASH1(bSeed + 7));
		const seed = i + ring * 17;
		const kind = towerBlock ? 'tower' : BUILDINGS[i % 2];
		const wide = wBase * (0.92 + HASH1(seed) * 0.16);
		const tall = hBase * (0.9 + HASH1(seed + 5) * 0.2);
		const rotDeg = (Math.atan2(cx - x, cz - z) * 180) / Math.PI + (HASH1(seed + 9) - 0.5) * 16;
		const color = kind === 'tower' ? undefined : blockTone;
		ops.push({ op: 'add', kind, pos: [x, 0, z], rot: rotDeg, scale: [wide, tall, wide], color });
	}
	return ops;
}

// generic op comparator (covers remove / addZone / addPath / add) with an epsilon on hash-driven coords
function opEq(g: Op, r: Op, where: string) {
	expect(g.op, `${where} op`).toBe(r.op);
	if (r.op === 'remove' && g.op === 'remove') { expect(g.id, `${where} id`).toBe(r.id); return; }
	if (r.op === 'addZone' && g.op === 'addZone') {
		expect(g.material).toBe(r.material); expect(g.shape).toBe(r.shape);
		for (let k = 0; k < 3; k++) expect(Math.abs(g.pos![k] - r.pos![k]), `${where} pos[${k}]`).toBeLessThan(GEN_EPS);
		expect(g.size!, `${where} size`).toBeCloseTo(r.size!, 6); return;
	}
	if (r.op === 'addPath' && g.op === 'addPath') {
		expect(g.material).toBe(r.material); expect(g.width).toBe(r.width);
		for (let k = 0; k < 3; k++) {
			expect(Math.abs(g.fromPos![k] - r.fromPos![k]), `${where} from[${k}]`).toBeLessThan(GEN_EPS);
			expect(Math.abs(g.toPos![k] - r.toPos![k]), `${where} to[${k}]`).toBeLessThan(GEN_EPS);
		}
		return;
	}
	if (r.op === 'add' && g.op === 'add') {
		opAddEq(g, r, where);
		expect(g.color, `${where} color`).toBe(r.color);
	}
}

describe('city generator parity (JS reference ↔ Rust port)', () => {
	beforeAll(async () => { await math.init(); });

	const bldg = (id: string, kind: string, x: number, z: number) => ({ id, kind, pos: [x, 0, z] }) as WorldObject;
	const cluster = (cx: number, cz: number) =>
		[bldg('a', 'house', cx - 8, cz), bldg('b', 'cabin', cx + 7, cz - 3), bldg('c', 'house', cx, cz + 9), bldg('d', 'tower', cx + 4, cz + 6), bldg('e', 'cabin', cx - 5, cz - 7), bldg('f', 'house', cx + 10, cz + 2)];

	const cases: { name: string; world: World; player: Player }[] = [
		{ name: 'fresh city, positive coords', world: mkWorld(), player: { pos: [20, 0, 30], yaw: 0.5 } },
		{ name: 'fresh city, NEGATIVE coords (js_round trap)', world: mkWorld(), player: { pos: [-7, 0, -13], yaw: 2.3 } },
		{
			name: 'grow existing city + remove old plaza & spokes',
			world: mkWorld(
				cluster(0, -16),
				[{ id: 'z0', material: 'plaza', pos: [0, 0, -16], size: 6 }] as World['zones']
			),
			player: { pos: [0, 0, 0], yaw: 0 }
		},
		{
			name: 'city beside water (cull)',
			world: mkWorld(cluster(0, -16), [{ id: 'z0', material: 'water', pos: [10, 0, -16], size: 14 }] as World['zones']),
			player: { pos: [0, 0, 0], yaw: 0 }
		}
	];

	// INTEGRATION: the production binary city.ts wrapper (seed store → wgCity → decodeGenOps, water + removables as
	// typed arrays) must produce the SAME engine Op[] as the JS REFERENCE algorithm — buildings, lamps, plaza, spokes, removes.
	it('binary city.cityOps matches the reference city path', () => {
		for (const { name, world, player } of cases) {
			if (name.startsWith('grow')) world.paths = [{ id: 'p9', material: 'path', from: [0, 0, -16], to: [40, 0, -16], width: 4 }] as World['paths'];
			const ref = refCity(world, player);
			const got = binCityOps(world, player);
			expect(got.length, `${name} bin count`).toBe(ref.length);
			ref.forEach((r, i) => opEq(got[i], r, `${name} bin #${i}`));
		}
	});
});
