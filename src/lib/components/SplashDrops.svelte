<script lang="ts">
	// Water-ENTRY SPLASH — a quick burst of droplets flung up when the player steps/jumps into a pond, the
	// "plonk" the wade was missing (wading already slows you, sinks the avatar + rings ripples; this adds the
	// impact moment). One small GPU Points cloud, a CPU ring buffer (≤48), emitted on the inWater false→true
	// transition at the water SURFACE. Droplets arc up + out and fall back under gravity, fading as they land.
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { playerState } from '$lib/playerState.svelte';
	import { heightAt } from '$lib/terrain';
	import { waterSurfaceY } from '$lib/water';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();

	const N = 48;
	const LIFE = 0.7; // seconds a droplet lives
	const G = 14; // gravity on the droplets (m/s²)

	const posArr = new Float32Array(N * 3);
	const ageArr = new Float32Array(N).fill(1); // 1 = dead/invisible
	const velArr = new Float32Array(N * 3); // vx, vy, vz
	for (let i = 0; i < N; i++) posArr[i * 3 + 1] = -9999;

	const geo = new THREE.BufferGeometry();
	geo.setAttribute('position', new THREE.BufferAttribute(posArr, 3));
	geo.setAttribute('aAge', new THREE.BufferAttribute(ageArr, 1));

	const mat = new THREE.ShaderMaterial({
		transparent: true,
		depthWrite: false,
		vertexShader: /* glsl */ `
			attribute float aAge;
			varying float vAge;
			void main() {
				vAge = aAge;
				vec4 mv = modelViewMatrix * vec4(position, 1.0);
				gl_PointSize = clamp((1.0 - aAge * 0.6) * 130.0 / max(-mv.z, 1.0), 1.5, 36.0); // shrinks as it falls
				gl_Position = projectionMatrix * mv;
			}
		`,
		fragmentShader: /* glsl */ `
			precision mediump float;
			varying float vAge;
			void main() {
				if (vAge >= 1.0) discard;
				float d = length(gl_PointCoord - 0.5);
				if (d > 0.5) discard;
				float soft = smoothstep(0.5, 0.1, d);
				float op = soft * (1.0 - vAge) * 0.85;
				if (op < 0.01) discard;
				gl_FragColor = vec4(vec3(0.82, 0.9, 1.0), op); // cool water-white droplet
			}
		`
	});
	const points = new THREE.Points(geo, mat);
	points.frustumCulled = false;

	let head = 0;
	let wasInWater = false;
	const jit = () => Math.random() * 2 - 1;

	function burst(px: number, pz: number, surfaceY: number, speed: number) {
		const n = 10 + Math.min(10, Math.round(speed)); // a faster entry throws up more
		for (let k = 0; k < n; k++) {
			head = (head + 1) % N;
			posArr[head * 3] = px + jit() * 0.25;
			posArr[head * 3 + 1] = surfaceY + 0.05;
			posArr[head * 3 + 2] = pz + jit() * 0.25;
			velArr[head * 3] = jit() * 2.2; // out
			velArr[head * 3 + 1] = 2.5 + Math.random() * 2.5; // up
			velArr[head * 3 + 2] = jit() * 2.2;
			ageArr[head] = 0.0001;
		}
	}

	let lastX = 0;
	let lastZ = 0;
	let inited = false;
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

		// the moment you ENTER water → splash at the surface where you stepped in
		if (playerState.inWater && !wasInWater) {
			let surfaceY = heightAt(px, pz, world.terrain) + 0.3; // fallback
			for (const z of world.zones ?? []) {
				if (z.material !== 'water') continue;
				if ((px - z.pos[0]) ** 2 + (pz - z.pos[2]) ** 2 < z.size * z.size) {
					surfaceY = waterSurfaceY(z, world.terrain);
					break;
				}
			}
			burst(px, pz, surfaceY, speed);
		}
		wasInWater = playerState.inWater;

		// integrate droplets: ballistic arc, fade as they fall back
		let any = false;
		for (let i = 0; i < N; i++) {
			if (ageArr[i] >= 1) continue;
			any = true;
			ageArr[i] += dt / LIFE;
			velArr[i * 3 + 1] -= G * dt;
			posArr[i * 3] += velArr[i * 3] * dt;
			posArr[i * 3 + 1] += velArr[i * 3 + 1] * dt;
			posArr[i * 3 + 2] += velArr[i * 3 + 2] * dt;
		}
		if (any) {
			geo.attributes.position.needsUpdate = true;
			geo.attributes.aAge.needsUpdate = true;
		}
	});
</script>

<T is={points} />
