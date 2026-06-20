// Deterministic world-state + op application. The LLM emits intent + a symbolic anchor;
// THIS resolves exact, collision-free geometry. Pure & deterministic: same ops in the same
// order → same layout → same shareable string. See docs/robust-pipeline.md.
import { kindDef } from './kinds';
import { heightAt } from './terrain';
import { inWater } from './water';
import type { World, WorldObject, Player } from './world';

export type Op =
	| { op: 'add'; kind: string; count?: number; pos?: [number, number, number]; at?: string; dist?: number; scale?: [number, number, number]; color?: string; rot?: number }
	| { op: 'scatter'; kind: string; count: number; area: string; color?: string }
	| { op: 'remove'; id: string }
	| { op: 'move'; id: string; pos?: [number, number, number]; at?: string; dist?: number }
	| { op: 'paint'; id: string; color: string }
	| { op: 'setGround'; value: string }
	| { op: 'setSky'; value: string }
	// scenery ops — stored for the renderer to interpret later (not applied to objects yet)
	| { op: 'addZone'; material: string; shape: string; at?: string; pos?: [number, number, number]; size?: number }
	| { op: 'addPath'; material: string; from?: string; to?: string; fromPos?: [number, number, number]; toPos?: [number, number, number]; width?: number }
	| { op: 'setTerrain'; preset: string; amplitude?: number }
	// pure UI feedback — the model emits this to tell the user a limit; the engine ignores it
	| { op: 'note'; text: string };

// reported by applyOps when a big-footprint op (lake/terrain) couldn't find fully clear space
export interface PlacementConflict {
	label: string;
	blockers: string[]; // object ids sitting under the placed area
}

type Vec3 = [number, number, number];

const TAU = Math.PI * 2;
// safety cap so "add 9999 cats" can't lock up the renderer (each count-N op is clamped to this)
const MAX_COUNT = 120;

const kr = (k: string) => kindDef(k).r;
const dist2 = (a: Vec3, b: Vec3) => {
	const dx = a[0] - b[0], dz = a[2] - b[2];
	return dx * dx + dz * dz;
};
const snap = (v: number, g = 0.5) => Math.round(v / g) * g;
const forward = (yaw: number): Vec3 => [Math.sin(yaw), 0, -Math.cos(yaw)];

const nearest = (list: WorldObject[], p: Vec3): WorldObject | undefined => {
	let best: WorldObject | undefined;
	let bd = Infinity;
	for (const o of list) {
		const d = dist2(o.pos, p);
		if (d < bd) {
			bd = d;
			best = o;
		}
	}
	return best;
};

function clashes(pos: Vec3, radius: number, objects: WorldObject[], ignoreId?: string): boolean {
	for (const o of objects) {
		if (o.id === ignoreId) continue;
		const min = radius + kr(o.kind);
		if (dist2(pos, o.pos) < min * min) return true;
	}
	return false;
}

// Spiral outward from `anchor` to the nearest spot where `radius` fits clash-free
// (against existing objects AND an optional `avoid` disc, e.g. the player). Forces y=0.
function findFreeSpot(
	anchor: Vec3,
	radius: number,
	objects: WorldObject[],
	opts: { step?: number; maxRing?: number; avoid?: { pos: Vec3; r: number }; ignoreId?: string; water?: (x: number, z: number) => boolean } = {}
): Vec3 {
	const step = opts.step ?? 1.2;
	const maxRing = opts.maxRing ?? 40;
	const avoid = opts.avoid;
	const free = (p: Vec3) => {
		if (clashes(p, radius, objects, opts.ignoreId)) return false;
		if (opts.water && opts.water(p[0], p[2])) return false; // never settle on a lake
		if (avoid) {
			const min = radius + avoid.r;
			if (dist2(p, avoid.pos) < min * min) return false;
		}
		return true;
	};
	const start: Vec3 = [snap(anchor[0]), 0, snap(anchor[2])];
	if (free(start)) return start;
	for (let ring = 1; ring <= maxRing; ring++) {
		const n = Math.max(6, ring * 6);
		const rad = ring * step + radius;
		for (let i = 0; i < n; i++) {
			const a = (i / n) * Math.PI * 2;
			const c: Vec3 = [snap(anchor[0] + Math.cos(a) * rad), 0, snap(anchor[2] + Math.sin(a) * rad)];
			if (free(c)) return c;
		}
	}
	return start; // best guess within constraints
}

