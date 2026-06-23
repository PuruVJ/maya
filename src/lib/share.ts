// "The world is a link." Pack the World into a compact positional form (short keys, arrays, and the
// derived ground-Y dropped — recomputed from heightAt on load), gzip it with the browser-native
// CompressionStream, and base64url it for the URL hash. No deps, no server. Round-trips via decode.
import { repairIds, type World } from './world';
import { heightAt } from './terrain';

const r1 = (n: number) => Math.round(n * 10) / 10;
const scaleOrZero = (s?: [number, number, number]) =>
	s && (s[0] !== 1 || s[1] !== 1 || s[2] !== 1) ? [r1(s[0]), r1(s[1]), r1(s[2])] : 0;

/* eslint-disable @typescript-eslint/no-explicit-any */

// compact positional pack — objects/zones/paths store [x,z] only; Y is re-derived on load
type Live = Map<string, { x: number; z: number; dead: boolean; asleep: boolean; ageFrac?: number }>;
type PlayerPos = { x: number; z: number; yaw: number };

function pack(w: World, live?: Live, player?: PlayerPos) {
	return {
		v: w.v,
		n: w.name,
		g: w.ground,
		s: w.sky,
		sp: [r1(w.spawn[0]), r1(w.spawn[1]), r1(w.spawn[2])],
		// where the player is standing right now → reopening the link drops you back here (handy on reload)
		pl: player ? [r1(player.x), r1(player.z), r1(player.yaw)] : 0,
		// 7th element = live-state flag (0 alive · 1 dead · 2 asleep); pos uses the live position if the agent
		// has wandered (so a shared link reopens animals where they are now / as corpses)
		o: w.objects.map((o) => {
			const lv = live?.get(o.id);
			const x = lv ? lv.x : o.pos[0];
			const z = lv ? lv.z : o.pos[2];
			const dead = lv ? lv.dead : o.dead;
			const asleep = lv ? lv.asleep : o.asleep;
			const arr: (string | number | number[])[] = [o.kind, r1(x), r1(z), o.color ?? 0, r1(o.rot ?? 0), scaleOrZero(o.scale), dead ? 1 : asleep ? 2 : 0];
			// 8th element (creatures only) = AGE life-fraction → a reload keeps adults adult, not seeded-young
			const af = lv?.ageFrac ?? o.ageFrac;
			if (af != null) arr.push(Math.round(af * 1000) / 1000);
			return arr;
		}),
		z: w.zones.map((z) => [z.material, z.shape, r1(z.pos[0]), r1(z.pos[2]), z.size]),
		p: w.paths.map((p) => [p.material, r1(p.from[0]), r1(p.from[2]), r1(p.to[0]), r1(p.to[2]), p.width]),
		t: w.terrain.map((f) => [r1(f.center[0]), r1(f.center[1]), f.radius, f.height, f.rough])
	};
}

function unpack(d: any): World {
	const terrain = (d.t ?? []).map((f: any[]) => ({ center: [f[0], f[1]] as [number, number], radius: f[2], height: f[3], rough: f[4] }));
	const y = (x: number, z: number) => heightAt(x, z, terrain);
	return {
		v: d.v ?? 1,
		name: d.n ?? 'Shared world',
		ground: d.g ?? 'grass',
		sky: d.s ?? 'day',
		spawn: Array.isArray(d.sp) ? [d.sp[0], d.sp[1], d.sp[2]] : [0, 0, 0],
		start: Array.isArray(d.pl) ? { x: d.pl[0], z: d.pl[1], yaw: d.pl[2] } : undefined,
		objects: (d.o ?? []).map((a: any[], i: number) => ({
			id: 'o' + i.toString(36),
			kind: a[0],
			pos: [a[1], y(a[1], a[2]), a[2]] as [number, number, number],
			color: a[3] || undefined,
			rot: a[4] || 0,
			scale: (Array.isArray(a[5]) ? a[5] : [1, 1, 1]) as [number, number, number],
			dead: a[6] === 1 || undefined, // live-state flag (0/1/2) → restore corpse/sleeper
			asleep: a[6] === 2 || undefined,
			ageFrac: typeof a[7] === 'number' ? a[7] : undefined // 8th element (creatures) → restore exact age on spawn
		})),
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
