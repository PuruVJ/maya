<script lang="ts">
	// Warm WINDOW-LIGHT SPILL on the ground around lit buildings at night — the companion to LampGlow. The
	// wall shader lights ~55% of windows after dusk (emissive glow), but that light never reached the ground,
	// so a night town's streets stayed dark between the lamps. This adds a soft warm additive disc around each
	// house/cabin/tower (the building itself occludes the centre, so it reads as light SPILLING from the
	// windows onto the surrounding ground), fading in with the same uNight as the lit windows. One InstancedMesh
	// (1 draw call), static — rebuilt only when the building set changes. instanceMatrix MUST be applied in the
	// vertex shader (raw ShaderMaterial on an InstancedMesh skips project_vertex) or every disc stacks at origin.
	import { untrack } from 'svelte';
	import { T } from '@threlte/core';
	import * as THREE from 'three';
	import { kindDef } from '$lib/kinds';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();

	const LIT = new Set(['house', 'cabin', 'tower']); // the kinds whose walls get lit windows
	const NIGHT: Record<string, number> = { day: 0, sunset: 0.5, fog: 0.18, night: 1, space: 1 }; // matches Building's window-glow
	const MAX = 512;
	const geo = new THREE.CircleGeometry(1, 20).rotateX(-Math.PI / 2);
	const uNight = { value: 0 };
	const mat = new THREE.ShaderMaterial({
		transparent: true,
		depthWrite: false,
		blending: THREE.AdditiveBlending,
		uniforms: { uNight },
		vertexShader: /* glsl */ `varying vec2 vUv; void main(){ vUv = uv; gl_Position = projectionMatrix * modelViewMatrix * instanceMatrix * vec4(position, 1.0); }`,
		fragmentShader: /* glsl */ `
			varying vec2 vUv;
			uniform float uNight;
			void main(){
				float r = length(vUv - 0.5) * 2.0;          // 0 centre .. 1 rim
				float g = smoothstep(1.0, 0.18, r);          // soft pool
				gl_FragColor = vec4(vec3(1.0, 0.80, 0.50), g * g * 0.28 * uNight); // warm window spill, only at night
			}
		`
	});
	const glows = new THREE.InstancedMesh(geo, mat, MAX);
	glows.frustumCulled = false;
	glows.renderOrder = -1; // over the ground, under the building
	glows.count = 0;
	const dummy = new THREE.Object3D();

	// rebuild only when the lit-building set changes (kind+id+scale signature)
	const sig = $derived(
		world.objects
			.filter((o) => LIT.has(o.kind))
			.map((o) => o.id + (o.scale ? o.scale.join('x') : ''))
			.join(',')
	);
	$effect(() => {
		sig; // dependency
		const builds = untrack(() => world.objects.filter((o) => LIT.has(o.kind))).slice(0, MAX);
		let n = 0;
		for (const b of builds) {
			const sx = b.scale?.[0] ?? 1;
			const sz = b.scale?.[2] ?? 1;
			const r = kindDef(b.kind).r * Math.max(sx, sz) * 1.7; // spills out past the walls
			dummy.position.set(b.pos[0], b.pos[1] + 0.05, b.pos[2]);
			dummy.scale.set(r, 1, r);
			dummy.rotation.set(0, 0, 0);
			dummy.updateMatrix();
			glows.setMatrixAt(n++, dummy.matrix);
		}
		glows.count = n;
		glows.instanceMatrix.needsUpdate = true;
	});

	$effect(() => {
		uNight.value = NIGHT[world.sky] ?? 0;
	});
</script>

<T is={glows} />
