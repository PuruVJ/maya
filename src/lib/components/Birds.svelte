<script lang="ts">
	// A small flock of birds wheeling around the player — the world's only aerial life. Each bird is a
	// tiny procedural group (body + two flapping wings) on a LOW, WIDE orbit so it crosses your forward
	// view rather than sitting straight overhead. The orbit is deliberately imperfect — irregular speed,
	// a wandering radius and fluttering altitude (layered sines) — so the path looks erratic and alive,
	// not a clean circle. Heading is taken from actual velocity, so birds bank into their jagged turns.
	// A handful of birds → cheap; ambient, never saved or shared.
	import { untrack } from 'svelte';
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { heightAt } from '$lib/terrain';
	import { playerState } from '$lib/playerState.svelte';
	import { litMat } from '$lib/sharedAssets';
	import type { World } from '$lib/world';

	// `mode` swaps the same erratic-flock engine between daytime BIRDS and nocturnal BATS — aerial life that
	// follows the day/night cycle (birds roost at night, bats come out). Scene mounts one of each.
	let { world, mode = 'bird' }: { world: World; mode?: 'bird' | 'bat' } = $props();
	const isBat = untrack(() => mode === 'bat'); // constant per mount (Scene mounts one bird + one bat instance)

	const COUNT = isBat ? 9 : 7;
	const bodyGeo = new THREE.ConeGeometry(0.12, 0.62, 5).rotateX(Math.PI / 2);
	const wingGeoL = new THREE.BoxGeometry(0.9, 0.04, 0.34).translate(0.45, 0, 0); // extends +x
	const wingGeoR = new THREE.BoxGeometry(0.9, 0.04, 0.34).translate(-0.45, 0, 0); // extends -x
	const mat = litMat(isBat ? '#211f27' : '#3a3f48'); // bats near-black

	// per-creature params. Bats fly LOWER, FASTER and flap harder for a jittery, erratic night flit.
	const birds = Array.from({ length: COUNT }, (_, i) => ({
		angle: (i / COUNT) * Math.PI * 2,
		radius: (isBat ? 13 : 20) + (i % 4) * 3, // bats keep a tighter, nearer loop
		speed: (isBat ? 0.3 : 0.18) + (i % 3) * (isBat ? 0.08 : 0.05),
		alt: (isBat ? 5 : 9) + (i % 5) * (isBat ? 1.3 : 1.6), // bats lower in the sky
		rPhase: i * 1.3,
		rPhase2: i * 2.7,
		sPhase: i * 0.7,
		aPhase: i * 1.9,
		aPhase2: i * 3.1,
		flap: (isBat ? 11 : 5) + (i % 4) * 1.3, // bats flutter fast
		flapPhase: i * 1.7,
		prevX: 0,
		prevZ: 0,
		hdg: 0,
		inited: false
	}));

	let groups = $state<THREE.Group[]>([]);
	let wingsL = $state<THREE.Group[]>([]);
	let wingsR = $state<THREE.Group[]>([]);

	let cx = 0; // orbit centre lags behind the player so the flock drifts in naturally
	let cz = 0;
	let t = 0;

	useTask((dt) => {
		// birds fly by day/sunset/fog; bats only at night/space. When inactive, hide and skip all work.
		const active = isBat
			? world.sky === 'night' || world.sky === 'space'
			: world.sky === 'day' || world.sky === 'sunset' || world.sky === 'fog';
		if (!active) {
			for (const g of groups) if (g) g.visible = false;
			return;
		}
		t += dt;
		const k = Math.min(1, dt * 0.4);
		cx += (playerState.pos[0] - cx) * k;
		cz += (playerState.pos[2] - cz) * k;
		for (let i = 0; i < COUNT; i++) {
			const b = birds[i];
			const g = groups[i];
			if (!g) continue;
			g.visible = true; // (re)show when active

			// irregular angular speed → the bird hurries and dawdles around the loop (not constant)
			b.angle += b.speed * (1 + 0.5 * Math.sin(t * 1.3 + b.sPhase)) * dt;
			// wandering radius → the loop bulges and pinches instead of being a clean circle
			const R = b.radius + Math.sin(t * 0.7 + b.rPhase) * 3.5 + Math.sin(t * 1.9 + b.rPhase2) * 1.6;
			const x = cx + Math.cos(b.angle) * R;
			const z = cz + Math.sin(b.angle) * R;
			// fluttering altitude → bobs and dips erratically
			const y =
				heightAt(x, z, world.terrain) +
				b.alt +
				Math.sin(t * 0.9 + b.aPhase) * 2 +
				Math.sin(t * 2.6 + b.aPhase2) * 0.9;
			g.position.set(x, y, z);

			// face the way it's actually moving (the wobble makes the path non-tangent to a circle)
			if (!b.inited) {
				b.prevX = x;
				b.prevZ = z;
				b.inited = true;
			}
			const vx = x - b.prevX;
			const vz = z - b.prevZ;
			b.prevX = x;
			b.prevZ = z;
			let d = (vx * vx + vz * vz > 1e-6 ? Math.atan2(vx, vz) : b.hdg) - b.hdg;
			while (d > Math.PI) d -= 2 * Math.PI;
			while (d < -Math.PI) d += 2 * Math.PI;
			b.hdg += d * Math.min(1, dt * 6); // smooth toward the travel direction
			g.rotation.y = b.hdg;
			g.rotation.z = THREE.MathUtils.clamp(d * 5, -0.5, 0.5); // bank into the turn

			const flap = Math.sin(t * b.flap + b.flapPhase) * 0.7;
			if (wingsL[i]) wingsL[i].rotation.z = flap;
			if (wingsR[i]) wingsR[i].rotation.z = -flap;
		}
	});
</script>

{#each birds as _, i (i)}
	<T.Group bind:ref={groups[i]} scale={isBat ? 0.5 : 1}>
		<T.Mesh geometry={bodyGeo} material={mat} />
		<T.Group bind:ref={wingsL[i]} position={[0.08, 0.05, 0]}>
			<T.Mesh geometry={wingGeoL} material={mat} />
		</T.Group>
		<T.Group bind:ref={wingsR[i]} position={[-0.08, 0.05, 0]}>
			<T.Mesh geometry={wingGeoR} material={mat} />
		</T.Group>
	</T.Group>
{/each}
