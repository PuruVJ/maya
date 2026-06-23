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

	for (const { size, cx, cz, seed } of cases) {
		it(`settlement_plan matches the reference for a ${size}`, () => {
			const ref = refPlan(cx, cz, size, seed, `t_`);
			const got = math.settlementPlan(cx, cz, size, seed, `t_`);
			expect(got, 'wasm settlementPlan').not.toBeNull();
			if (!got) return;
			expect(got.objects.length, 'object count').toBe(ref.objects.length);
			expect(got.paths.length, 'path count').toBe(ref.paths.length);
			expect(Math.abs(got.radius - ref.radius), 'radius').toBeLessThan(POS_EPS);
			ref.objects.forEach((o, i) => expectObjEqual(got.objects[i], o, `${size} obj#${i}`));
			ref.paths.forEach((pp, i) => {
				const g = got.paths[i];
				expect(g.id, `${size} path#${i} id`).toBe(pp.id);
				for (let k = 0; k < 3; k++) {
					expect(Math.abs(g.from[k] - pp.from[k]), `${size} path#${i} from[${k}]`).toBeLessThan(POS_EPS);
					expect(Math.abs(g.to[k] - pp.to[k]), `${size} path#${i} to[${k}]`).toBeLessThan(POS_EPS);
				}
			});
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
	expect(g.scale![0], `${where} scale`).toBeCloseTo(r.scale![0], 6);
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

	for (const { name, world, player } of worlds) {
		it(`forest_ops matches the reference — ${name}`, () => {
			const ref = refForest(world, player);
			const got = math.forestOps(JSON.stringify(world), player.pos[0], player.pos[2], player.yaw);
			expect(got, 'wasm forestOps').not.toBeNull();
			if (!got) return;
			expect(got.length, `${name} op count`).toBe(ref.length);
			ref.forEach((r, i) => opAddEq(got[i], r, `${name} #${i}`));
		});
	}

	it('lake_ops matches the reference — fresh + grow', () => {
		const fresh = mkWorld();
		const p: Player = { pos: [5, 0, 5], yaw: 1.2 };
		expect(math.lakeOps(JSON.stringify(fresh), p.pos[0], p.pos[2], p.yaw)).toEqual(refLake(fresh, p));

		const withLake = mkWorld([], [{ id: 'z3', material: 'water', pos: [5, 0, -10], size: 8 }] as World['zones']);
		const p2: Player = { pos: [0, 0, 0], yaw: 0 };
		const got = math.lakeOps(JSON.stringify(withLake), p2.pos[0], p2.pos[2], p2.yaw);
		const ref = refLake(withLake, p2);
		expect(got, 'grow-lake ops').not.toBeNull();
		if (!got) return;
		expect(got.length).toBe(ref.length);
		expect(got[0]).toEqual(ref[0]); // remove the same id
		// addZone: compare fields with epsilon on the size/pos
		const a = got[1] as Extract<Op, { op: 'addZone' }>, b = ref[1] as Extract<Op, { op: 'addZone' }>;
		expect(a.material).toBe(b.material);
		expect(a.size).toBeCloseTo(b.size!, 9);
	});
});