const AREA: Record<string, Vec3> = {
	north: [0, 0, -30], south: [0, 0, 30], east: [30, 0, 0], west: [-30, 0, 0],
	center: [0, 0, 0], everywhere: [0, 0, 0]
};

// terrain presets → one CONTAINED feature (radius / peak height / roughness)
const TERRAIN_PRESET: Record<string, { radius: number; height: number; rough: number }> = {
	hills: { radius: 18, height: 4, rough: 1 },
	mountains: { radius: 24, height: 16, rough: 0.4 },
	dunes: { radius: 20, height: 2.5, rough: 1.5 },
	valley: { radius: 18, height: -5, rough: 0.5 },
	plateau: { radius: 16, height: 5, rough: 0 }
};

function blockersAt(objects: WorldObject[], c: [number, number], radius: number): string[] {
	const ids: string[] = [];
	for (const o of objects) {
		const dx = o.pos[0] - c[0];
		const dz = o.pos[2] - c[1];
		if (dx * dx + dz * dz < radius * radius) ids.push(o.id);
	}
	return ids;
}

// search outward from `prefer` for the clearest centre for a footprint of `radius`;
// returns that centre + the object ids still inside it ([] = found fully clear).
function findClearArea(objects: WorldObject[], prefer: [number, number], radius: number): { center: [number, number]; blockers: string[] } {
	const cands: [number, number][] = [prefer];
	for (let ring = 1; ring <= 6; ring++) {
		const rr = ring * radius * 0.9;
		for (let i = 0; i < 8; i++) {
			const a = (i / 8) * Math.PI * 2;
			cands.push([prefer[0] + Math.cos(a) * rr, prefer[1] + Math.sin(a) * rr]);
		}
	}
	let best = prefer;
	let bestBlockers = blockersAt(objects, prefer, radius);
	for (const c of cands) {
		if (bestBlockers.length === 0) break;
		const b = blockersAt(objects, c, radius);
		if (b.length < bestBlockers.length) {
			best = c;
			bestBlockers = b;
		}
	}
	return { center: best, blockers: bestBlockers };
}

// Fuzzy object lookup (the small model is bad at exact ids):
// "last"/"it"/"that" → newest object · exact id → "o"+id → nearest object of that KIND →
// (loose only) nearest object overall. `loose=false` for edit/remove/move targets so a garbage
// id can't nuke a random object; "here"/"me" are NOT object refs (caller falls back to the player).
function resolveRef(ref: string, objects: WorldObject[], p: Vec3, loose = true): WorldObject | undefined {
	const r = ref.trim().toLowerCase();
	if (r === 'last' || r === 'it' || r === 'that' || r === 'previous') return objects[objects.length - 1];
	if (r === 'here' || r === 'me' || r === 'player' || r === 'us' || r === '') return undefined;
	return (
		objects.find((o) => o.id.toLowerCase() === r) ||
		objects.find((o) => o.id.toLowerCase() === 'o' + r) ||
		nearest(objects.filter((o) => o.kind.toLowerCase() === r), p) ||
		(loose ? nearest(objects, p) : undefined)
	);
}

