<script lang="ts">
	// Placed shrubs, INSTANCED. A bush is three overlapping spheres; each was its own keyed <Prop>. All three lobes
	// share ONE per-bush green — obj.color if painted, else a deterministic pick from the leaf palette hashed by id
	// (so a scatter is a mixed, reload-stable thicket) — driven by instanceColor on a white-base foliageMat (the
	// static leaf-dapple + darker-underside apply on top; the wind sway was REMOVED, see sharedAssets.foliageMat).
	// Three InstancedMeshes (one per lobe). Player collides via Player.svelte's round push-out (bush col='ball' →
	// def.r, independent of the dropped RigidBody); animals avoid via the Rust obstacle set.
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { foliageMat, LEAF_GREENS } from '$lib/sharedAssets';
	import type { World, WorldObject } from '$lib/world';

	let { world }: { world: World } = $props();

	const MAX = 2000; // scatter thickets can be large; bounded by the region STRUCT budget

	// REPLICATES Prop.svelte's bushCol (FNV-1a over the id → LEAF_GREENS) so an instanced thicket matches verbatim.
	function bushColOf(o: WorldObject): string {
		if (o.color) return o.color;
		let h = 2166136261;
		for (let i = 0; i < o.id.length; i++) {
			h ^= o.id.charCodeAt(i);
			h = Math.imul(h, 16777619);
		}
		return LEAF_GREENS[(h >>> 0) % LEAF_GREENS.length];
	}

	// kinds.ts `bush`: three spheres (12×10 segs, matching partGeo); part-offsets baked into the per-instance matrix.
	const PARTS: { geo: THREE.SphereGeometry; off: [number, number, number] }[] = [
		{ geo: new THREE.SphereGeometry(0.6, 12, 10), off: [0, 0.5, 0] },
		{ geo: new THREE.SphereGeometry(0.45, 12, 10), off: [0.35, 0.4, 0.1] },
		{ geo: new THREE.SphereGeometry(0.4, 12, 10), off: [-0.3, 0.45, -0.1] }
	];
	const meshes = PARTS.map((p) => {
		const im = new THREE.InstancedMesh(p.geo, foliageMat('#ffffff'), MAX); // white base → per-bush colour
		im.castShadow = true;
		im.receiveShadow = true;
		im.frustumCulled = false;
		im.count = 0;
		return im;
	});

	const b = new THREE.Object3D(); // the bush's T·R·S frame
	const off = new THREE.Matrix4(); // a lobe's local offset
	const m = new THREE.Matrix4(); // b.matrix · off → the lobe's world matrix
	const col = new THREE.Color(); // reused → no per-bush alloc
	let lastSig = '';

	useTask(() => {
		// rebuild only when the bush set changes (bushes never move; fold position + a colour code).
		let cnt = 0;
		let sig = 0;
		for (const o of world.objects) {
			if (o.kind !== 'bush') continue;
			cnt++;
			const ch = o.color ? o.color.charCodeAt(1) * 7 + o.color.length : 0;
			sig = (Math.imul(sig, 1000003) + ((o.pos[0] * 16 + o.pos[2]) | 0) + ch) | 0;
		}
		const key = cnt + ':' + sig;
		if (key === lastSig) return;
		lastSig = key;

		let n = 0;
		for (const o of world.objects) {
			if (o.kind !== 'bush') continue;
			if (n >= MAX) break;
			const sx = o.scale?.[0] ?? 1;
			const sy = o.scale?.[1] ?? 1;
			const sz = o.scale?.[2] ?? 1;
			b.position.set(o.pos[0], o.pos[1], o.pos[2]);
			b.rotation.set(0, ((o.rot ?? 0) * Math.PI) / 180, 0);
			b.scale.set(sx, sy, sz);
			b.updateMatrix();
			col.set(bushColOf(o)); // one green for all three lobes of this bush
			for (let i = 0; i < PARTS.length; i++) {
				off.makeTranslation(PARTS[i].off[0], PARTS[i].off[1], PARTS[i].off[2]);
				m.multiplyMatrices(b.matrix, off);
				meshes[i].setMatrixAt(n, m);
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
