// Native procedural CITY generator — the "make city" command. Deterministic (no LLM): each call adds a
// concentric RING of inward-facing buildings + street lamps around the city centre, so repeating "make
// city" grows the SAME city outward, bigger and bigger. The first call also lays a central plaza and a
// crossroads. Emits engine Ops (add / addZone / addPath with explicit coords), so it's collision-resolved,
// undoable and shareable like any other build. See [[architecture-ops-not-geometry]].
import type { Op } from './engine';
import type { World, Player } from './world';
import { inWater } from './water';

const BUILDINGS = ['house', 'cabin', 'tower'];
// warm earthy wall tones → houses/cabins aren't all the same beige (towers keep their stone)
const WALL_TONES = ['#d2b48c', '#c9a978', '#be9d72', '#cdb389', '#b89a86', '#c2a15f', '#a98c63'];
const STONE_TONES = ['#b7b2a8', '#adb0b3', '#c1bcb0', '#a8a59c']; // cool stone for the downtown core
const TAU = Math.PI * 2;

// City DISTRICTS — a loose template per concentric ring so the city reads as DESIGNED, not random noise: a
// dense downtown CORE of stone towers at the centre (built first), a mixed mid-rise belt, then low warm
// residential streets outward (added as it grows). A building varies only LOOSELY around its district +
// block baseline rather than every house being independently random. Index = clamp(ringIndex).
const DISTRICTS: { towerChance: number; h: [number, number]; w: [number, number]; tones: string[] }[] = [
	{ towerChance: 0.3, h: [1.5, 2.2], w: [1.0, 1.25], tones: STONE_TONES }, // core — a FEW landmark towers, not a huddle
	{ towerChance: 0.1, h: [1.1, 1.6], w: [0.95, 1.2], tones: WALL_TONES }, // midtown — mostly mid-rise, the odd tower
	{ towerChance: 0.03, h: [0.85, 1.15], w: [0.85, 1.05], tones: WALL_TONES } // residential — low houses, towers very rare
];
const districtFor = (ring: number) => DISTRICTS[Math.min(ring, DISTRICTS.length - 1)];
const lerp = (r: [number, number], t: number) => r[0] + (r[1] - r[0]) * t;

/** Is this object part of a city (a building we cluster around)? */
const isBuilding = (kind: string) => kind === 'house' || kind === 'cabin' || kind === 'tower';

/**
 * Ops that build (or grow) a city. Centre = the existing buildings' centroid if any (so it grows the
 * current city wherever you stand), else a spot in front of the player. Adds the next ring outward.
 */
