<script lang="ts">
	// Footstep DUST — soft puffs kicked up at the player's feet when running/sprinting on dry ground, plus a
	// little burst on landing a jump. World-anchored (the dust stays where it was kicked and settles), tinted
	// by the GROUND so it reads right everywhere: tan on sand, white powder in snow, a faint scuff on grass.
	// Skipped while wading (the water shader already rings ripples there). One small GPU Points cloud, a ring
	// buffer of particles updated on the CPU (≤40, trivial). Completes the "world reacts to your movement" loop.
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { playerState } from '$lib/playerState.svelte';
	import { heightAt } from '$lib/terrain';
	import { GROUND_COLOR } from '$lib/kinds';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();

	const N = 40;
	const LIFE = 0.85; // seconds a puff lives
	const RUN_THRESH = 7; // only running/sprinting kicks dust up (a walk barely disturbs the ground)

	const posArr = new Float32Array(N * 3);
	const ageArr = new Float32Array(N).fill(1); // 1 = dead/invisible
	const velArr = new Float32Array(N * 2);
	for (let i = 0; i < N; i++) posArr[i * 3 + 1] = -9999; // park dead particles far below

	const geo = new THREE.BufferGeometry();
	geo.setAttribute('position', new THREE.BufferAttribute(posArr, 3));
	geo.setAttribute('aAge', new THREE.BufferAttribute(ageArr, 1));

	const mat = new THREE.ShaderMaterial({
		uniforms: { uColor: { value: new THREE.Color('#c9bfa6') } },
		transparent: true,
		depthWrite: false,
		vertexShader: /* glsl */ `
			attribute float aAge;
			varying float vAge;
			void main() {
				vAge = aAge;
				vec4 mv = modelViewMatrix * vec4(position, 1.0);
				gl_PointSize = clamp((0.25 + aAge * 0.9) * 260.0 / max(-mv.z, 1.0), 2.0, 90.0); // grows as it dissipates
				gl_Position = projectionMatrix * mv;
			}
		`,
		fragmentShader: /* glsl */ `
			precision mediump float;
			uniform vec3 uColor;
			varying float vAge;
			void main() {
				if (vAge >= 1.0) discard;
				float d = length(gl_PointCoord - 0.5);
				if (d > 0.5) discard;
				float soft = smoothstep(0.5, 0.12, d);
				float op = soft * (1.0 - vAge) * smoothstep(0.0, 0.2, vAge) * 0.5;
				if (op < 0.01) discard;
				gl_FragColor = vec4(uColor, op);
			}
		`
	});
	const points = new THREE.Points(geo, mat);
	points.frustumCulled = false;

	$effect(() => {
		// tint to the ground colour, lightened so it reads as a puff (snow → near-white, sand → pale tan)
		const g = new THREE.Color(GROUND_COLOR[world.ground] ?? GROUND_COLOR.grass);
		g.lerp(new THREE.Color(1, 1, 1), 0.4);
		mat.uniforms.uColor.value.copy(g);
	});

	let head = 0;
	let emitT = 0;
	let lastX = 0;
	let lastZ = 0;
	let inited = false;
	let wasAir = false;
	const jit = () => Math.random() * 0.5 - 0.25;

	function emit(px: number, pz: number) {
		head = (head + 1) % N;
		const x = px + jit();
		const z = pz + jit();
		posArr[head * 3] = x;
		posArr[head * 3 + 1] = heightAt(x, z, world.terrain) + 0.12;
		posArr[head * 3 + 2] = z;
		velArr[head * 2] = Math.random() * 0.6 - 0.3; // settle outward a touch
		velArr[head * 2 + 1] = Math.random() * 0.6 - 0.3;
		ageArr[head] = 0.0001;
	}

	useTask((dt) => {
		const px = playerState.pos[0];
		const pz = playerState.pos[2];
		if (!inited) {
			lastX = px;
			lastZ = pz;
			inited = true;
		}
		const speed = Math.hypot(px - lastX, pz - lastZ) / Math.max(dt, 1e-4);
		lastX = px;
		lastZ = pz;
		const dry = !playerState.inWater && world.sky !== 'fog'; // no dust off wet ground while it's raining
		const grounded = playerState.grounded;

		if (grounded && wasAir && dry) ((emit(px, pz), emit(px, pz))); // landing burst
		wasAir = !grounded;

		if (grounded && dry && speed > RUN_THRESH) {
			emitT -= dt;
			if (emitT <= 0) {
				emit(px, pz);
				emitT = Math.max(0.06, 0.18 - speed * 0.006); // faster you run, more puffs
			}
		}

		for (let i = 0; i < N; i++) {
			if (ageArr[i] >= 1) continue;
			ageArr[i] += dt / LIFE;
			posArr[i * 3] += velArr[i * 2] * dt;
			posArr[i * 3 + 1] += dt * 0.35; // drift up as it settles/dissipates
			posArr[i * 3 + 2] += velArr[i * 2 + 1] * dt;
		}
		geo.attributes.position.needsUpdate = true;
		geo.attributes.aAge.needsUpdate = true;
	});
</script>

<T is={points} />
