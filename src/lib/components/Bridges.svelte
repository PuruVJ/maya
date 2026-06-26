<script lang="ts">
	// Plank bridges, INSTANCED VISUAL + kept colliders. A bridge is a deck + two rails (timber). Unlike the other
	// instanced props, a bridge is WALKABLE — you cross water on it, and the player stands on the deck via a Rapier
	// collider (the terrain floor is analytic; objects above are resolved by the character controller). So the visual
	// is instanced (deck + rails, woodMat) but each bridge KEEPS a fixed deck collider — matching the old <Prop> box
	// collider exactly (half-extents def.r×def.h/2×def.r at y=def.h/2) so walkability is byte-for-byte unchanged. The
	// collider list is rebuilt in the SAME useTask as the matrices (only when the bridge set changes), so visual and
	// physics stay in lockstep without assuming deep reactivity. Player's manual box push-out (Player.svelte, over
	// world.objects) still keeps you off the deck edge at ground level — unchanged, bridges remain in world.objects.
	import { T, useTask } from '@threlte/core';
	import { RigidBody, Collider } from '@threlte/rapier';
	import * as THREE from 'three';
	import { woodMat } from '$lib/sharedAssets';
	import type { World, WorldObject } from '$lib/world';

	let { world }: { world: World } = $props();

	const MAX = 64; // bridges are sparse (water crossings)
	const DECK = '#8a5a2b';
	const BARK = '#7c5230';
	const R = 2; // kinds.ts bridge def.r — the box-collider half-width (matches the old Prop collider)
	const H = 0.6; // kinds.ts bridge def.h — collider full height

	// kinds.ts `bridge`: deck box + two rail boxes; part-offsets baked into the per-instance matrix.
	const deckGeo = new THREE.BoxGeometry(3.6, 0.25, 1.6);
	const railGeo = new THREE.BoxGeometry(3.6, 0.4, 0.12);
	const deck = new THREE.InstancedMesh(deckGeo, woodMat(DECK), MAX);
	const rails = new THREE.InstancedMesh(railGeo, woodMat(BARK), MAX * 2); // two rails per bridge
	for (const im of [deck, rails]) {
		im.castShadow = true;
		im.receiveShadow = true;
		im.frustumCulled = false;
		im.count = 0;
	}
	const DECK_OFF: [number, number, number] = [0, 0.2, 0];
	const RAIL_OFFS: [number, number, number][] = [
		[0, 0.5, 0.74],
		[0, 0.5, -0.74]
	];

	const g = new THREE.Object3D(); // the bridge's T·R·S frame
	const off = new THREE.Matrix4(); // a part's local offset
	const m = new THREE.Matrix4(); // g.matrix · off → the part's world matrix
	let lastSig = '';
	let bridges = $state<WorldObject[]>([]); // drives the collider {#each}; rebuilt only when the set changes

	useTask(() => {
		// rebuild only when the bridge set changes (bridges never move; a cheap count+position fold).
		let cnt = 0;
		let sig = 0;
		for (const o of world.objects) {
			if (o.kind !== 'bridge') continue;
			cnt++;
			sig = (Math.imul(sig, 1000003) + ((o.pos[0] * 16 + o.pos[2]) | 0)) | 0;
		}
		const key = cnt + ':' + sig;
		if (key === lastSig) return;
		lastSig = key;

		const list: WorldObject[] = [];
		let d = 0; // deck instance head
		let rr = 0; // rail instance head
		for (const o of world.objects) {
			if (o.kind !== 'bridge') continue;
			if (d >= MAX) break;
			list.push(o);
			const sx = o.scale?.[0] ?? 1;
			const sy = o.scale?.[1] ?? 1;
			const sz = o.scale?.[2] ?? 1;
			g.position.set(o.pos[0], o.pos[1], o.pos[2]);
			g.rotation.set(0, ((o.rot ?? 0) * Math.PI) / 180, 0);
			g.scale.set(sx, sy, sz);
			g.updateMatrix();
			off.makeTranslation(DECK_OFF[0], DECK_OFF[1], DECK_OFF[2]);
			m.multiplyMatrices(g.matrix, off);
			deck.setMatrixAt(d, m);
			for (const t of RAIL_OFFS) {
				off.makeTranslation(t[0], t[1], t[2]);
				m.multiplyMatrices(g.matrix, off);
				rails.setMatrixAt(rr++, m);
			}
			d++;
		}
		deck.count = d;
		rails.count = rr;
		deck.instanceMatrix.needsUpdate = true;
		rails.instanceMatrix.needsUpdate = true;
		bridges = list;
	});
</script>

<T is={deck} />
<T is={rails} />
<!-- walkable deck colliders (no mesh) — replicates the old Prop box collider so you still cross water on them -->
{#each bridges as o (o.id)}
	<T.Group position={[o.pos[0], o.pos[1], o.pos[2]]} rotation={[0, ((o.rot ?? 0) * Math.PI) / 180, 0]}>
		<RigidBody type="fixed">
			<T.Group position={[0, (H * (o.scale?.[1] ?? 1)) / 2, 0]}>
				<Collider
					shape="cuboid"
					args={[R * (o.scale?.[0] ?? 1), (H * (o.scale?.[1] ?? 1)) / 2, R * (o.scale?.[2] ?? 1)]}
				/>
			</T.Group>
		</RigidBody>
	</T.Group>
{/each}
