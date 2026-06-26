<script lang="ts">
	// Wildflowers, INSTANCED. A scatter drops a patch of them; each was its own keyed <Prop> (stem + bloom mesh, a
	// Rapier collider, a pop-in). Two InstancedMeshes (stem + bloom). The bloom takes a vivid per-flower colour —
	// obj.color if painted, else a deterministic pick from a 7-colour palette hashed by id (so a scatter is a MIXED,
	// reload-stable patch) — driven by instanceColor on a white-base flowerMat (the yellow eye + petal lobes apply on
	// top). The stem stays leaf-green (or the paint colour). Player collides via Player.svelte's round push-out
	// (flower col='cyl' → def.r, independent of the dropped RigidBody); animals avoid via the Rust obstacle set.
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { propMat, flowerMat } from '$lib/sharedAssets';
	import type { World, WorldObject } from '$lib/world';

	let { world }: { world: World } = $props();

	const MAX = 2000; // scatter patches can be large; bounded by the region STRUCT budget
	const LEAF = '#3f8f4a';
	const FLOWERS = ['#f0c020', '#ec5a8a', '#f4f1ea', '#b06ad8', '#e85a3a', '#5a8fe0', '#f08a30'];

	// the per-flower bloom colour — REPLICATES Prop.svelte's flowerCol (FNV-1a over the id) so an instanced patch
	// matches the per-Prop one exactly; explicit paint overrides.
	function bloomColOf(o: WorldObject): string {
		if (o.color) return o.color;
		let h = 2166136261;
		for (let i = 0; i < o.id.length; i++) {
			h ^= o.id.charCodeAt(i);
			h = Math.imul(h, 16777619);
		}
		return FLOWERS[(h >>> 0) % FLOWERS.length];
	}

	// kinds.ts `flower`: stem (cyl) + bloom (sphere), matching partGeo's tessellation; box centred at origin, the
	// part-offset baked into the per-instance matrix. Stem = propMat (leaf/paint), bloom = flowerMat (palette/paint).
	const stemGeo = new THREE.CylinderGeometry(0.05, 0.05, 0.5, 12);
	const bloomGeo = new THREE.SphereGeometry(0.18, 12, 10);
	const stem = new THREE.InstancedMesh(stemGeo, propMat('#ffffff'), MAX);
	const bloom = new THREE.InstancedMesh(bloomGeo, flowerMat('#ffffff'), MAX);
	for (const im of [stem, bloom]) {
		im.castShadow = true;
		im.receiveShadow = true;
		im.frustumCulled = false;
		im.count = 0;
	}
	const STEM_OFF: [number, number, number] = [0, 0.25, 0];
	const BLOOM_OFF: [number, number, number] = [0, 0.55, 0];

	const f = new THREE.Object3D(); // the flower's T·R·S frame
	const off = new THREE.Matrix4(); // a part's local offset
	const m = new THREE.Matrix4(); // f.matrix · off → the part's world matrix
	const col = new THREE.Color(); // reused → no per-flower alloc
	let lastSig = '';

	useTask(() => {
		// rebuild only when the flower set changes (flowers never move; fold position + a colour code).
		let cnt = 0;
		let sig = 0;
		for (const o of world.objects) {
			if (o.kind !== 'flower') continue;
			cnt++;
			const ch = o.color ? o.color.charCodeAt(1) * 7 + o.color.length : 0;
			sig = (Math.imul(sig, 1000003) + ((o.pos[0] * 16 + o.pos[2]) | 0) + ch) | 0;
		}
		const key = cnt + ':' + sig;
		if (key === lastSig) return;
		lastSig = key;

		let n = 0;
		for (const o of world.objects) {
			if (o.kind !== 'flower') continue;
			if (n >= MAX) break;
			const sx = o.scale?.[0] ?? 1;
			const sy = o.scale?.[1] ?? 1;
			const sz = o.scale?.[2] ?? 1;
			f.position.set(o.pos[0], o.pos[1], o.pos[2]);
			f.rotation.set(0, ((o.rot ?? 0) * Math.PI) / 180, 0);
			f.scale.set(sx, sy, sz);
			f.updateMatrix();
			off.makeTranslation(STEM_OFF[0], STEM_OFF[1], STEM_OFF[2]);
			m.multiplyMatrices(f.matrix, off);
			stem.setMatrixAt(n, m);
			stem.setColorAt(n, col.set(o.color ?? LEAF)); // a painted flower colours the stem too; else leaf-green
			off.makeTranslation(BLOOM_OFF[0], BLOOM_OFF[1], BLOOM_OFF[2]);
			m.multiplyMatrices(f.matrix, off);
			bloom.setMatrixAt(n, m);
			bloom.setColorAt(n, col.set(bloomColOf(o)));
			n++;
		}
		for (const im of [stem, bloom]) {
			im.count = n;
			im.instanceMatrix.needsUpdate = true;
			if (im.instanceColor) im.instanceColor.needsUpdate = true;
		}
	});
</script>

<T is={stem} />
<T is={bloom} />