export function cityOps(world: World, player: Player): Op[] {
	const ops: Op[] = [];
	// build AT the player (a spot just ahead) — only GROW an existing city if there's a cluster RIGHT HERE
	// (so the demo's origin buildings don't drag every "make city" back to 0,0)
	const fx = Math.sin(player.yaw);
	const fz = -Math.cos(player.yaw);
	const tx = player.pos[0] + fx * 16;
	const tz = player.pos[2] + fz * 16;
	const near = world.objects.filter((o) => isBuilding(o.kind) && Math.hypot(o.pos[0] - tx, o.pos[2] - tz) < 45);

	let cx: number;
	let cz: number;
	if (near.length) {
		cx = near.reduce((s, o) => s + o.pos[0], 0) / near.length; // grow the city you're standing in
		cz = near.reduce((s, o) => s + o.pos[2], 0) / near.length;
	} else {
		cx = Math.round(tx / 2) * 2; // a fresh city, here in front of you
		cz = Math.round(tz / 2) * 2;
	}

	// current extent of THIS city → the new ring sits just beyond it
	let maxR = 0;
	for (const o of near) {
		const d = Math.hypot(o.pos[0] - cx, o.pos[2] - cz);
		if (d > maxR) maxR = d;
	}
	const RING_GAP = 16; // wider gap between rings → the city spreads out as it grows, not packs denser
	const ringR = near.length ? maxR + RING_GAP : 16;
	const ring = near.length ? Math.round(maxR / RING_GAP) : 0; // how many rings out → district + sprawl + plaza size

	const SPOKES = 6;
	const ROAD_W = 4;
	const edge = ringR + 8; // the radial roads reach just past the outer ring of buildings

	// RE-LAY the road network so it always reaches the rim as the city grows: remove the old city spokes
	// (paths that start at the centre) and add fresh ones from the plaza out to the current edge. User-drawn
	// roads (which don't start at the centre) are untouched.
	for (const p of world.paths ?? []) {
		if (Math.hypot(p.from[0] - cx, p.from[2] - cz) < 6) ops.push({ op: 'remove', id: p.id });
	}
	// (re)lay the central plaza, GROWING it with the city — a big city earns a big downtown square, not a 6 m patch
	for (const z of world.zones ?? []) {
		if (z.material === 'plaza' && Math.hypot(z.pos[0] - cx, z.pos[2] - cz) < 10) ops.push({ op: 'remove', id: z.id });
	}
	if (!inWater(world.zones, cx, cz)) {
		ops.push({ op: 'addZone', material: 'plaza', shape: 'rect', pos: [cx, 0, cz], size: Math.min(15, 6 + ring * 2) });
	}
	const spokeAng: number[] = [];
	for (let s = 0; s < SPOKES; s++) {
		const ang = (s / SPOKES) * TAU + 0.26; // off the cardinal axes
		spokeAng.push(ang);
		ops.push({ op: 'addPath', material: 'path', fromPos: [cx, 0, cz], toPos: [cx + Math.cos(ang) * edge, 0, cz + Math.sin(ang) * edge], width: ROAD_W });
		// a street lamp BESIDE each spoke (offset perpendicular) at this ring's radius
		const off = ROAD_W / 2 + 0.6;
		const lx = cx + Math.cos(ang) * ringR - Math.sin(ang) * off;
		const lz = cz + Math.sin(ang) * ringR + Math.cos(ang) * off;
		if (!inWater(world.zones, lx, lz)) ops.push({ op: 'add', kind: 'lamp', pos: [lx, 0, lz] });
	}

	// buildings on the new ring, placed in the BLOCKS between spokes (never on a road), each FACING the plaza.
	// Their look follows this ring's DISTRICT template (core/mid/residential), so the skyline is coherent.
	// outer rings SPRAWL — buildings spaced further apart the further out you go (downtown denser, suburbs airy),
	// so each "make city" expands the footprint and spreads out more, rather than cramming the same area.
	const spacing = 13 + ring * 3;
	const count = Math.max(5, Math.min(30, Math.round((TAU * ringR) / spacing)));
	const district = districtFor(ring);
	const clearAng = Math.min(0.26, (ROAD_W / 2 + 2) / ringR); // angular gap kept clear around each spoke
	const SECTOR = TAU / SPOKES; // one city block = the wedge between two spokes
	for (let i = 0; i < count; i++) {
		const a = (i / count) * TAU + ring * 0.4 + 0.13;
		let onRoad = false;
		for (const sa of spokeAng) {
			const da = Math.abs(((((a - sa) % TAU) + TAU + Math.PI) % TAU) - Math.PI); // shortest angular dist
			if (da < clearAng) ((onRoad = true), 0);
		}
		if (onRoad) continue; // would sit on a road → skip (leaves the street clear)
		// slight radial scatter so a block isn't a mathematically perfect arc → reads organic, not stamped.
		// Stays well within the ring gap (no overlap with the next ring) and the angle is unchanged (still off roads).
		const jr = ringR + (hash1(ring * 31 + i * 7) - 0.5) * RING_GAP * 0.4;
		const x = cx + Math.cos(a) * jr;
		const z = cz + Math.sin(a) * jr;
		if (inWater(world.zones, x, z)) continue; // never build on a lake

		// BLOCK coherence: everything in the same wedge of the same ring shares a style — a tower block vs a
		// low-rise street, ONE wall-tone family, a common height baseline — so a street reads as a real
		// neighbourhood instead of a jumble. Individual buildings then vary only loosely around that.
		const sector = Math.floor((((a - 0.26) % TAU) + TAU) % TAU / SECTOR);
		const bSeed = ring * 23 + sector * 7;
		const towerBlock = hash1(bSeed + 11) < district.towerChance;
		const blockTone = district.tones[Math.floor(hash1(bSeed + 3) * district.tones.length)];
		const wBase = lerp(district.w, hash1(bSeed + 5)); // this block's footprint baseline
		const hBase = lerp(district.h, hash1(bSeed + 7)); // this block's height baseline → even rooflines per street

		const seed = i + ring * 17;
		const kind = towerBlock ? 'tower' : BUILDINGS[i % 2]; // tower block → towers; else alternating houses/cabins
		const wide = wBase * (0.92 + hash1(seed) * 0.16); // ±8% loose footprint variation around the block
		const tall = hBase * (0.9 + hash1(seed + 5) * 0.2); // ±10% loose height around the block baseline
		const rotDeg = (Math.atan2(cx - x, cz - z) * 180) / Math.PI + (hash1(seed + 9) - 0.5) * 16; // face plaza, ±8°
		const color = kind === 'tower' ? undefined : blockTone; // towers keep stone; a block shares one wall tone
		ops.push({ op: 'add', kind, pos: [x, 0, z], rot: rotDeg, scale: [wide, tall, wide], color });
	}
	return ops;
}

/** Does this typed instruction mean "make/grow a city"? (handled natively, not by the LLM). */
export function isCityCommand(cmd: string): boolean {
	return /^(make|build|grow|add|create|generate|bigger|expand)?\s*(me\s+)?(a\s+|the\s+|my\s+)?(big(ger)?\s+|huge\s+)?(city|town|village)$/.test(cmd);
}

// ── Forest (same growth pattern as the city) ───────────────────────────────────────────────────────
const FOREST_KINDS = ['tree', 'tree', 'pine']; // ~2:1 broadleaf : pine
const isTree = (kind: string) => kind === 'tree' || kind === 'pine';
const hash1 = (i: number) => {
	const v = Math.sin(i * 12.9898 + 4.13) * 43758.5453;
	return v - Math.floor(v);
};

