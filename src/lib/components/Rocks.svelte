<script lang="ts">
	// Scattered boulders, INSTANCED. The worldgen grammar can `scatter rock count:40`, so a generated world holds
	// many — each was its own keyed <Prop> (two sphere meshes, a Rapier RigidBody, a pop-in). Two InstancedMeshes
	// (big + small sphere) draw the lot. Per-rock COLOUR varies (a painted rock sets obj.color; otherwise the two
	// spheres use their own greys) so we drive colour with instanceColor on a white base material — exactly the
	// AmbientScatter canopy trick — and the rock shader's AO/grain/lichen apply on top. The player still collides via
	// Player.svelte's ROUND push-out (rock col='ball' → def.r, independent of the dropped RigidBody) and animals avoid
	// rocks via the Rust obstacle set, so removing the per-rock RigidBody changes neither.
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { rockMat } from '$lib/sharedAssets';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();

	const MAX = 1000; // scatter ops + multiple regions → low-hundreds live; bounded by the region STRUCT budget

	// kinds.ts `rock`: two spheres (12×10 segs, matching partGeo). Box centred at origin; the part-offset is baked
	// into the per-instance matrix so the geometry is shared and the rock shader's local-space grain matches Prop.
	const PARTS: { geo: THREE.SphereGeometry; off: [number, number, number]; col: string }[] = [
		{ geo: new THREE.SphereGeometry(0.85, 12, 10), off: [0, 0.4, 0], col: '#8c8c92' },
		{ geo: new THREE.SphereGeometry(0.5, 12, 10), off: [0.5, 0.3, 0.2], col: '#7e7e86' }
	];
	const meshes = PARTS.map((p) => {
		const im = new THREE.InstancedMesh(p.geo, rockMat('#ffffff'), MAX); // white base → per-instance colour
		im.castShadow = true;
		im.receiveShadow = true;
		im.frustumCulled = false;
		im.count = 0;
		return im;
	});

	const r = new THREE.Object3D(); // the rock's T·R·S frame
	const off = new THREE.Matrix4(); // a sphere's local offset
	const m = new THREE.Matrix4(); // r.matrix · off → the sphere's world matrix
	const col = new THREE.Color(); // reused → no per-rock alloc
	let lastSig = '';

	useTask(() => {
		// rebuild only when the rock set changes (rocks never move; fold position + a cheap colour code so a repaint
		// or an add/remove rebuilds, but a steady world doesn't churn the instance buffers every frame).
		let cnt = 0;
		let sig = 0;
		for (const o of world.objects) {
			if (o.kind !== 'rock') continue;
			cnt++;
			const ch = o.color ? o.color.charCodeAt(1) * 7 + o.color.length : 0;
			sig = (Math.imul(sig, 1000003) + ((o.pos[0] * 16 + o.pos[2]) | 0) + ch) | 0;
		}
		const key = cnt + ':' + sig;
		if (key === lastSig) return;
		lastSig = key;

		let n = 0;
		for (const o of world.objects) {
			if (o.kind !== 'rock') continue;
			if (n >= MAX) break;
			const sx = o.scale?.[0] ?? 1;
			const sy = o.scale?.[1] ?? 1;
			const sz = o.scale?.[2] ?? 1;
			r.position.set(o.pos[0], o.pos[1], o.pos[2]);
			r.rotation.set(0, ((o.rot ?? 0) * Math.PI) / 180, 0);
			r.scale.set(sx, sy, sz);
			r.updateMatrix();
			for (let i = 0; i < PARTS.length; i++) {
				off.makeTranslation(PARTS[i].off[0], PARTS[i].off[1], PARTS[i].off[2]);
				m.multiplyMatrices(r.matrix, off);
				meshes[i].setMatrixAt(n, m);
				col.set(o.color ?? PARTS[i].col); // a painted rock recolours both spheres; else each keeps its own grey
				meshes[i].setColorAt(n, col);
			}
			n++;
		}
		for (const im of meshes) {
			im.count = n;
			im.instanceMatrix.needsUpdate = true;
			if (im.instanceColor) im.instanceColor.needsUpdate = true;
		}
	});
</script>

{#each meshes as im (im.uuid)}
	<T is={im} />
{/each}
