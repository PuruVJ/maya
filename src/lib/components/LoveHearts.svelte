<script lang="ts">
	// Floating HEARTS — when two organisms bond (conceive), a pink heart pops above the couple, rises a little
	// and fades. Driven by the sim's CONCEIVE events (drainLoves), drawn as ONE pooled GPU Points cloud with a
	// heart SDF in the fragment shader — no assets, no per-heart objects. Decorative; not saved/shared.
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { sim } from '$lib/sim';
	import { heightAt } from '$lib/terrain';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();

	const POOL = 24; // max hearts on screen at once (round-robin reuse); births are bursty but brief
	const LIFE = 2.4; // seconds a heart lives (rise + fade)

	const pos = new Float32Array(POOL * 3); // base spawn position per slot
	const start = new Float32Array(POOL).fill(-1e9); // spawn time per slot (far past = inactive)
	const geo = new THREE.BufferGeometry();
	const posAttr = new THREE.Float32BufferAttribute(pos, 3);
	const startAttr = new THREE.Float32BufferAttribute(start, 1);
	geo.setAttribute('position', posAttr);
	geo.setAttribute('aStart', startAttr);

	const uniforms = { uTime: { value: 0 }, uLife: { value: LIFE } };
	const mat = new THREE.ShaderMaterial({
		uniforms,
		transparent: true,
		depthWrite: false,
		vertexShader: /* glsl */ `
			attribute float aStart;
			uniform float uTime;
			uniform float uLife;
			varying float vA;
			void main() {
				float age = uTime - aStart;
				vec3 p = position;
				p.y += age * 0.7;                                  // drift upward
				vec4 mv = modelViewMatrix * vec4(p, 1.0);
				gl_Position = projectionMatrix * mv;
				float t = clamp(age / uLife, 0.0, 1.0);
				float pop = smoothstep(0.0, 0.12, t);              // quick scale-in
				float fade = 1.0 - smoothstep(0.55, 1.0, t);       // fade out over the back half
				float live = step(0.0, age) * step(age, uLife);    // 0 when inactive / expired
				gl_PointSize = pop * 26.0 * (300.0 / max(-mv.z, 1.0)) * live;
				vA = fade * live;
			}
		`,
		fragmentShader: /* glsl */ `
			varying float vA;
			void main() {
				if (vA < 0.01) discard;
				// heart implicit on centred coords: (x^2 + y^2 - 1)^3 - x^2 y^3 <= 0
				vec2 c = (gl_PointCoord - 0.5) * 2.6;
				float x = c.x;
				float y = -c.y - 0.35;                             // flip + lift so the lobes sit up top
				float h = pow(x * x + y * y - 1.0, 3.0) - x * x * y * y * y;
				float a = smoothstep(0.06, -0.06, h) * vA;         // soft edge
				if (a < 0.01) discard;
				gl_FragColor = vec4(1.0, 0.32, 0.45, a);           // warm pink
			}
		`
	});

	const points = new THREE.Points(geo, mat);
	points.frustumCulled = false;

	let next = 0; // round-robin slot cursor
	useTask((dt) => {
		uniforms.uTime.value += dt;
		const loves = sim.drainLoves();
		if (loves.length === 0) return;
		for (const lv of loves) {
			const s = next;
			next = (next + 1) % POOL;
			pos[s * 3] = lv.x;
			pos[s * 3 + 1] = heightAt(lv.x, lv.z, world.terrain) + 1.9; // above their heads
			pos[s * 3 + 2] = lv.z;
			start[s] = uniforms.uTime.value;
		}
		posAttr.needsUpdate = true;
		startAttr.needsUpdate = true;
	});
</script>

<T is={points} />
