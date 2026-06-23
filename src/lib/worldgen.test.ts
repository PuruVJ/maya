// PARITY GUARD for the JS→Rust world-gen port. The procedural generators (settlement planner; later city/forest/
// lake) moved into Rust (crates/worldsim/src/worldgen.rs) so Rust owns all world-gen compute. This pins the Rust
// output to the original JS algorithm so the port can't silently change a generated town. The reference JS impl is
// embedded here as the spec-of-record (the production copy is now a thin wasm bridge), so the guard stays meaningful
// after the production JS is reduced to delegation.
import { describe, it, expect, beforeAll } from 'vitest';
import { math } from './math';
import type { WorldObject, Path } from './world';

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
