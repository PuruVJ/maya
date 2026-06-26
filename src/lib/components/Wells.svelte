<script lang="ts">
	// Village wells, INSTANCED. A well is a stone shaft + two timber posts + a little pyramid roof; each was its own
	// keyed <Prop>. Wells are sparse (settlers dig ~one per town) so this is mostly a consistency / draw-call win.
	// Three InstancedMeshes: shaft (stoneMat), posts (propMat BARK, 2 per well), roof (propMat ROOF). Fixed part
	// colours — wells are structural and effectively never painted, so no per-instance colour (the graves approach).
	// Player collides via Player.svelte's round push-out (well col='cyl' → def.r, independent of the dropped
	// RigidBody); animals avoid via the Rust obstacle set. NB the well also feeds the thirst sim via feedWaterSources
	// in Scene (built from world.objects, unaffected by how the well is drawn).
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { stoneMat, propMat } from '$lib/sharedAssets';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();

	const MAX = 64; // wells are sparse; bounded by the region STRUCT budget
	const STONE = '#9a9aa2';
	const BARK = '#7c5230';
	const ROOF = '#5a3b30';

	// kinds.ts `well`: shaft cyl, two posts box, pyramid roof (cone-4 rotated 45°, matching partGeo). Part-offsets
	// baked into the per-instance matrix; box centred at origin.
	const shaftGeo = new THREE.CylinderGeometry(1.0, 1.0, 1.0, 12);
	const postGeo = new THREE.BoxGeometry(0.12, 1.3, 0.12);
	const roofGeo = new THREE.ConeGeometry(1.3, 0.7, 4).rotateY(Math.PI / 4);
	const shaft = new THREE.InstancedMesh(shaftGeo, stoneMat(STONE), MAX);
	const posts = new THREE.InstancedMesh(postGeo, propMat(BARK), MAX * 2); // two posts per well
	const roof = new THREE.InstancedMesh(roofGeo, propMat(ROOF), MAX);
	for (const im of [shaft, posts, roof]) {
		im.castShadow = true;
		im.receiveShadow = true;
		im.frustumCulled = false;
		im.count = 0;
	}
	const SHAFT_OFF: [number, number, number] = [0, 0.5, 0];
	const ROOF_OFF: [number, number, number] = [0, 2.3, 0];
	const POST_OFFS: [number, number, number][] = [
		[0.8, 1.4, 0],
		[-0.8, 1.4, 0]
	];

	const w = new THREE.Object3D(); // the well's T·R·S frame
	const off = new THREE.Matrix4(); // a part's local offset
	const m = new THREE.Matrix4(); // w.matrix · off → the part's world matrix
	let lastSig = '';

	useTask(() => {
		// rebuild only when the well set changes (wells never move; a cheap count+position fold).
		let cnt = 0;
		let sig = 0;
		for (const o of world.objects) {
			if (o.kind !== 'well') continue;
			cnt++;
			sig = (Math.imul(sig, 1000003) + ((o.pos[0] * 16 + o.pos[2]) | 0)) | 0;
		}
		const key = cnt + ':' + sig;
		if (key === lastSig) return;
		lastSig = key;

		let n = 0; // wells written (shaft/roof index)
		let pp = 0; // post instance write head
		for (const o of world.objects) {
			if (o.kind !== 'well') continue;
			if (n >= MAX) break;
			const sx = o.scale?.[0] ?? 1;
			const sy = o.scale?.[1] ?? 1;
			const sz = o.scale?.[2] ?? 1;
			w.position.set(o.pos[0], o.pos[1], o.pos[2]);
			w.rotation.set(0, ((o.rot ?? 0) * Math.PI) / 180, 0);
			w.scale.set(sx, sy, sz);
			w.updateMatrix();
			off.makeTranslation(SHAFT_OFF[0], SHAFT_OFF[1], SHAFT_OFF[2]);
			m.multiplyMatrices(w.matrix, off);
			shaft.setMatrixAt(n, m);
			off.makeTranslation(ROOF_OFF[0], ROOF_OFF[1], ROOF_OFF[2]);
			m.multiplyMatrices(w.matrix, off);
			roof.setMatrixAt(n, m);
			for (const t of POST_OFFS) {
				off.makeTranslation(t[0], t[1], t[2]);
				m.multiplyMatrices(w.matrix, off);
				posts.setMatrixAt(pp++, m);
			}
			n++;
		}
		shaft.count = n;
		roof.count = n;
		posts.count = pp;
		shaft.instanceMatrix.needsUpdate = true;
		roof.instanceMatrix.needsUpdate = true;
		posts.instanceMatrix.needsUpdate = true;
	});
</script>

<T is={shaft} />
<T is={posts} />
<T is={roof} />
