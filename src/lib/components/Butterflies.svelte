<script lang="ts">
	// Daytime BUTTERFLIES — a few bright ones fluttering LOW near the player, with fast-flapping wings and
	// erratic darting paths (frequent retargeting + vertical bob = the characteristic flutter). Fills the
	// near-ground daytime niche (fireflies are night, birds are high, fish are in water). Ambient, never
	// saved; a handful → cheap. Day/sunset only — they tuck away at night/fog/space.
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { heightAt } from '$lib/terrain';
	import { playerState } from '$lib/playerState.svelte';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();

	const COUNT = 8;
	const PALETTE = ['#f0a02a', '#e8d23a', '#5a8fe0', '#e85a8a', '#f2f2f2', '#b66ad8', '#e0623a', '#6ad0c0'];
	// horizontal wings extending out from a tiny dark body; flap pivots about the body's forward axis
	const wingGeoL = new THREE.PlaneGeometry(0.5, 0.42).rotateX(-Math.PI / 2).translate(0.26, 0, 0);
	const wingGeoR = new THREE.PlaneGeometry(0.5, 0.42).rotateX(-Math.PI / 2).translate(-0.26, 0, 0);
	const bodyGeo = new THREE.BoxGeometry(0.06, 0.06, 0.42);
	const bodyMat = new THREE.MeshStandardMaterial({ color: '#2a2620', flatShading: true });
	const wingMats = PALETTE.map((c) => new THREE.MeshStandardMaterial({ color: c, flatShading: true, side: THREE.DoubleSide }));

	const rnd = (n: number) => {
		const v = Math.sin(n * 12.9898 + 4.13) * 43758.5453;
		return v - Math.floor(v);
	};
	const bf = Array.from({ length: COUNT }, (_, i) => ({
		x: 0,
		z: 0,
		y: 0,
		tx: 0,
		tz: 0,
		hdg: 0,
		col: i % PALETTE.length,
		alt: 0.5 + rnd(i) * 1.1, // 0.5–1.6 m off the ground
		speed: 1.6 + rnd(i + 3) * 1.6,
		flap: rnd(i + 9) * 6.28,
		bob: rnd(i + 5) * 6.28,
		inited: false
	}));

	let groups = $state<THREE.Group[]>([]);
	let wingsL = $state<THREE.Group[]>([]);
	let wingsR = $state<THREE.Group[]>([]);

	let cx = 0; // wander centre lags the player so they drift along with you
	let cz = 0;
	let t = 0;

	const retarget = (b: (typeof bf)[number], i: number) => {
		const a = rnd(t * 7.3 + i + b.x) * Math.PI * 2;
		const r = 2 + rnd(t * 3.1 + i) * 9; // dart to a fresh point near the centre
		b.tx = cx + Math.cos(a) * r;
		b.tz = cz + Math.sin(a) * r;
	};

	useTask((dt) => {
		// daytime only — and NOT in a snow world (no butterflies in winter; out-of-season would read wrong)
		const active = (world.sky === 'day' || world.sky === 'sunset') && world.ground !== 'snow';
		for (const g of groups) if (g) g.visible = active;
		if (!active) return;
		t += dt;
		const k = Math.min(1, dt * 0.6);
		cx += (playerState.pos[0] - cx) * k;
		cz += (playerState.pos[2] - cz) * k;

		for (let i = 0; i < COUNT; i++) {
			const b = bf[i];
			const g = groups[i];
			if (!g) continue;
			if (!b.inited) {
				b.x = cx + rnd(i) * 6 - 3;
				b.z = cz + rnd(i + 1) * 6 - 3;
				retarget(b, i);
				b.inited = true;
			}
			// STARTLE: dart AWAY when you walk into them → you scatter a cloud of butterflies (the near-ground
			// daytime "world reacts to you" touch, like the grass parting + bushes rustling). They drift back
			// after, since the normal retarget aims near the player-following wander centre.
			const pdx = b.x - playerState.pos[0];
			const pdz = b.z - playerState.pos[2];
			const pd = Math.hypot(pdx, pdz);
			const startled = pd < 1.9;
			if (startled) {
				b.tx = b.x + (pdx / (pd || 1)) * 7; // flee straight away from you
				b.tz = b.z + (pdz / (pd || 1)) * 7;
			}
			let dx = b.tx - b.x;
			let dz = b.tz - b.z;
			const d = Math.hypot(dx, dz);
			if (d < 0.8 && !startled) retarget(b, i); // arrived → flit somewhere new (erratic)
			dx /= d || 1;
			dz /= d || 1;
			// jittery speed so it darts and pauses, not a smooth glide — and a burst of speed when startled
			const sp = b.speed * (0.5 + 0.5 * Math.sin(t * 3.0 + b.bob)) * (startled ? 2.4 : 1.0);
			b.x += dx * sp * dt;
			b.z += dz * sp * dt;
			b.y = heightAt(b.x, b.z, world.terrain) + b.alt + Math.sin(t * 6.0 + b.bob) * 0.35; // fluttery bob
			g.position.set(b.x, b.y, b.z);

			let dh = Math.atan2(dx, dz) - b.hdg;
			while (dh > Math.PI) dh -= 2 * Math.PI;
			while (dh < -Math.PI) dh += 2 * Math.PI;
			b.hdg += dh * Math.min(1, dt * 8);
			g.rotation.y = b.hdg;

			b.flap += (14 + sp * 2) * dt;
			const f = (Math.sin(b.flap) * 0.5 + 0.5) * 1.4; // 0..1.4, wings beat up over the back
			if (wingsL[i]) wingsL[i].rotation.z = f;
			if (wingsR[i]) wingsR[i].rotation.z = -f;
		}
	});
</script>

{#each bf as b, i (i)}
	<T.Group bind:ref={groups[i]} visible={false}>
		<T.Mesh geometry={bodyGeo} material={bodyMat} />
		<T.Group bind:ref={wingsL[i]}>
			<T.Mesh geometry={wingGeoL} material={wingMats[b.col]} />
		</T.Group>
		<T.Group bind:ref={wingsR[i]}>
			<T.Mesh geometry={wingGeoR} material={wingMats[b.col]} />
		</T.Group>
	</T.Group>
{/each}