// Resolve a symbolic anchor → a world point. ALL spatial relations live here, never in the LLM.
// Egocentric dirs use the player's yaw; FoR is always egocentric by design (see docs/relative-spacing.md).
function resolveAnchor(op: { pos?: Vec3; at?: string; dist?: number }, player: Player, objects: WorldObject[]): Vec3 {
	if (Array.isArray(op.pos)) return op.pos;
	const at = (op.at || 'front').trim();
	const ci = at.indexOf(':');
	const head = (ci >= 0 ? at.slice(0, ci) : at).toLowerCase();
	let rest = ci >= 0 ? at.slice(ci + 1) : '';
	if (rest.toLowerCase().startsWith('near:')) rest = rest.slice(5); // tolerate "front:near:tower" → tower
	const p = player.pos;
	const fx = -Math.sin(player.yaw); // forward
	const fz = -Math.cos(player.yaw);
	const d = op.dist && op.dist > 0 ? Math.min(op.dist, 120) : 5; // clamp absurd dist (model sometimes emits 1e15)

	if (head === 'here') return p;

	// egocentric directions, optionally relative to a referenced OBJECT ("front:o1" = in front of it)
	const DIRS: Record<string, [number, number]> = {
		front: [fx, fz], ahead: [fx, fz],
		behind: [-fx, -fz], back: [-fx, -fz],
		right: [-fz, fx], left: [fz, -fx]
	};
	const dir = DIRS[head];
	if (dir) {
		const ref = rest && rest !== 'me' ? resolveRef(rest, objects, p, false) : undefined;
		if (ref) {
			const off = kr(ref.kind) + (op.dist && op.dist > 0 ? Math.min(op.dist, 60) : 2.5);
			return [ref.pos[0] + dir[0] * off, 0, ref.pos[2] + dir[1] * off];
		}
		return [p[0] + dir[0] * d, 0, p[2] + dir[1] * d];
	}

	if (head === 'between') {
		const [a, b] = rest.split(',');
		const oa = a ? resolveRef(a, objects, p) : undefined;
		const ob = b ? resolveRef(b, objects, p) : undefined;
		if (oa && ob) return [(oa.pos[0] + ob.pos[0]) / 2, 0, (oa.pos[2] + ob.pos[2]) / 2];
		const one = oa || ob;
		if (one) return [one.pos[0] + kr(one.kind) + 1.5, 0, one.pos[2]];
		return p;
	}

	if (head === 'on') {
		const t = resolveRef(rest, objects, p);
		if (t) return [t.pos[0], t.pos[1] + kindDef(t.kind).h * (t.scale?.[1] ?? 1), t.pos[2]]; // on the roof
		return p;
	}

	// beside / next to / around a thing (around's ring is built in the add case; this is the centre)
	if (head === 'near' || head === 'beside' || head === 'nextto' || head === 'by' || head === 'around' || head === 'surround') {
		const t = resolveRef(rest, objects, p);
		if (t) return [t.pos[0] + kr(t.kind) + 1.5, 0, t.pos[2]];
	}

	if (AREA[at.toLowerCase()]) return AREA[at.toLowerCase()];
	return p;
}

