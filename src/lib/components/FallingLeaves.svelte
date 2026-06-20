<script lang="ts">
	// Ambient DRIFTING AUTUMN LEAVES — a sparse GPU Points cloud that tumbles down on the breeze by day and
	// fades out at night (the daylight counterpart to AmbientParticles' fireflies). Pairs with the varied
	// autumn canopies (Tree/AmbientScatter draw the odd autumn tree) so the air over a forest has leaves on
	// the wind. Pure shader, no assets; TILES around the player (world-anchored wrap) so it follows without
	// sticking to you. Alpha-blended (solid leaves, NOT additive glow); the opaque terrain occludes any leaf
	// that falls below ground, so it reads as settling. Decorative; not saved/shared. Hidden in space.
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { playerState } from '$lib/playerState.svelte';
	import { wind, WIND_GUST } from '$lib/wind';

	let { sky = 'day', ground = 'grass' }: { sky?: string; ground?: string } = $props();

	const COUNT = 60; // sparse — leaves read bigger than dust motes, so fewer than the firefly cloud
	const BOX = 64; // fill a ±32 m footprint around the player
	const FALL_H = 14; // air column height the leaves fall through (absolute world Y; terrain occludes below)
	// how "night" each sky is → leaves are the inverse (out by night, full by day; thinned at dusk/overcast)
	const NIGHT: Record<string, number> = { day: 0, sunset: 0.5, fog: 0.35, night: 1, space: 1 };

	// base x,z in the footprint; base y spread through the column; a per-leaf phase drives speed/flutter/hue/size
	const pos = new Float32Array(COUNT * 3);
	const phase = new Float32Array(COUNT);
	for (let i = 0; i < COUNT; i++) {
		pos[i * 3] = Math.random() * BOX;
		pos[i * 3 + 1] = Math.random() * FALL_H;
		pos[i * 3 + 2] = Math.random() * BOX;
		phase[i] = Math.random() * 100;
	}
	const geo = new THREE.BufferGeometry();
	geo.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3));
	geo.setAttribute('aPhase', new THREE.Float32BufferAttribute(phase, 1));

	// share the GLOBAL wind clock (ticked by Scene) so leaves surge in phase with grass/trees, not on a private clock
	const uniforms = { uTime: wind.uTime, uPlayer: { value: new THREE.Vector2() }, uDay: { value: 1 } };

	const mat = new THREE.ShaderMaterial({
		uniforms,
		transparent: true,
		depthWrite: false, // leaves don't occlude each other; the terrain still occludes them (depth test on)
		vertexShader: /* glsl */ `
			attribute float aPhase;
			uniform vec2 uPlayer;
			uniform float uTime;
			uniform float uDay;
			varying float vA;
			varying vec3 vCol;
			void main() {
				vec3 p = position;
				// fall: descend through the column and wrap back to the top; per-leaf speed so they don't march in step
				float fall = 0.55 + 0.4 * fract(aPhase * 1.7);
				float yy = mod(p.y - uTime * fall - aPhase * 5.0, ${FALL_H}.0);
				// flutter: tumble side-to-side on the breeze as it falls (a leaf, not a raindrop). The SHARED gust
				// scales the flutter so leaves are swept harder exactly when the grass/trees lean into a gust.
				float gust = ${WIND_GUST};
				float fl = uTime * (1.3 + fract(aPhase * 2.3)) + aPhase * 6.2831;
				p.x += sin(fl) * 0.6 * gust;
				p.z += cos(fl * 0.85) * 0.5 * gust;
				vec2 origin = uPlayer - vec2(${(BOX / 2).toFixed(1)});
				vec2 wxz = origin + mod(p.xz - origin, vec2(${BOX.toFixed(1)})); // wrap → stays centred on player
				vec4 mv = modelViewMatrix * vec4(wxz.x, yy, wxz.y, 1.0);
				gl_Position = projectionMatrix * mv;
				gl_PointSize = (5.0 + 3.0 * fract(aPhase * 3.1)) * (300.0 / max(-mv.z, 1.0)); // varied leaf size
				// fade in at the top of the column, out near the ground, and away entirely at night
				float fade = smoothstep(${FALL_H}.0, ${(FALL_H - 2).toFixed(1)}, yy) * smoothstep(0.0, 2.0, yy);
				vA = fade * uDay * 0.9;
				// varied autumn hue per leaf: gold → orange → brown
				float h = fract(aPhase * 0.61);
				vec3 gold = vec3(0.86, 0.62, 0.18);
				vec3 orange = vec3(0.80, 0.36, 0.12);
				vec3 brown = vec3(0.52, 0.29, 0.11);
				vCol = h < 0.5 ? mix(gold, orange, h * 2.0) : mix(orange, brown, (h - 0.5) * 2.0);
			}
		`,
		fragmentShader: /* glsl */ `
			varying float vA;
			varying vec3 vCol;
			void main() {
				vec2 c = gl_PointCoord - 0.5;
				float leaf = smoothstep(0.5, 0.18, length(vec2(c.x * 1.7, c.y))); // soft leaf-ish oval (narrow across)
				float a = leaf * vA;
				if (a < 0.01) discard;
				gl_FragColor = vec4(vCol, a);
			}
		`
	});

	const points = new THREE.Points(geo, mat);
	points.frustumCulled = false; // positions live in the shader

	useTask((dt) => {
		points.visible = sky !== 'space' && ground !== 'snow'; // no leaves in the void — or in winter (snow world)
		if (!points.visible) return;
		// uTime is the shared wind clock (Scene ticks it) — don't advance it here
		uniforms.uPlayer.value.set(playerState.pos[0], playerState.pos[2]);
		const day = 1 - (NIGHT[sky] ?? 0);
		uniforms.uDay.value += (day - uniforms.uDay.value) * Math.min(1, 2 * dt); // ease day↔night
	});
</script>

<T is={points} />
