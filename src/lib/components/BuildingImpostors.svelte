<script lang="ts">
	// Your REAL distant buildings, drawn as cheap silhouette boxes so a town reads as rough shapes from across
	// the map instead of POPPING into existence as you walk up to it (the detailed Building mounts only within
	// the near reveal radius — that hard edge was the "house appears out of nothing" the player hated). One
	// InstancedMesh → a single draw call for the whole far skyline of the player's own city. Follows the player
	// like AmbientScatter/Skyline; recomputes only when you move a cell or the building set changes. Decorative
	// (no collision, not saved) — it just mirrors the building objects that already exist.
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { heightAt } from '$lib/terrain';
	import { playerState } from '$lib/playerState.svelte';
	import { kindDef } from '$lib/kinds';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();

	const BUILDING_KINDS = new Set(['house', 'cabin', 'tower']);
	const MAX = 600; // instance cap (a large city)
	// NEAR is just INSIDE the detailed reveal radius (SHOW_R2 = 125 m in Scene) so the box and the real building
	// overlap rather than leave a gap → the silhouette is already standing there when the detail mounts, so the
	// transition reads as "the shape resolves into a house", never "a house blinks into being".
	const NEAR2 = 120 * 120;
	const FAR2 = 650 * 650; // beyond this it's lost to the fog
	const MOVE2 = 8 * 8; // recompute only when the player crosses ~8 m (cheap; the far skyline barely changes)

	const geo = new THREE.BoxGeometry(1, 1, 1).translate(0, 0.5, 0); // base sits on the ground; scale.y = height
	const mat = new THREE.MeshStandardMaterial({ color: '#414b63', flatShading: true }); // cool slate → reads as a fogged silhouette, catches moonlight
	const mesh = new THREE.InstancedMesh(geo, mat, MAX);
	mesh.castShadow = false;
	mesh.receiveShadow = false;
	mesh.frustumCulled = false;
	mesh.count = 0;
	const dummy = new THREE.Object3D();

	let lastX = NaN;
	let lastZ = NaN;
	let lastLen = -1;
	useTask(() => {
		const px = playerState.pos[0];
		const pz = playerState.pos[2];
		const len = world.objects.length;
		if (!Number.isNaN(lastX) && len === lastLen && (px - lastX) ** 2 + (pz - lastZ) ** 2 < MOVE2) return;
		lastX = px;
		lastZ = pz;
		lastLen = len;
		let n = 0;
		for (const o of world.objects) {
			if (!BUILDING_KINDS.has(o.kind)) continue;
			const d2 = (o.pos[0] - px) ** 2 + (o.pos[2] - pz) ** 2;
			if (d2 < NEAR2 || d2 > FAR2) continue; // near → the detailed building draws it; far → fogged out
			if (n >= MAX) break;
			const def = kindDef(o.kind);
			dummy.position.set(o.pos[0], heightAt(o.pos[0], o.pos[2], world.terrain), o.pos[2]);
			dummy.rotation.set(0, o.rot ?? 0, 0);
			dummy.scale.set((o.scale?.[0] ?? 1) * def.r * 2, (o.scale?.[1] ?? 1) * def.h, (o.scale?.[2] ?? 1) * def.r * 2);
			dummy.updateMatrix();
			mesh.setMatrixAt(n, dummy.matrix);
			n++;
		}
		mesh.count = n;
		mesh.instanceMatrix.needsUpdate = true;
	});
</script>

<T is={mesh} />
