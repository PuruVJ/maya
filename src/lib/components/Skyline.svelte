<script lang="ts">
	// Distant procedural CITIES on the horizon — world-seeded points-of-interest that hint "there's a place
	// over there" as you roam, so the world feels vast and populated (the player's own builds + `make city`
	// fill the near field). World-stable: POIs are hashed by an absolute coarse cell, so a given skyline
	// stays put in the world and grows/parallaxes correctly as you walk toward it — only the far rim changes
	// when you cross a rebuild boundary. They live in a FAR band (NEAR…RADIUS) and fade out as you approach,
	// so you never walk into fake boxes — the city stays a tantalising silhouette. Ambient/decorative: one
	// InstancedMesh, not saved or shared, no collision. Same follow-the-player pattern as AmbientScatter.
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { heightAt } from '$lib/terrain';
	import { playerState } from '$lib/playerState.svelte';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();

	const MAX = 800; // instance cap (a few POIs × ~14 buildings each)
	const CELL = 380; // one candidate POI per this coarse cell
	const RADIUS = 620; // POIs render out to here (fogged beyond)
	const NEAR = 190; // closer than this → fully faded (you never reach the silhouette)
	const FADE = 110; // fade band just outside NEAR
	const REBUILD = 80; // re-place only when the player crosses an 80 m cell

	const geo = new THREE.BoxGeometry(1, 1, 1).translate(0, 0.5, 0); // base at ground; scale.y = height
	const city = new THREE.InstancedMesh(geo, new THREE.MeshStandardMaterial({ color: '#48526b', flatShading: true }), MAX);
	city.castShadow = false;
	city.receiveShadow = false;
	city.frustumCulled = false;
	city.count = 0;
	const dummy = new THREE.Object3D();

	const hash = (i: number, j: number, s: number) => {
		const v = Math.sin(i * 127.1 + j * 311.7 + s * 74.7) * 43758.5453;
		return v - Math.floor(v);
	};

	let lastCx = NaN;
	let lastCz = NaN;
	let lastLen = -1;

	useTask(() => {
		const pcx = Math.round(playerState.pos[0] / REBUILD) * REBUILD;
		const pcz = Math.round(playerState.pos[2] / REBUILD) * REBUILD;
		const len = world.terrain.length;
		if (pcx === lastCx && pcz === lastCz && len === lastLen) return;
		lastCx = pcx;
		lastCz = pcz;
		lastLen = len;

		const px = playerState.pos[0];
		const pz = playerState.pos[2];
		const c0 = Math.floor((px - RADIUS) / CELL);
		const c1 = Math.floor((px + RADIUS) / CELL);
		const d0 = Math.floor((pz - RADIUS) / CELL);
		const d1 = Math.floor((pz + RADIUS) / CELL);
		let n = 0;
		for (let ci = c0; ci <= c1 && n < MAX; ci++) {
			for (let cj = d0; cj <= d1 && n < MAX; cj++) {
				if (hash(ci, cj, 1) > 0.32) continue; // only ~1/3 of cells hold a POI
				// POI centre (jittered within the cell), and how far it is from the player
				const poiX = ci * CELL + (hash(ci, cj, 2) - 0.5) * CELL * 0.6;
				const poiZ = cj * CELL + (hash(ci, cj, 3) - 0.5) * CELL * 0.6;
				const dist = Math.hypot(poiX - px, poiZ - pz);
				if (dist > RADIUS) continue;
				// fade in over NEAR…NEAR+FADE → 0 when close (never reach it), 1 far out
				const t = (dist - NEAR) / FADE;
				const fade = t <= 0 ? 0 : t >= 1 ? 1 : t * t * (3 - 2 * t);
				if (fade <= 0) continue;

				const groundY = heightAt(poiX, poiZ, world.terrain);
				const buildings = 9 + Math.floor(hash(ci, cj, 4) * 7); // 9–15 buildings → a little skyline
				for (let k = 0; k < buildings && n < MAX; k++) {
					const bx = poiX + (hash(ci, k, 5) - 0.5) * 56;
					const bz = poiZ + (hash(ci, k + 7, 6) - 0.5) * 56;
					const w = 6 + hash(cj, k, 7) * 6;
					const h = (7 + hash(cj, k + 3, 8) ** 2 * 34) * fade; // squared → a few stand tall (towers)
					dummy.position.set(bx, groundY, bz);
					dummy.scale.set(w, h, w);
					dummy.rotation.set(0, hash(ci + k, cj, 9) * 6.283, 0);
					dummy.updateMatrix();
					city.setMatrixAt(n++, dummy.matrix);
				}
			}
		}
		city.count = n;
		city.instanceMatrix.needsUpdate = true;
	});
</script>

<T is={city} />
