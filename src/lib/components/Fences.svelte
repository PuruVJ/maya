<script lang="ts">
	// Town perimeter walls, INSTANCED. Every live fence panel in the world used to be its own <Prop> — a keyed
	// {#each} block carrying its own geometry, material refs, a Rapier RigidBody + Collider, and a pop-in tween.
	// A walled settlement is 100+ panels, and they mount/unmount as you cross the BUILD_SHOW/KEEP thresholds, so
	// approaching a town spiked draw calls AND thrashed Rapier with collider insertions mid-frame — the jitter the
	// user felt "since we first started work on fences", worst near civilisations. This draws ALL live fences in
	// TWO draw calls (posts + rails), rebuilt only when the fence set actually changes, and carries NO physics body
	// (the player's own push-out in Player.svelte keeps the wall solid; fences were never animal collision — see the
	// `fence-store-on-expansion` + `settlement-clean-zone` memories). Mirrors the AmbientScatter instancing template.
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { woodMat } from '$lib/sharedAssets';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();

	// the unit fence panel (kinds.ts `fence`): two BARK posts at local x = ±0.5, two #8a5a2b rails at y = 0.7/0.35.
	// Each box is centred at the origin here; its part-offset is baked into the per-instance matrix instead, so the
	// geometry is shared and the wood shader's local-space grain matches the old per-Prop meshes exactly.
	const POST = '#7c5230'; // BARK
	const RAIL = '#8a5a2b';
	const postGeo = new THREE.BoxGeometry(0.16, 1.0, 0.16);
	const railGeo = new THREE.BoxGeometry(1.4, 0.16, 0.1);
	// local part offsets within a panel (two instances per panel for each mesh)
	const POST_OFF: [number, number, number][] = [
		[0.5, 0.5, 0],
		[-0.5, 0.5, 0]
	];
	const RAIL_OFF: [number, number, number][] = [
		[0, 0.7, 0],
		[0, 0.35, 0]
	];

	const MAX_PANELS = 2000; // live panels are bounded by the region STRUCT budget; a huge ring is ~100-200 panels
	const MAX_INST = MAX_PANELS * 2; // two posts / two rails per panel
	const posts = new THREE.InstancedMesh(postGeo, woodMat(POST), MAX_INST);
	const rails = new THREE.InstancedMesh(railGeo, woodMat(RAIL), MAX_INST);
	posts.castShadow = rails.castShadow = true;
	posts.receiveShadow = rails.receiveShadow = true;
	posts.frustumCulled = rails.frustumCulled = false; // panels span a whole town; one bounds test would cull the lot
	posts.count = rails.count = 0;

	const panel = new THREE.Object3D(); // the panel's T·R·S frame
	const off = new THREE.Matrix4(); // a part's local offset within the panel
	const m = new THREE.Matrix4(); // panel.matrix · off → the part's world matrix

	let lastSig = ''; // rebuild only when the fence set changes (add / remove / region wake-swap)

	useTask(() => {
		// cheap O(objects) signature pass — fences never move, so count + a position fold catches every change
		// (add, decay-removal, a region sleeping/waking and swapping its stored panels) without a per-frame rebuild.
		let cnt = 0;
		let sig = 0;
		for (const o of world.objects) {
			if (o.kind !== 'fence') continue;
			cnt++;
			sig = (Math.imul(sig, 1000003) + ((o.pos[0] * 16 + o.pos[2]) | 0)) | 0;
		}
		const key = cnt + ':' + sig;
		if (key === lastSig) return;
		lastSig = key;

		let p = 0; // post instance write head
		let r = 0; // rail instance write head
		for (const o of world.objects) {
			if (o.kind !== 'fence') continue;
			if (p >= MAX_INST) break; // safety: never overrun the preallocated instance buffers
			const sx = o.scale?.[0] ?? 1;
			const sy = o.scale?.[1] ?? 1;
			const sz = o.scale?.[2] ?? 1;
			panel.position.set(o.pos[0], o.pos[1], o.pos[2]);
			panel.rotation.set(0, ((o.rot ?? 0) * Math.PI) / 180, 0); // rot stored in DEGREES (settlement_ops)
			panel.scale.set(sx, sy, sz);
			panel.updateMatrix();
			for (const t of POST_OFF) {
				off.makeTranslation(t[0], t[1], t[2]);
				m.multiplyMatrices(panel.matrix, off);
				posts.setMatrixAt(p++, m);
			}
			for (const t of RAIL_OFF) {
				off.makeTranslation(t[0], t[1], t[2]);
				m.multiplyMatrices(panel.matrix, off);
				rails.setMatrixAt(r++, m);
			}
		}
		posts.count = p;
		rails.count = r;
		posts.instanceMatrix.needsUpdate = true;
		rails.instanceMatrix.needsUpdate = true;
	});
</script>

<T is={posts} />
<T is={rails} />
