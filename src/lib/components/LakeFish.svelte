<script lang="ts">
	// Ambient FISH for a lake — a small shoal that swims around inside the water zone's organic blob, riding
	// just at the surface (the water is opaque, so they sit on top to stay visible). Deterministic per zone id
	// so a shared link shows the same shoal; NOT world objects and NOT agents (kept off the URL + out of the
	// food-chain hot path). They dart away when a cat or the player closes on the bank, and they publish their
	// live positions to fishRegistry so cats get lured to the water's edge (the pond obstacle stops the cat
	// dry → it "fishes" without wading in). Scene mounts one per water zone.
	import { untrack } from 'svelte';
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { playerState } from '$lib/playerState.svelte';
	import { agentManager } from '$lib/agents.svelte';
	import { fishRegistry, type FishPos } from '$lib/fish.svelte';
	import { waterSeed, waterEdgeFactor, waterSurfaceY } from '$lib/water';
	import { PRIM, creatureMat } from '$lib/sharedAssets';
	import type { Zone, TerrainFeature } from '$lib/world';

	let { zone, terrain = [] }: { zone: Zone; terrain?: TerrainFeature[] } = $props();
	const Z = untrack(() => ({ id: zone.id, x: zone.pos[0], y: zone.pos[1], z: zone.pos[2], size: zone.size }));
	const seed = waterSeed(Z.id);
	// the flat base surface height (shared with Water.svelte); fish RIDE the live waves on top of this (below).
	const WLEVEL = untrack(() => waterSurfaceY(zone, terrain));

	// Gerstner wave HEIGHT, mirroring Water.svelte's gerstner() vertical term (same dirs/wavelengths/amps/speeds
	// + the same elapsed-time clock) so each fish bobs with the actual surface and never submerges under a crest
	// of the opaque water (it used to flicker, fixed at a flat height while the waves rolled over it).
	const W_DIR = ([[1, 0], [0.7, 0.71], [-0.6, 0.8], [0.2, -0.98]] as [number, number][]).map(([x, z]) => {
		const l = Math.hypot(x, z);
		return [x / l, z / l] as [number, number];
	});
	const W_K = [7, 4.5, 3, 2].map((w) => (2 * Math.PI) / w);
	const W_AMP = [0.045, 0.028, 0.018, 0.012];
	const W_SPD = [1.1, 1.5, 1.9, 2.3];
	const waveAt = (x: number, z: number, time: number) => {
		let y = 0;
		for (let i = 0; i < 4; i++) y += W_AMP[i] * Math.sin(W_K[i] * (W_DIR[i][0] * x + W_DIR[i][1] * z) + time * W_SPD[i]);
		return y;
	};

	const COUNT = Math.max(3, Math.min(8, Math.round(Z.size / 4))); // the STARTING shoal
	const CAP = COUNT * 2 + 2; // pond carrying capacity — the shoal breeds UP toward this, then holds
	const REPRO_MIN = 22; // seconds between births (a calm pond fills slowly so the growth reads as "life")
	const REPRO_MAX = 46;
	const FLEE_R = 6; // a threat within this many metres of a fish sends it darting for open water
	const rnd = (n: number) => {
		const v = Math.sin(n * 12.9898 + 78.233) * 43758.5453;
		return v - Math.floor(v);
	};

	// blob test for THIS zone (mirrors water.ts inWater, but scoped to one pond)
	const inBlob = (x: number, z: number) => {
		const lx = x - Z.x;
		const ly = Z.z - z; // matches Water's vLocal handedness (−90° tilt)
		const r2 = lx * lx + ly * ly;
		if (r2 >= Z.size * Z.size) return false;
		const edge = Z.size * waterEdgeFactor(seed, Math.atan2(ly, lx));
		return r2 < edge * edge;
	};

	type Fish = { x: number; z: number; h: number; tx: number; tz: number; ph: number; spd: number; sz: number; leap: number; leapCd: number };
	// pre-allocate the FULL capacity (the array never grows → no reactivity in the hot path); only the first
	// `active` fish are alive + rendered, and `active` climbs toward CAP as the shoal breeds (see useTask).
	const fish: Fish[] = untrack(() => {
		const out: Fish[] = [];
		for (let i = 0; i < CAP; i++) {
			const a = rnd(seed + i) * Math.PI * 2;
			const r = Z.size * 0.6 * Math.sqrt(rnd(seed + i * 3.1));
			const x = Z.x + Math.cos(a) * r;
			const z = Z.z + Math.sin(a) * r;
			out.push({ x, z, h: a, tx: x, tz: z, ph: rnd(i + 1) * 6.28, spd: 1.0 + rnd(i + 7) * 0.8, sz: 0.8 + rnd(i + 11) * 0.5, leap: 0, leapCd: 4 + rnd(i + 17) * 8 });
		}
		return out;
	});
	let active = COUNT; // how many of `fish` are currently alive (grows via breeding) — plain let, not reactive
	let bornCd = untrack(() => REPRO_MIN + rnd(seed) * (REPRO_MAX - REPRO_MIN)); // countdown to the next birth
	// the live-position array cats query (only the active fish — inactive ones are parked far off so they
	// neither lure cats nor render). It's CAP-long and fixed (registered once); births just flip entries live.
	const school: FishPos[] = untrack(() => fish.map((f, i) => (i < active ? { x: f.x, z: f.z } : { x: 1e9, z: 1e9 })));
	$effect(() => fishRegistry.register(school));

	const groups: THREE.Group[] = [];
	const bodyMat = creatureMat('#d98a4a'); // koi orange
	const finMat = creatureMat('#c2703a');

	// pick a fresh wander target somewhere inside the blob
	const retarget = (f: Fish) => {
		for (let k = 0; k < 6; k++) {
			const a = rnd(f.ph + f.x + k) * Math.PI * 2;
			const r = Z.size * 0.7 * Math.sqrt(rnd(f.z + k + 1));
			const tx = Z.x + Math.cos(a) * r;
			const tz = Z.z + Math.sin(a) * r;
			if (inBlob(tx, tz)) ((f.tx = tx), (f.tz = tz));
			if (inBlob(tx, tz)) return;
		}
	};

	let t = 0; // elapsed since mount — same clock as Water's uTime (both mount with the zone) → waves stay in phase
	useTask((dt) => {
		t += dt;
		// BREEDING — the shoal grows toward CAP. Every so often a new fish "hatches" beside an existing one
		// (we just activate the next pre-allocated slot + seed it near a random parent), so a pond that starts
		// sparse fills out over a few minutes. Holds at CAP; no fish are removed (nothing eats them — cats only
		// get lured to the bank), so this reads as a recovering/thriving shoal rather than runaway growth.
		if (active < CAP) {
			bornCd -= dt;
			if (bornCd <= 0) {
				const parent = fish[(rnd(t + active) * active) | 0];
				const b = fish[active];
				b.x = parent.x;
				b.z = parent.z;
				b.tx = parent.x;
				b.tz = parent.z;
				b.ph = rnd(t * 1.3 + active) * 6.28;
				b.leap = 0;
				b.leapCd = 3 + rnd(t + active * 1.7) * 8;
				school[active].x = b.x;
				school[active].z = b.z;
				active++;
				bornCd = REPRO_MIN + rnd(t * 2.1 + active) * (REPRO_MAX - REPRO_MIN);
			}
		}
		// gather threats once: the player + any nearby land predator (cat/lion/person) near this pond
		const reach = Z.size + 10;
		const threats: { x: number; z: number }[] = [{ x: playerState.pos[0], z: playerState.pos[2] }];
		agentManager.forEach((m) => {
			if (m.dead) return;
			if (m.kind !== 'cat' && m.kind !== 'lion' && m.kind !== 'person') return;
			if (Math.abs(m.agent.x - Z.x) < reach && Math.abs(m.agent.z - Z.z) < reach) threats.push({ x: m.agent.x, z: m.agent.z });
		});

		for (let i = 0; i < active; i++) {
			const f = fish[i];
			// nearest threat
			let tdx = 0;
			let tdz = 0;
			let td = Infinity;
			for (const t of threats) {
				const d = Math.hypot(f.x - t.x, f.z - t.z);
				if (d < td) ((td = d), (tdx = f.x - t.x), (tdz = f.z - t.z));
			}

			let dirx: number;
			let dirz: number;
			let boost = 1;
			if (td < FLEE_R) {
				// bolt away from the threat, biased back toward open water (the lake centre)
				const fd = Math.hypot(tdx, tdz) || 0.1;
				dirx = tdx / fd + (Z.x - f.x) * 0.05;
				dirz = tdz / fd + (Z.z - f.z) * 0.05;
				boost = 2.2;
				f.ph += dt * 14; // frantic tail
			} else {
				if (Math.hypot(f.tx - f.x, f.tz - f.z) < 0.6) retarget(f);
				dirx = f.tx - f.x;
				dirz = f.tz - f.z;
				f.ph += dt * (3 + f.spd * 2);
			}
			const dl = Math.hypot(dirx, dirz) || 1;
			let nx = f.x + (dirx / dl) * f.spd * boost * dt;
			let nz = f.z + (dirz / dl) * f.spd * boost * dt;
			if (!inBlob(nx, nz)) {
				// would beach itself → stay put this step and aim back toward the centre
				nx = f.x;
				nz = f.z;
				f.tx = Z.x;
				f.tz = Z.z;
			}
			f.x = nx;
			f.z = nz;
			f.h += (Math.atan2(dirz, dirx) - f.h) * Math.min(1, 6 * dt); // ease heading toward travel dir
			school[i].x = f.x;
			school[i].z = f.z;

			// occasional surface LEAP (a parabolic arc clear of the water) for life — never while fleeing
			if (f.leap > 0) {
				f.leap += dt / 0.7; // ~0.7 s arc
				if (f.leap >= 1) f.leap = 0;
			} else {
				f.leapCd -= dt;
				if (f.leapCd <= 0 && td >= FLEE_R) ((f.leap = 0.001), (f.leapCd = 6 + rnd(f.x + i) * 10));
			}
			const arc = f.leap > 0 ? Math.sin(f.leap * Math.PI) : 0; // 0→1→0

			const g = groups[i];
			if (g) {
				g.visible = true;
				g.position.set(f.x, WLEVEL + waveAt(f.x, f.z, t) + 0.06 + arc * 0.55, f.z); // ride the live wave surface
				g.rotation.y = -f.h + Math.PI / 2; // model faces +Z; align to heading
				g.rotation.z = Math.sin(f.ph) * 0.18; // body roll/wiggle
				g.rotation.x = arc > 0 ? -Math.cos(f.leap * Math.PI) * 0.6 : 0; // nose up on the way up, down on the way down
			}
		}
		// keep the not-yet-hatched slots invisible (they default to a fish sitting at the world origin otherwise)
		for (let i = active; i < fish.length; i++) {
			const g = groups[i];
			if (g) g.visible = false;
		}
	});
</script>

{#each fish as f, i (i)}
	<T.Group bind:ref={groups[i]} scale={f.sz}>
		<!-- body: a flattened spindle -->
		<T.Mesh geometry={PRIM.sphere} scale={[0.18, 0.12, 0.42]} material={bodyMat} />
		<!-- tail fin -->
		<T.Mesh geometry={PRIM.cone} scale={[0.02, 0.16, 0.18]} position={[0, 0, -0.32]} rotation={[Math.PI / 2, 0, 0]} material={finMat} />
		<!-- dorsal fin breaking the surface -->
		<T.Mesh geometry={PRIM.cone} scale={[0.02, 0.12, 0.12]} position={[0, 0.1, 0]} material={finMat} />
	</T.Group>
{/each}
