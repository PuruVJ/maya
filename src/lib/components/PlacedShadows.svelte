<script lang="ts">
	// Contact-shadow blobs under PLACED round objects (trees/pines/rocks/wells/lamps/props). The directional
	// light only budgets real shadows within the player-following frustum (±60 m), so placed forests/props
	// beyond that float; a soft dark disc grounds them everywhere in ONE draw call. STATIC — rebuilt only when
	// world.objects changes (add/remove/move), not per frame (unlike CreatureShadows). Box-footprint kinds
	// (houses/cabins) are skipped: they have real near shadows + base weathering, and a round blob under a
	// rectangular house reads wrong. NOTE the instanceMatrix in the vertex shader (raw ShaderMaterial on an
	// InstancedMesh must apply it itself, else every blob stacks at the world origin — the CreatureShadows bug).
	import { T } from '@threlte/core';
	import * as THREE from 'three';
	import { heightAt } from '$lib/terrain';
	import { kindDef } from '$lib/kinds';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();
	const CREATURES = new Set(['person', 'cat', 'lion', 'rabbit', 'kangaroo', 'dinosaur']);

	const MAX = 1024;
	const geo = new THREE.CircleGeometry(1, 12).rotateX(-Math.PI / 2);
	const mat = new THREE.ShaderMaterial({
		transparent: true,
		depthWrite: false,
		vertexShader: /* glsl */ `varying vec2 vUv; void main(){ vUv = uv; gl_Position = projectionMatrix * modelViewMatrix * instanceMatrix * vec4(position, 1.0); }`,
		fragmentShader: /* glsl */ `varying vec2 vUv; void main(){ float r = length(vUv - 0.5) * 2.0; float a = smoothstep(1.0, 0.2, r) * 0.30; gl_FragColor = vec4(0.0, 0.0, 0.0, a); }`
	});
	const blobs = new THREE.InstancedMesh(geo, mat, MAX);
	blobs.frustumCulled = false;
	blobs.renderOrder = -1; // draw just over the opaque ground, like the other contact shadows
	blobs.count = 0;
	const dummy = new THREE.Object3D();

	// rebuild on any world.objects change (add / remove / move / paint) — static placements, so no per-frame work
	$effect(() => {
		let n = 0;
		for (const o of world.objects) {
			if (n >= MAX) break;
			if (CREATURES.has(o.kind)) continue;
			const def = kindDef(o.kind);
			if (def.parts[0]?.geo === 'box') continue; // houses/cabins → real shadow + weathering, not a round blob
			const sx = o.scale?.[0] ?? 1;
			const sz = o.scale?.[2] ?? 1;
			const r = def.r * Math.max(sx, sz) * 1.25; // disc a touch wider than the footprint
			dummy.position.set(o.pos[0], heightAt(o.pos[0], o.pos[2], world.terrain) + 0.05, o.pos[2]);
			dummy.scale.set(r, 1, r);
			dummy.updateMatrix();
			blobs.setMatrixAt(n++, dummy.matrix);
		}
		blobs.count = n;
		blobs.instanceMatrix.needsUpdate = true;
	});
</script>

<T is={blobs} />
