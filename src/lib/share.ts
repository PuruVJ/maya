// "The world is a link." Pack the World into a compact positional form (short keys, arrays, and the
// derived ground-Y dropped — recomputed from heightAt on load), gzip it with the browser-native
// CompressionStream, and base64url it for the URL hash. No deps, no server. Round-trips via decode.
import { repairIds, type World, type WorldObject, type RegionAggregate } from './world';
import { heightAt } from './terrain';

const r1 = (n: number) => Math.round(n * 10) / 10;
const scaleOrZero = (s?: [number, number, number]) =>
	s && (s[0] !== 1 || s[1] !== 1 || s[2] !== 1) ? [r1(s[0]), r1(s[1]), r1(s[2])] : 0;

/* eslint-disable @typescript-eslint/no-explicit-any */

// compact positional pack — objects/zones/paths store [x,z] only; Y is re-derived on load
type Live = Map<string, { x: number; z: number; dead: boolean; asleep: boolean; ageFrac?: number }>;
type PlayerPos = { x: number; z: number; yaw: number; y?: number };

// One object → its positional array (kind, x, z, color, rot, scale, live-flag, [ageFrac]). Shared by live objects
// AND by region-aggregate statics, so the two stay in lockstep.
function packObj(o: WorldObject, x: number, z: number, dead: boolean, asleep: boolean, ageFrac?: number) {
	const arr: (string | number | number[])[] = [o.kind, r1(x), r1(z), o.color ?? 0, r1(o.rot ?? 0), scaleOrZero(o.scale), dead ? 1 : asleep ? 2 : 0];
	if (ageFrac != null) arr.push(Math.round(ageFrac * 1000) / 1000);
	return arr;
}
function unpackObj(a: any[], id: string, y: (x: number, z: number) => number): WorldObject {
	return {
		id,
		kind: a[0],
		pos: [a[1], y(a[1], a[2]), a[2]] as [number, number, number],
		color: a[3] || undefined,
		rot: a[4] || 0,
		scale: (Array.isArray(a[5]) ? a[5] : [1, 1, 1]) as [number, number, number],
		dead: a[6] === 1 || undefined,
		asleep: a[6] === 2 || undefined,
		ageFrac: typeof a[7] === 'number' ? a[7] : undefined
	};
}

function pack(w: World, live?: Live, player?: PlayerPos) {
	return {
		v: w.v,
		n: w.name,
		g: w.ground,
		s: w.sky,
		sp: [r1(w.spawn[0]), r1(w.spawn[1]), r1(w.spawn[2])],
		// where the player is standing right now → reopening the link drops you back here (handy on reload)
		pl: player ? [r1(player.x), r1(player.z), r1(player.yaw), r1(player.y ?? 0)] : 0,
		// 7th element = live-state flag (0 alive · 1 dead · 2 asleep); pos uses the live position if the agent
		// has wandered (so a shared link reopens animals where they are now / as corpses)
		o: w.objects.map((o) => {
			const lv = live?.get(o.id);
			// 8th element (creatures only) = AGE life-fraction → a reload keeps adults adult, not seeded-young
			return packObj(o, lv ? lv.x : o.pos[0], lv ? lv.z : o.pos[2], !!(lv ? lv.dead : o.dead), !!(lv ? lv.asleep : o.asleep), lv?.ageFrac ?? o.ageFrac);
		}),
		z: w.zones.map((z) => [z.material, z.shape, r1(z.pos[0]), r1(z.pos[2]), z.size]),
		p: w.paths.map((p) => [p.material, r1(p.from[0]), r1(p.from[2]), r1(p.to[0]), r1(p.to[2]), p.width]),
		t: w.terrain.map((f) => [r1(f.center[0]), r1(f.center[1]), f.radius, f.height, f.rough]),
		// DORMANT REGION AGGREGATES — the streamed-out FAR world. WITHOUT THIS a shared link keeps only the ~LIVE_BUDGET
		// objects live near the sharer and silently drops everything else (the "2000 objects → ~500 on load" bug). Each
		// region packs [key, gene, lastTick, [kind,n,…] counts, [statics…]]; statics pack like objects (classified by
		// kind on wake, so no extra flag needed). 0 (not []) when there are none → costs nothing for un-streamed worlds.
		r: w.regions
			? Object.entries(w.regions).map(([key, a]) => [key, r1(a.gene), Math.round(a.lastTick), Object.entries(a.counts).flatMap(([k, n]) => [k, n]), (a.statics ?? []).map((s) => packObj(s, s.pos[0], s.pos[2], !!s.dead, !!s.asleep, s.ageFrac))])
			: 0
	};
}

