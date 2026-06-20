<script lang="ts">
	// Cozy CHIMNEY SMOKE — soft wisps rising from every house/cabin roof, so a village reads as lived-in.
	// One shared GPU Points cloud (a single draw call) for the whole world; each particle's rise/sway/fade is
	// computed in the vertex+fragment shader from gl_VertexID-seeded randomness, driven by the shared wind
	// clock — no per-frame CPU work. World-anchored (smoke stays over its house as you roam); the buffer is
	// rebuilt only when the set of houses changes. Pure shader, no textures ([[shader-first-direction]]).
	import { untrack } from 'svelte';
	import { T } from '@threlte/core';
	import * as THREE from 'three';
	import { wind, WIND_GUST } from '$lib/wind';
	import { kindDef } from '$lib/kinds';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();

	const PER = 12; // particles per chimney
	const MAXH = 48; // cap chimneys (a mega-city won't drown the frame) → ≤576 points
	const CHIMNEY = new Set(['house', 'cabin']);
	const rnd = (n: number) => {
		const v = Math.sin(n * 12.9898 + 78.233) * 43758.5453;
		return v - Math.floor(v);
	};

	const mat = new THREE.ShaderMaterial({
		uniforms: { uTime: wind.uTime, uColor: { value: new THREE.Color(0.87, 0.88, 0.92) } }, // shares Scene's wind clock
		transparent: true,
		depthWrite: false,
		vertexShader: /* glsl */ `
			attribute float aSeed;
			uniform float uTime;
			varying float vAge;
			varying float vDist;
			void main() {
				float speed = 0.12 + 0.10 * fract(aSeed * 1.7);
				float age = fract(uTime * speed + aSeed);           // 0 (just left the chimney) .. 1 (dissipated)
				vAge = age;
				vec3 p = position;
				p.y += age * 3.4;                                   // rise
				float t = uTime * 0.5 + aSeed * 6.2831;
				float g = ${WIND_GUST};                             // smoke streams harder when a gust passes
				p.x += (sin(t) * 0.35 + 0.7) * age * g;             // sway + wind drift, gust-modulated
				p.z += cos(t * 0.8) * 0.35 * age * g;
				vec4 mv = modelViewMatrix * vec4(p, 1.0);
				vDist = -mv.z;
				gl_PointSize = clamp((0.5 + age * 2.0) * 320.0 / max(-mv.z, 1.0), 2.0, 130.0); // grows + perspective
				gl_Position = projectionMatrix * mv;
			}
		`,
		fragmentShader: /* glsl */ `
			precision mediump float;
			uniform vec3 uColor;
			varying float vAge;
			varying float vDist;
			void main() {
				float d = length(gl_PointCoord - 0.5);
				if (d > 0.5) discard;
				float soft = smoothstep(0.5, 0.05, d);                       // soft round puff
				float life = (1.0 - vAge) * smoothstep(0.0, 0.18, vAge);     // fade in off the chimney, then out
				float fog = smoothstep(175.0, 60.0, vDist);                  // far chimneys fade (Points get no scene fog)
				float opacity = soft * life * fog * 0.42;
				if (opacity < 0.01) discard;
				gl_FragColor = vec4(uColor, opacity);
			}
		`
	});

	const points = new THREE.Points(new THREE.BufferGeometry(), mat);
	points.frustumCulled = false; // particles fly off their base positions in the shader → CPU bounds lie

	// rebuild the particle buffer only when the set of chimney buildings changes (id list as a cheap signature)
	const sig = $derived(world.objects.filter((o) => CHIMNEY.has(o.kind)).map((o) => o.id).join(','));
	$effect(() => {
		sig; // dependency
		const houses = untrack(() => world.objects.filter((o) => CHIMNEY.has(o.kind))).slice(0, MAXH);
		const n = houses.length * PER;
		const pos = new Float32Array(n * 3);
		const seed = new Float32Array(n);
		let k = 0;
		for (const h of houses) {
			const sy = h.scale?.[1] ?? 1;
			const oy = (kindDef(h.kind).h + 0.5) * sy; // smoke origin ≈ just above the roof
			for (let p = 0; p < PER; p++) {
				pos[k * 3] = h.pos[0] + (rnd(k) - 0.5) * 0.4;
				pos[k * 3 + 1] = h.pos[1] + oy;
				pos[k * 3 + 2] = h.pos[2] + (rnd(k + 99) - 0.5) * 0.4;
				seed[k] = rnd(k + 7);
				k++;
			}
		}
		const g = new THREE.BufferGeometry();
		g.setAttribute('position', new THREE.BufferAttribute(pos, 3));
		g.setAttribute('aSeed', new THREE.BufferAttribute(seed, 1));
		points.geometry.dispose();
		points.geometry = g;
	});

	$effect(() => {
		points.visible = world.sky !== 'space'; // no chimney smoke in the void
	});
</script>

<T is={points} />
