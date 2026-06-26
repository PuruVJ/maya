<script lang="ts">
	// Cemetery headstones, INSTANCED. A grave is a static 4-box prop (earth mound + headstone + cross upright/arms);
	// they were each their own keyed <Prop> (geometry, materials, a Rapier RigidBody, a pop-in tween) that mounts and
	// unmounts as you cross the reveal thresholds. They're capped at GRAVE_CAP (a small cemetery), so this is a small
	// perf win — but it's the same per-Prop mount/RigidBody churn the fences had, and instancing them keeps the static
	// sweep consistent. Four InstancedMeshes (one per part geometry+colour); the player still collides with a grave via
	// Player.svelte's own push-out on parts[0] (the mound = the real footprint), and animals avoid graves via the Rust
	// obstacle set (built separately from these meshes) — so dropping the per-grave RigidBody changes neither.
	// NB: grave `rot` is stored in RADIANS (Math.random()*2π) but the old <Prop> path converts it as DEGREES, so a grave
	// only ever turned ~0–6° — we REPLICATE that deg→rad here for exact visual parity (not a behaviour change).
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { propMat } from '$lib/sharedAssets';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();

	const MAX = 64; // safely above GRAVE_CAP (14) — trimGraves bounds the live count

	// the unit grave (kinds.ts `grave`): each box centred at the origin, its part-offset baked into the per-instance
	// matrix so the geometry is shared and the prop shader's local-space detail matches the old per-Prop meshes.
	const PARTS: { geo: THREE.BoxGeometry; off: [number, number, number]; col: string }[] = [
		{ geo: new THREE.BoxGeometry(0.6, 0.12, 0.92), off: [0, 0.06, 0], col: '#8a7660' }, // turned-earth mound
		{ geo: new THREE.BoxGeometry(0.46, 0.5, 0.12), off: [0, 0.3, -0.32], col: '#d2d3dc' }, // headstone
		{ geo: new THREE.BoxGeometry(0.1, 0.3, 0.09), off: [0, 0.64, -0.33], col: '#dcdde4' }, // cross — upright
		{ geo: new THREE.BoxGeometry(0.3, 0.1, 0.09), off: [0, 0.62, -0.33], col: '#dcdde4' } // cross — arms
	];
	const meshes = PARTS.map((p) => {
		const im = new THREE.InstancedMesh(p.geo, propMat(p.col), MAX);
		im.castShadow = true;
		im.receiveShadow = true;
		im.frustumCulled = false;
		im.count = 0;
		return im;
	});

	const g = new THREE.Object3D(); // the grave's T·R·S frame
	const off = new THREE.Matrix4(); // a part's local offset
	const m = new THREE.Matrix4(); // g.matrix · off → the part's world matrix
	let lastSig = '';

	useTask(() => {
		// rebuild only when the grave set changes (graves never move; a cheap count+position fold catches add/trim).
		let cnt = 0;
		let sig = 0;
		for (const o of world.objects) {
			if (o.kind !== 'grave') continue;
			cnt++;
			sig = (Math.imul(sig, 1000003) + ((o.pos[0] * 16 + o.pos[2]) | 0)) | 0;
		}
		const key = cnt + ':' + sig;
		if (key === lastSig) return;
		lastSig = key;

		let n = 0;
		for (const o of world.objects) {
			if (o.kind !== 'grave') continue;
			if (n >= MAX) break;
			const sx = o.scale?.[0] ?? 1;
			const sy = o.scale?.[1] ?? 1;
			const sz = o.scale?.[2] ?? 1;
			g.position.set(o.pos[0], o.pos[1], o.pos[2]);
			g.rotation.set(0, ((o.rot ?? 0) * Math.PI) / 180, 0);
			g.scale.set(sx, sy, sz);
			g.updateMatrix();
			for (let i = 0; i < PARTS.length; i++) {
				off.makeTranslation(PARTS[i].off[0], PARTS[i].off[1], PARTS[i].off[2]);
				m.multiplyMatrices(g.matrix, off);
				meshes[i].setMatrixAt(n, m);
			}
			n++;
		}
		for (const im of meshes) {
			im.count = n;
			im.instanceMatrix.needsUpdate = true;
		}
	});
</script>

{#each meshes as im (im.uuid)}
	<T is={im} />
{/each}
