<script lang="ts">
	// Ambient air particles — faint drifting DUST MOTES by day that cross-fade into glowing FIREFLIES at
	// night (tying into the day/night system alongside lamps + lit windows). One GPU Points cloud, additive
	// soft dots, that TILES around the player (world-anchored wrap, so it fills the air without sticking to
	// you). Pure shader, no assets. Decorative; not saved/shared. Hidden in space.
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { playerState } from '$lib/playerState.svelte';
	import { weather } from '$lib/weather';

	let { sky = 'day' }: { sky?: string } = $props();

	const COUNT = 50; // sparse — 320 then 130 STILL read as "too many fireflies" (distracting); keep it subtle
	const BOX = 72; // particles fill a ±36 m box around the player (spread the small count wider → calmer, less swarmy)
	const NIGHT: Record<string, number> = { day: 0, sunset: 0.5, fog: 0.3, night: 1, space: 1 };

	// base positions (x,z in the box; y in a low air band) + a per-particle phase for drift/blink
	const pos = new Float32Array(COUNT * 3);
	const phase = new Float32Array(COUNT);
	for (let i = 0; i < COUNT; i++) {
		pos[i * 3] = Math.random() * BOX;
		pos[i * 3 + 1] = 0.4 + Math.random() * 5;
		pos[i * 3 + 2] = Math.random() * BOX;
		phase[i] = Math.random() * 100;
	}
	const geo = new THREE.BufferGeometry();
	geo.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3));
	geo.setAttribute('aPhase', new THREE.Float32BufferAttribute(phase, 1));

	const uniforms = { uTime: { value: 0 }, uPlayer: { value: new THREE.Vector2() }, uNight: { value: 0 }, uSnow: weather.uSnow };

	const mat = new THREE.ShaderMaterial({
		uniforms,
		transparent: true,
		depthWrite: false,
		blending: THREE.AdditiveBlending,
		vertexShader: /* glsl */ `
			attribute float aPhase;
			uniform vec2 uPlayer;
			uniform float uTime;
			uniform float uNight;
			uniform float uSnow;
			varying float vA;
			varying float vN;
			void main() {
				vec3 p = position;
				p.x += sin(uTime * 0.3 + aPhase) * 0.9;        // gentle drift
				p.z += cos(uTime * 0.27 + aPhase * 1.3) * 0.9;
				p.y += sin(uTime * 0.6 + aPhase * 2.1) * 0.5;
				vec2 origin = uPlayer - vec2(${(BOX / 2).toFixed(1)});
				vec2 wxz = origin + mod(p.xz - origin, vec2(${BOX.toFixed(1)})); // wrap → stays centred on player
				vec4 mv = modelViewMatrix * vec4(wxz.x, p.y, wxz.y, 1.0);
				gl_Position = projectionMatrix * mv;
				gl_PointSize = mix(1.6, 5.0, uNight) * (300.0 / max(-mv.z, 1.0)); // motes small, fireflies bigger
				float blink = 0.45 + 0.55 * sin(uTime * 2.6 + aPhase * 4.0);
				vA = mix(0.1, blink * 0.95 * (1.0 - uSnow), uNight); // dust → fireflies; but NO fireflies in winter (snow world)
				vN = uNight;
			}
		`,
		fragmentShader: /* glsl */ `
			varying float vA;
			varying float vN;
			void main() {
				float r = length(gl_PointCoord - 0.5);
				float soft = smoothstep(0.5, 0.0, r);          // round soft dot
				vec3 col = mix(vec3(1.0, 0.95, 0.82), vec3(0.72, 1.0, 0.42), vN); // warm dust → firefly green
				float a = soft * vA;
				if (a < 0.01) discard;
				gl_FragColor = vec4(col, a);
			}
		`
	});

	const points = new THREE.Points(geo, mat);
	points.frustumCulled = false; // positions live in the shader

	useTask((dt) => {
		points.visible = sky !== 'space'; // no dust/fireflies in the void
		if (!points.visible) return;
		uniforms.uTime.value += dt;
		uniforms.uPlayer.value.set(playerState.pos[0], playerState.pos[2]);
		uniforms.uNight.value += ((NIGHT[sky] ?? 0) - uniforms.uNight.value) * Math.min(1, 2 * dt); // ease day↔night
	});
</script>

<T is={points} />
