<script lang="ts">
	// Warm LIGHT POOLS under street lamps — a single InstancedMesh of additive radial discs on the ground,
	// one per lamp, fading in after dusk (uNight). Real per-lamp point lights would be far too expensive, so
	// this is the classic cheap fake: a soft warm glow decal that makes a lit city read as actually
	// illuminated (the lamp's bulb glow + halo light the air; this lights the street). Static lamps → the
	// instance matrices only rebuild when the lamp set changes; the day/night fade is a uniform. No textures.
	import { untrack } from 'svelte';
	import { T } from '@threlte/core';
	import * as THREE from 'three';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();

	const NIGHT: Record<string, number> = { day: 0, sunset: 0.55, fog: 0.35, night: 1, space: 1 };
	const MAX = 256;
	const geo = new THREE.CircleGeometry(1, 20).rotateX(-Math.PI / 2); // flat disc on the ground
	const uNight = { value: 0 };
	const mat = new THREE.ShaderMaterial({
		transparent: true,
		depthWrite: false,
		blending: THREE.AdditiveBlending,
		uniforms: { uNight },
		vertexShader: /* glsl */ `varying vec2 vUv; void main(){ vUv = uv; gl_Position = projectionMatrix * modelViewMatrix * instanceMatrix * vec4(position, 1.0); }`, // instanceMatrix MUST be applied (raw ShaderMaterial skips project_vertex) → without it every glow stacked at the world origin
		fragmentShader: /* glsl */ `
			varying vec2 vUv;
			uniform float uNight;
			void main(){
				float r = length(vUv - 0.5) * 2.0;        // 0 centre .. 1 rim
				float g = smoothstep(1.0, 0.0, r);
				gl_FragColor = vec4(vec3(1.0, 0.82, 0.46), g * g * 0.5 * uNight); // warm pool, only at night
			}
		`
	});
	const glows = new THREE.InstancedMesh(geo, mat, MAX);
	glows.frustumCulled = false;
	glows.count = 0;
	const dummy = new THREE.Object3D();

	// rebuild the instance matrices only when the lamp set changes (id list as a cheap signature)
	const lampSig = $derived(world.objects.filter((o) => o.kind === 'lamp').map((o) => o.id).join(','));
	$effect(() => {
		lampSig; // dependency
		const lamps = untrack(() => world.objects.filter((o) => o.kind === 'lamp')).slice(0, MAX);
		let n = 0;
		for (const l of lamps) {
			dummy.position.set(l.pos[0], l.pos[1] + 0.06, l.pos[2]); // just above the grounded lamp base
			dummy.scale.set(4.5, 1, 4.5); // ~4.5 m pool of light
			dummy.rotation.set(0, 0, 0);
			dummy.updateMatrix();
			glows.setMatrixAt(n++, dummy.matrix);
		}
		glows.count = n;
		glows.instanceMatrix.needsUpdate = true;
	});

	$effect(() => {
		uNight.value = NIGHT[world.sky] ?? 0; // pools fade in at dusk, off by day
	});
</script>

<T is={glows} />