export function applyOps(
	world: World,
	ops: Op[],
	player: Player = { pos: [0, 0, 0], yaw: 0 },
	out?: { conflicts: PlacementConflict[] }
): World {
	// start IDs past the highest existing 'o' id (NOT array length) — after a remove, length-based ids
	// would collide with surviving objects → Svelte "each_key_duplicate" crash
	let n = 0;
	for (const o of world.objects) {
		if (o.id[0] === 'o') {
			const v = parseInt(o.id.slice(1), 36);
			if (Number.isFinite(v) && v >= n) n = v + 1;
		}
	}
	const newId = () => 'o' + (n++).toString(36);
	// never drop a freshly-placed object on top of the player who asked for it
	const avoid = { pos: player.pos, r: 0.6 };
	if (!world.zones) world.zones = [];
	if (!world.paths) world.paths = [];
	if (!world.terrain) world.terrain = [];
	// zone/path id counters must start past the highest EXISTING id (not the array length) — after a remove a
	// length-based next id collides with a survivor → duplicate 'p'/'z' keys → Svelte each_key_duplicate crash
	// (the same guard the 'o' object ids have above; paths/zones lacked it — e.g. two 'pc' paths after a remove).
	let zn = 0;
	for (const z of world.zones) {
		if (z.id[0] === 'z') {
			const v = parseInt(z.id.slice(1), 36);
			if (Number.isFinite(v) && v >= zn) zn = v + 1;
		}
	}
	let pn = 0;
	for (const p of world.paths) {
		if (p.id[0] === 'p') {
			const v = parseInt(p.id.slice(1), 36);
			if (Number.isFinite(v) && v >= pn) pn = v + 1;
		}
	}
	// never settle a solid object on a lake — placement routes around water (the blob shape the shader draws)
	const water = (x: number, z: number) => inWater(world.zones, x, z);

	const place = (kind: string, pos: Vec3, op: { scale?: Vec3; color?: string; rot?: number }) => {
		pos[1] = heightAt(pos[0], pos[2], world.terrain);
		world.objects.push({ id: newId(), kind, pos, scale: op.scale ?? [1, 1, 1], color: op.color, rot: op.rot ?? 0 });
	};

	for (const op of ops) {
		switch (op.op) {
			case 'add': {
				const r = kr(op.kind);
				const atStr = typeof op.at === 'string' ? op.at.trim().toLowerCase() : '';
				const onTop = atStr.startsWith('on:');
				const around = atStr.startsWith('around:') || atStr.startsWith('surround');
				const count = Math.max(1, Math.min(Math.floor(op.count ?? 1), MAX_COUNT));
				if (around) {
					// "fences around the house" → a RING around the referenced object's footprint
					const ref = resolveRef(atStr.slice(atStr.indexOf(':') + 1), world.objects, player.pos, false);
					const c = ref ? ref.pos : resolveAnchor(op, player, world.objects);
					const ringR = (ref ? kr(ref.kind) : 3) + r + 1.2;
					const ringN = Math.max(count, 8); // "around" implies several
					for (let i = 0; i < ringN; i++) {
						const a = (i / ringN) * TAU;
						place(op.kind, [c[0] + Math.cos(a) * ringR, 0, c[2] + Math.sin(a) * ringR], op);
					}
					break;
				}
				const anchor = resolveAnchor(op, player, world.objects);
				// count > 1 ("3 huts") packs them around the anchor — each findFreeSpot avoids the ones
				// already placed this op, so they don't overlap.
				for (let i = 0; i < count; i++) {
					if (onTop) {
						// on a roof — keep the exact anchor height, don't re-ground to the terrain
						world.objects.push({ id: newId(), kind: op.kind, pos: [anchor[0], anchor[1], anchor[2]], scale: op.scale ?? [1, 1, 1], color: op.color, rot: op.rot ?? 0 });
					} else {
						place(op.kind, findFreeSpot(anchor, r, world.objects, { avoid, water }), op);
					}
				}
				break;
			}
			case 'scatter': {
				const r = kr(op.kind);
				// scatter AROUND THE PLAYER (+ a nudge toward a named direction) so "30 cats" land near
				// you, not at the world origin
				const dir = AREA[op.area] ?? [0, 0, 0];
				const center: Vec3 = [player.pos[0] + dir[0] * 0.6, 0, player.pos[2] + dir[2] * 0.6];
				const total = Math.max(1, Math.min(Math.floor(op.count), MAX_COUNT));
				const spread = op.area === 'everywhere' ? 28 : 15;
				const GA = Math.PI * (3 - Math.sqrt(5)); // golden angle → even, deterministic spread
				for (let i = 0; i < total; i++) {
					const rr = spread * Math.sqrt((i + 0.5) / total);
					const a = i * GA;
					const anchor: Vec3 = [center[0] + Math.cos(a) * rr, 0, center[2] + Math.sin(a) * rr];
					place(op.kind, findFreeSpot(anchor, r, world.objects, { step: r * 1.5, avoid, water }), { color: op.color });
				}
				break;
			}
			case 'remove': {
				// fuzzy target (kind / "it" / id) but NOT loose — a bad ref is a no-op, never a random kill
				const t = resolveRef(op.id, world.objects, player.pos, false);
				if (t) {
					world.objects = world.objects.filter((o) => o.id !== t.id);
					break;
				}
				// ...also remove ZONES (lakes/plazas/…) and PATHS (roads) — by exact id or a material keyword,
				// nearest to the player. (resolveRef only knows objects, so these were un-deletable before.)
				const rid = typeof op.id === 'string' ? op.id.trim().toLowerCase() : '';
				if (world.zones.some((z) => z.id === rid)) {
					world.zones = world.zones.filter((z) => z.id !== rid);
					break;
				}
				if (world.paths.some((p) => p.id === rid)) {
					world.paths = world.paths.filter((p) => p.id !== rid);
					break;
				}
				const ZONE_WORD: Record<string, string> = {
					lake: 'water', pond: 'water', water: 'water', pool: 'water', plaza: 'plaza', courtyard: 'plaza',
					square: 'plaza', field: 'grass', lawn: 'grass', meadow: 'grass', sand: 'sand', beach: 'sand',
					ice: 'ice', lava: 'lava', flowers: 'flowers'
				};
				const mat = ZONE_WORD[rid];
				const nearestOf = <T extends { pos?: Vec3; from?: Vec3 }>(list: T[]): T | undefined => {
					let best: T | undefined, bd = Infinity;
					for (const it of list) {
						const c = it.pos ?? it.from ?? [0, 0, 0];
						const d = dist2(c, player.pos);
						if (d < bd) ((bd = d), (best = it));
					}
					return best;
				};
				if (mat) {
					const z = nearestOf(world.zones.filter((zo) => zo.material === mat));
					if (z) {
						world.zones = world.zones.filter((zo) => zo.id !== z.id);
						break;
					}
				}
				if (/^(road|street|path|trail|bridge)s?$/.test(rid)) {
					const pth = nearestOf(world.paths);
					if (pth) world.paths = world.paths.filter((p) => p.id !== pth.id);
				}
				break;
			}
			case 'move': {
				const o = resolveRef(op.id, world.objects, player.pos, false);
				if (o) {
					o.pos = op.pos ?? findFreeSpot(resolveAnchor(op, player, world.objects), kr(o.kind), world.objects, { avoid, ignoreId: o.id, water });
					o.pos[1] = heightAt(o.pos[0], o.pos[2], world.terrain);
				}
				break; // unknown ref → ignored
			}
			case 'paint': {
				const o = resolveRef(op.id, world.objects, player.pos, false);
				if (o) o.color = op.color;
				break; // unknown ref → ignored
			}
			case 'setGround':
				world.ground = op.value;
				break;
			case 'setSky':
				// night-only game (user decision 2026-06-21): any sky request resolves to night. `op.value` is
				// kept in the grammar (the tuned model still emits it) but coerced here so there's no path to day.
				world.sky = 'night';
				break;
			case 'addZone': {
				const size = op.size ?? 10;
				let prefer: Vec3 = resolveAnchor({ pos: op.pos, at: op.at }, player, world.objects);
				// a bare "dig a lake" must not land on the player — bias to open ground ahead
				if (!op.pos && (!op.at || op.at === 'here')) {
					const f = forward(player.yaw);
					prefer = [player.pos[0] + f[0] * (size + 4), 0, player.pos[2] + f[2] * (size + 4)];
				}
				const area = findClearArea(world.objects, [prefer[0], prefer[2]], size);
				const c: Vec3 = [area.center[0], heightAt(area.center[0], area.center[1], world.terrain), area.center[1]];
				world.zones.push({ id: 'z' + (zn++).toString(36), material: op.material, shape: op.shape, pos: c, size });
				if (area.blockers.length && out) {
					out.conflicts.push({ label: op.material === 'water' ? 'lake' : op.material, blockers: area.blockers });
				}
				break;
			}
			case 'addPath': {
				// explicit coords (fromPos/toPos) win — used by the native city generator; the LLM still
				// emits symbolic from/to anchors
				const from = op.fromPos ?? resolveAnchor({ at: op.from }, player, world.objects);
				let to = op.toPos ?? resolveAnchor({ at: op.to }, player, world.objects);
				// endpoints collapsed (bad/duplicate refs) → extend a sensible length ahead of the player
				if (dist2(from, to) < 4) {
					const f = forward(player.yaw);
					to = [from[0] + f[0] * 12, 0, from[2] + f[2] * 12];
				}
				world.paths.push({ id: 'p' + (pn++).toString(36), material: op.material, from, to, width: op.width ?? 3 });
				break;
			}
			case 'setTerrain': {
				if (op.preset === 'flat') {
					world.terrain = []; // flatten everything
				} else {
					const cfg = TERRAIN_PRESET[op.preset] ?? TERRAIN_PRESET.hills;
					const f = forward(player.yaw);
					const prefer: [number, number] = [player.pos[0] + f[0] * cfg.radius, player.pos[2] + f[2] * cfg.radius];
					const area = findClearArea(world.objects, prefer, cfg.radius);
					const height = op.amplitude && op.amplitude !== 0 ? op.amplitude : cfg.height;
					world.terrain.push({ center: area.center, radius: cfg.radius, height, rough: cfg.rough });
				}
				// re-ground every object onto the new surface (objects in a hill ride up onto it)
				for (const o of world.objects) o.pos[1] = heightAt(o.pos[0], o.pos[2], world.terrain);
				break;
			}
			default:
				break; // unknown / note → no world change
		}
	}
	return world;
}

// Test helper: do any two objects' footprints overlap?
export function overlaps(world: World): [string, string][] {
	const out: [string, string][] = [];
	const o = world.objects;
	for (let i = 0; i < o.length; i++)
		for (let j = i + 1; j < o.length; j++) {
			const min = kr(o[i].kind) + kr(o[j].kind);
			if (dist2(o[i].pos, o[j].pos) < min * min - 1e-6) out.push([o[i].id, o[j].id]);
		}
	return out;
}