function unpack(d: any): World {
	const terrain = (d.t ?? []).map((f: any[]) => ({ center: [f[0], f[1]] as [number, number], radius: f[2], height: f[3], rough: f[4] }));
	const y = (x: number, z: number) => heightAt(x, z, terrain);
	const w: World = {
		v: d.v ?? 1,
		name: d.n ?? 'Shared world',
		ground: d.g ?? 'grass',
		sky: d.s ?? 'day',
		spawn: Array.isArray(d.sp) ? [d.sp[0], d.sp[1], d.sp[2]] : [0, 0, 0],
		start: Array.isArray(d.pl) ? { x: d.pl[0], z: d.pl[1], yaw: d.pl[2], y: typeof d.pl[3] === 'number' ? d.pl[3] : undefined } : undefined,
		objects: (d.o ?? []).map((a: any[], i: number) => unpackObj(a, 'o' + i.toString(36), y)),
		zones: (d.z ?? []).map((a: any[], i: number) => ({
			id: 'z' + i.toString(36),
			material: a[0],
			shape: a[1],
			pos: [a[2], y(a[2], a[3]), a[3]] as [number, number, number],
			size: a[4]
		})),
		paths: (d.p ?? []).map((a: any[], i: number) => ({
			id: 'p' + i.toString(36),
			material: a[0],
			from: [a[1], y(a[1], a[2]), a[2]] as [number, number, number],
			to: [a[3], y(a[3], a[4]), a[4]] as [number, number, number],
			width: a[5]
		})),
		terrain
	};
	// restore DORMANT REGION AGGREGATES (far streamed-out world) → the full population survives the share, not just the
	// live near-set. As the recipient explores, these wake into individuals exactly like the sharer's world did.
	if (Array.isArray(d.r) && d.r.length) {
		const regions: Record<string, RegionAggregate> = {};
		for (const e of d.r as any[]) {
			const [key, gene, lastTick, countsFlat, statics] = e;
			const counts: Record<string, number> = {};
			const cf: any[] = countsFlat ?? [];
			for (let i = 0; i + 1 < cf.length; i += 2) counts[cf[i]] = cf[i + 1];
			regions[String(key)] = {
				counts,
				gene: typeof gene === 'number' ? gene : 1,
				lastTick: typeof lastTick === 'number' ? lastTick : 0,
				statics: (statics ?? []).map((a: any[], i: number) => unpackObj(a, 's' + String(key).replace(',', '_') + '_' + i.toString(36), y))
			};
		}
		w.regions = regions;
	}
	return w;
}

// --- browser-native gzip <-> base64url (no deps) ---
async function gzip(str: string): Promise<Uint8Array> {
	const cs = new CompressionStream('gzip');
	const buf = await new Response(new Blob([str]).stream().pipeThrough(cs)).arrayBuffer();
	return new Uint8Array(buf);
}
async function gunzip(bytes: Uint8Array): Promise<string> {
	const ds = new DecompressionStream('gzip');
	return new Response(new Blob([bytes as any]).stream().pipeThrough(ds)).text();
}
function toB64url(bytes: Uint8Array): string {
	let bin = '';
	for (const b of bytes) bin += String.fromCharCode(b);
	return btoa(bin).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}
function fromB64url(s: string): Uint8Array {
	const pad = s.length % 4 ? '='.repeat(4 - (s.length % 4)) : '';
	const bin = atob(s.replace(/-/g, '+').replace(/_/g, '/') + pad);
	const bytes = new Uint8Array(bin.length);
	for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
	return bytes;
}

/** World → URL-safe token for the `#w=` hash. */
export async function encodeWorld(w: World, live?: Live, player?: PlayerPos): Promise<string> {
	return toB64url(await gzip(JSON.stringify(pack(w, live, player))));
}

/** `#w=` token → World (Y re-grounded from terrain; ids regenerated + de-duplicated). */
export async function decodeWorld(token: string): Promise<World> {
	const w = repairIds(unpack(JSON.parse(await gunzip(fromB64url(token)))));
	w.sky = 'night'; // night-only game — any older shared world reopens as night
	return w;
}
