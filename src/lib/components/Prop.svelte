<script lang="ts">
	import { T, useTask } from '@threlte/core';
	import { RigidBody, Collider } from '@threlte/rapier';
	import * as THREE from 'three';
	import { kindDef } from '$lib/kinds';
	import { partGeo, propMat } from '$lib/sharedAssets';
	import type { WorldObject } from '$lib/world';

	let { obj }: { obj: WorldObject } = $props();

	// Generic FALLBACK prop renderer. Every concrete object kind now has a dedicated renderer (Tree/Building/Lamp/
	// Npc/Critter or one of the InstancedMesh renderers Fences/Graves/Rocks/Flowers/Bushes/Wells/Bridges), so this
	// only fires for an unrecognised kind — a plain propMat box/sphere with a primitive collider. Kept as a safety net.

	const def = $derived(kindDef(obj.kind));
	const sx = $derived(obj.scale?.[0] ?? 1);
	const sy = $derived(obj.scale?.[1] ?? 1);
	const sz = $derived(obj.scale?.[2] ?? 1);
	const rot = $derived(((obj.rot ?? 0) * Math.PI) / 180);

	// pop-in: spring up from nothing with a little overshoot when first revealed (easeOutBack)
	let model = $state<THREE.Group>();
	let t = 0;
	const eob = (x: number) => {
		const c1 = 1.70158;
		const c3 = c1 + 1;
		return 1 + c3 * (x - 1) ** 3 + c1 * (x - 1) ** 2;
	};
	// drive the spring until it settles, then STOP the task: a prop that's done popping in shouldn't keep a
	// per-frame callback alive forever (a big scene has hundreds → pure task-scheduler overhead each frame).
	const { stop } = useTask((dt) => {
		if (!model) return;
		t = Math.min(1, t + dt * 3.5);
		const s = eob(t);
		model.scale.set(sx * s, sy * s, sz * s);
		if (t >= 1) stop();
	});
</script>

<T.Group position={[obj.pos[0], obj.pos[1], obj.pos[2]]} rotation={[0, rot, 0]} userData={{ objectId: obj.id }}>
	<RigidBody type="fixed">
		<!-- primitive collider from the kinds registry (stays a primitive even after GLB swap) -->
		{#if def.col === 'box'}
			<T.Group position={[0, (def.h * sy) / 2, 0]}>
				<Collider shape="cuboid" args={[def.r * sx, (def.h * sy) / 2, def.r * sz]} />
			</T.Group>
		{:else if def.col === 'ball'}
			<T.Group position={[0, def.r * sy, 0]}>
				<Collider shape="ball" args={[def.r * sx]} />
			</T.Group>
		{:else}
			<T.Group position={[0, (def.h * sy) / 2, 0]}>
				<Collider shape="cylinder" args={[(def.h * sy) / 2, def.r * sx]} />
			</T.Group>
		{/if}

		<!-- composed model — SHARED cached geometry + material; scale starts at 0 and pops in -->
		<T.Group bind:ref={model} scale={[0, 0, 0]}>
			{#each def.parts as part, i (i)}
				<T.Mesh
					position={part.pos}
					geometry={partGeo(part)}
					material={propMat(obj.color ?? part.color, part.emissive)}
					castShadow
					receiveShadow
				/>
			{/each}
		</T.Group>
	</RigidBody>
</T.Group>
