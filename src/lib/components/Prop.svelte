<script lang="ts">
	import { T, useTask } from '@threlte/core';
	import { RigidBody, Collider } from '@threlte/rapier';
	import * as THREE from 'three';
	import { kindDef } from '$lib/kinds';
	import { partGeo, propMat, rockMat, stoneMat, woodMat, flowerMat, foliageMat, LEAF_GREENS } from '$lib/sharedAssets';
	import type { WorldObject } from '$lib/world';

	let { obj }: { obj: WorldObject } = $props();

	// a flower's bloom takes a vivid colour from a wildflower palette, picked deterministically by its id (so a
	// scatter is a MIXED patch, stable across reloads); explicit paint overrides. Stem stays its green.
	const FLOWERS = ['#f0c020', '#ec5a8a', '#f4f1ea', '#b06ad8', '#e85a3a', '#5a8fe0', '#f08a30'];
	const flowerCol = $derived.by(() => {
		if (obj.kind !== 'flower') return '';
		if (obj.color) return obj.color;
		let h = 2166136261;
		for (let i = 0; i < obj.id.length; i++) ((h ^= obj.id.charCodeAt(i)), (h = Math.imul(h, 16777619)));
		return FLOWERS[(h >>> 0) % FLOWERS.length];
	});

	// a placed bush takes ONE green from the shared leaf palette, hashed by id (so a scatter of bushes is a
	// mixed thicket, stable across reloads — same idea as the ambient-scatter bushes); explicit paint overrides.
	const bushCol = $derived.by(() => {
		if (obj.kind !== 'bush') return '';
		if (obj.color) return obj.color;
		let h = 2166136261;
		for (let i = 0; i < obj.id.length; i++) ((h ^= obj.id.charCodeAt(i)), (h = Math.imul(h, 16777619)));
		return LEAF_GREENS[(h >>> 0) % LEAF_GREENS.length];
	});

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
					material={obj.kind === 'rock'
						? rockMat(obj.color ?? part.color)
						: obj.kind === 'well' && i === 0
							? stoneMat(obj.color ?? part.color) /* the well shaft → stone masonry */
							: obj.kind === 'fence' || obj.kind === 'bridge'
								? woodMat(obj.color ?? part.color) /* timber → weathered wood w/ knots */
								: obj.kind === 'flower' && i === 1
									? flowerMat(flowerCol) /* bloom → vivid varied petals + yellow eye */
									: obj.kind === 'bush'
										? foliageMat(bushCol) /* shrub → wind-swayed, dappled foliage (not a flat ball) */
										: propMat(obj.color ?? part.color, part.emissive)}
					castShadow
					receiveShadow
				/>
			{/each}
		</T.Group>
	</RigidBody>
</T.Group>