/**
 * Ops that plant (or grow) a forest. Like `make city`: centre = existing trees' centroid (grows the same
 * wood), else ahead of the player. Each call fills the next annulus outward with naturally-jittered trees,
 * so repeating "make forest" thickens and widens it. Even area-fill via the golden angle + sqrt radius.
 */
export function forestOps(world: World, player: Player): Op[] {
	const ops: Op[] = [];
	// plant AT the player; only grow a wood that's RIGHT HERE (don't drag back to the demo trees at origin)
	const fx = Math.sin(player.yaw);
	const fz = -Math.cos(player.yaw);
	const tx = player.pos[0] + fx * 14;
	const tz = player.pos[2] + fz * 14;
	const near = world.objects.filter((o) => isTree(o.kind) && Math.hypot(o.pos[0] - tx, o.pos[2] - tz) < 40);

	let cx: number;
	let cz: number;
	if (near.length) {
		cx = near.reduce((s, o) => s + o.pos[0], 0) / near.length;
		cz = near.reduce((s, o) => s + o.pos[2], 0) / near.length;
	} else {
		cx = tx;
		cz = tz;
	}

	let innerR = 0;
	for (const o of near) {
		const d = Math.hypot(o.pos[0] - cx, o.pos[2] - cz);
		if (d > innerR) innerR = d;
	}
	const outerR = innerR + (near.length ? 16 : 14);

	// even area-fill of the annulus [innerR, outerR], capped so one call isn't a huge hang
	const area = Math.PI * (outerR * outerR - innerR * innerR);
	const count = Math.max(8, Math.min(32, Math.round(area / 16)));
	const GA = Math.PI * (3 - Math.sqrt(5)); // golden angle → even spread
	for (let i = 0; i < count; i++) {
		const t = (i + 0.5) / count;
		const r = Math.sqrt(innerR * innerR + t * (outerR * outerR - innerR * innerR)); // area-uniform radius
		const a = i * GA + hash1(i) * 0.6; // golden spiral + a little jitter → natural, not gridded
		const jr = 1 + (hash1(i + 99) - 0.5) * 4; // radial wobble
		const x = cx + Math.cos(a) * (r + jr);
		const z = cz + Math.sin(a) * (r + jr);
		if (inWater(world.zones, x, z)) continue; // trees don't grow in the lake
		const kind = FOREST_KINDS[Math.floor(hash1(i + 7) * FOREST_KINDS.length)];
		const s = 0.8 + hash1(i + 31) * 0.7; // size variety
		ops.push({ op: 'add', kind, pos: [x, 0, z], scale: [s, s, s], rot: hash1(i + 51) * 360 });
	}
	return ops;
}

/** Does this typed instruction mean "make/grow a forest"? (handled natively, not by the LLM). */
export function isForestCommand(cmd: string): boolean {
	return /^(make|build|grow|add|create|generate|plant|bigger|expand)?\s*(me\s+)?(a\s+|the\s+|my\s+)?(big(ger)?\s+|huge\s+|dense\s+)?(forest|woods?|jungle)$/.test(cmd);
}

// ── Lake (the third native generator; relies on remove handling zone ids) ──────────────────────────
/**
 * Ops to dig (or enlarge) a lake. Like city/forest: a fresh organic pond ahead of you, OR — if you're at
 * an existing lake — it removes that water zone and re-adds a bigger one centred the same, so repeating
 * "make lake" grows it. The shader carves the organic blob shoreline; addZone keeps it off objects.
 */
export function lakeOps(world: World, player: Player): Op[] {
	const fx = Math.sin(player.yaw);
	const fz = -Math.cos(player.yaw);
	const tx = player.pos[0] + fx * 18;
	const tz = player.pos[2] + fz * 18;

	let best: { id: string; pos: [number, number, number]; size: number } | null = null;
	let bd = Infinity;
	for (const z of world.zones ?? []) {
		if (z.material !== 'water') continue;
		const d = Math.hypot(z.pos[0] - tx, z.pos[2] - tz);
		if (d < z.size + 16 && d < bd) ((bd = d), (best = z));
	}
	if (best) {
		return [
			{ op: 'remove', id: best.id }, // grow in place: drop the old zone, lay a bigger one
			{ op: 'addZone', material: 'water', shape: 'blob', pos: [best.pos[0], 0, best.pos[2]], size: best.size + 6 }
		];
	}
	return [{ op: 'addZone', material: 'water', shape: 'blob', pos: [tx, 0, tz], size: 13 }];
}

/** Does this typed instruction mean "make/grow a lake"? (handled natively, not by the LLM). */
export function isLakeCommand(cmd: string): boolean {
	return /^(make|build|dig|grow|add|create|generate|bigger|expand)?\s*(me\s+)?(a\s+|the\s+|my\s+)?(big(ger)?\s+|huge\s+)?(lake|pond)$/.test(cmd);
}
