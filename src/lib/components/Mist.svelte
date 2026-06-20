<script lang="ts">
	// Procedural GROUND MIST — a thin, low layer of drifting wisps hugging the ground, distinct from the global
	// distance fog (that just fades the horizon; this is patchy mist you see lying ON the ground near you). A
	// single big horizontal plane that follows the player at ~ground level + a low offset, with a world-space
	// fbm fragment (parallax-correct: wisps stay put in the world as you roam) and a radial edge fade. Fades in
	// only on the moody skies (night / dawn-ish sunset / fog); none in clear day or space. Pure shader, no assets.
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { playerState } from '$lib/playerState.svelte';
	import { heightAt } from '$lib/terrain';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();

	// how thick the ground mist is, per sky (clear day / space = none; fog = most; night/sunset = atmospheric)
	const MIST: Record<string, number> = { day: 0, sunset: 0.35, fog: 0.8, night: 0.55, space: 0 };
	const MIST_Y = 0.9; // height of the mist sheet above the local ground

	const uniforms = {
		uTime: { value: 0 },
		uStrength: { value: 0 }
	};
	const geo = new THREE.PlaneGeometry(900, 900);
	const mat = new THREE.ShaderMaterial({
		uniforms,
		transparent: true,
		depthWrite: false,
		side: THREE.DoubleSide,
		toneMapped: false,
		vertexShader: /* glsl */ `
			varying vec2 vWorld;
			varying vec2 vLocal;
			void main() {
				vLocal = uv;
				vWorld = (modelMatrix * vec4(position, 1.0)).xz;
				gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
			}
		`,
		fragmentShader: /* glsl */ `
			uniform float uTime;
			uniform float uStrength;
			varying vec2 vWorld;
			varying vec2 vLocal;
			float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
			float noise(vec2 p) {
				vec2 i = floor(p), f = fract(p);
				float a = hash(i), b = hash(i + vec2(1.0, 0.0)), c = hash(i + vec2(0.0, 1.0)), d = hash(i + vec2(1.0, 1.0));
				vec2 u = f * f * (3.0 - 2.0 * f);
				return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
			}
			float fbm(vec2 p) { float v = 0.0, a = 0.5; for (int i = 0; i < 4; i++) { v += a * noise(p); p *= 2.03; a *= 0.5; } return v; }
			void main() {
				if (uStrength < 0.01) discard;
				// two layers drifting at different rates → the mist roils slowly instead of sliding rigidly
				float n = fbm(vWorld * 0.025 + vec2(uTime * 0.012, uTime * 0.008));
				n = 0.6 * n + 0.4 * fbm(vWorld * 0.06 - vec2(uTime * 0.02, uTime * 0.015));
				float mist = smoothstep(0.42, 0.72, n);
				float edge = smoothstep(0.5, 0.32, distance(vLocal, vec2(0.5))); // hide the square rim
				float a = mist * edge * uStrength * 0.42;
				if (a < 0.01) discard;
				gl_FragColor = vec4(vec3(0.80, 0.83, 0.87), a); // soft cool-white haze
			}
		`
	});

	const mesh = new THREE.Mesh(geo, mat);
	mesh.rotation.x = -Math.PI / 2; // lie flat
	mesh.frustumCulled = false; // huge + player-following
	mesh.renderOrder = 2; // over the ground/water, soft additive-ish veil

	$effect(() => {
		const s = MIST[world.sky] ?? 0;
		uniforms.uStrength.value = s;
		mesh.visible = s > 0; // hide the whole sheet in clear day / space → no full-screen discard cost
	});

	useTask((dt) => {
		uniforms.uTime.value += dt;
		if (!mesh.visible) return; // no mist this sky → skip the heightAt + reposition
		const px = playerState.pos[0];
		const pz = playerState.pos[2];
		// sit just above the player's LOCAL ground (the buildable zone is flat, so a flat sheet hugs it; far
		// out it melts into the distance fog before the terrain diverges enough to matter)
		mesh.position.set(px, heightAt(px, pz, world.terrain) + MIST_Y, pz);
	});
</script>

<T is={mesh} />
