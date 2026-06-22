import type { WorldObject, Path } from '$lib/world';

// PROCEDURAL SETTLEMENT PLANNER (docs/ideas-queue EF11). Given a centre, a SIZE tier and a seed, lay out a
// PLANNED town — a street grid, houses lining the roads (set back + facing the street), a central well/plaza, a
// watchtower or two, and a perimeter fence with a gate on the bigger ones. Deterministic (seeded), so a settlement
// is stable + every seed differs. The emergent sim will later SNAP a matured house-cluster onto one of these
// (tag everything `keep` so habitation-decay leaves planned towns alone); for now it also drives a demo gallery.

export type SettlementSize = 'hamlet' | 'village' | 'town' | 'city';
export const SIZES: SettlementSize[] = ['hamlet', 'village', 'town', 'city'];

// blocks-per-side of the street grid, and roughly how many houses to place, per tier
const PLAN: Record<SettlementSize, { blocks: number; houses: number; towers: number; fenced: boolean }> = {
	hamlet: { blocks: 1, houses: 4, towers: 0, fenced: false },
	village: { blocks: 2, houses: 10, towers: 1, fenced: false },
	town: { blocks: 3, houses: 20, towers: 1, fenced: true },
	city: { blocks: 4, houses: 34, towers: 2, fenced: true }
};

const GAP = 18; // metres between parallel streets (one block)
const SETBACK = 6; // how far houses sit back from the road they face
const HOUSE_SPACING = 7.5; // spacing of houses along a street

/** mulberry32 — a tiny deterministic PRNG so a (centre, seed) always plans the SAME town. */
function prng(seed: number): () => number {
	let a = seed >>> 0;
	return () => {
		a = (a + 0x6d2b79f5) | 0;
		let t = Math.imul(a ^ (a >>> 15), 1 | a);
		t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
		return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
	};
}

/** Plan one settlement. Returns the world-objects (houses/well/towers/fence/lamps) + the road Paths. */
export function settlementPlan(cx: number, cz: number, size: SettlementSize, seed: number, idPrefix: string): { objects: WorldObject[]; paths: Path[]; radius: number } {
	const rng = prng(seed);
	const p = PLAN[size];
	const objects: WorldObject[] = [];
	const paths: Path[] = [];
	let n = 0;
	const oid = () => `${idPrefix}o${n++}`;
	const pid = () => `${idPrefix}p${n++}`;
	const half = (p.blocks * GAP) / 2; // half the grid extent
	const lines: number[] = []; // street offsets from centre, both axes share this set
	for (let i = 0; i <= p.blocks; i++) lines.push(-half + i * GAP);

	// ── STREET GRID (roads) — a Path along every grid line, both axes; the perimeter ones double as the ring road
	for (const off of lines) {
		paths.push({ id: pid(), material: 'path', from: [cx - half, 0, cz + off], to: [cx + half, 0, cz + off], width: 3 }); // E–W (the road shader)
		paths.push({ id: pid(), material: 'path', from: [cx + off, 0, cz - half], to: [cx + off, 0, cz + half], width: 3 }); // N–S
	}

	// ── HOUSES line the E–W streets, set back on BOTH sides, facing the road; varied kind/scale, seeded jitter
	const kinds = size === 'hamlet' ? ['cabin', 'cabin', 'house'] : size === 'city' ? ['house', 'house', 'cabin', 'manor'] : ['house', 'cabin', 'house'];
	let placed = 0;
	outer: for (const off of lines) {
		const cols = Math.max(1, Math.floor((p.blocks * GAP) / HOUSE_SPACING));
		for (let c = 0; c <= cols; c++) {
			for (const sideZ of [-SETBACK, SETBACK]) {
				if (placed >= p.houses) break outer;
				if (rng() < 0.18) continue; // a few empty plots → not a rigid wall of houses
				const hx = cx - half + 4 + c * HOUSE_SPACING + (rng() - 0.5) * 1.4;
				const hz = cz + off + sideZ + (rng() - 0.5) * 1.2;
				const kind = kinds[(rng() * kinds.length) | 0];
				const s = 0.9 + rng() * 0.5;
				objects.push({ id: oid(), kind, pos: [hx, 0, hz], rot: sideZ < 0 ? 0 : 180, scale: [s, s, s], keep: true });
				placed++;
			}
		}
	}

	// ── CENTRAL PLAZA: a well at the crossroads, a lamp beside it
	objects.push({ id: oid(), kind: 'well', pos: [cx, 0, cz], keep: true });
	objects.push({ id: oid(), kind: 'lamp', pos: [cx + 2.5, 0, cz + 2.5], keep: true });

	// ── WATCHTOWER(S) at corners (a town's lookout), scaled tall
	for (let t = 0; t < p.towers; t++) {
		const corner = t === 0 ? [-half, -half] : [half, half];
		objects.push({ id: oid(), kind: 'tower', pos: [cx + corner[0], 0, cz + corner[1]], scale: [1, 1.3, 1], keep: true });
	}

	// ── LAMPS at the street intersections (skip the very centre, the well's there)
	for (const ox of lines) for (const oz of lines) {
		if (ox === 0 && oz === 0) continue;
		if (rng() < 0.5) objects.push({ id: oid(), kind: 'lamp', pos: [cx + ox, 0, cz + oz], keep: true });
	}

	// ── PERIMETER FENCE (town/city) — a ring of fence segments just outside the grid, with a GATE gap on the +X road
	if (p.fenced) {
		const R = half + 6;
		const segLen = 1.4; // one fence prop's span
		const per = 2 * Math.PI * R;
		const segs = Math.max(8, Math.floor(per / segLen));
		for (let i = 0; i < segs; i++) {
			const ang = (i / segs) * Math.PI * 2;
			if (Math.abs(ang) < 0.18 || Math.abs(ang - Math.PI * 2) < 0.18) continue; // gate gap where the +X road exits
			const fx = cx + Math.cos(ang) * R;
			const fz = cz + Math.sin(ang) * R;
			objects.push({ id: oid(), kind: 'fence', pos: [fx, 0, fz], rot: (ang * 180) / Math.PI + 90, keep: true });
		}
	}

	return { objects, paths, radius: half + (p.fenced ? 8 : 4) };
}
