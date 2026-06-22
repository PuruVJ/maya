<script lang="ts">
	// Settlement glows — warm lamp-light blooms that hint at REAL distant buildings THROUGH the fog ("a place
	// out there in the murk"), then fade out as you approach and the actual blocks reveal (Scene's lazy
	// distance-capped reveal). ONE additive Points cloud, one bloom per world.objects building → a city reads
	// as a CLUSTER of lights and a lone house as a single lamp (size = building count + each building's scale).
	// Cheap (1 draw call, no mounted geometry), so it satisfies "structures legible from far" WITHOUT the
	// frame-rate cost of mounting them. World-stable: the bloom positions only rebuild when buildings are
	// added/removed; the per-frame distance fade lives in the shader (uPlayer), so motion stays smooth.
	// Pairs with the denser fog (SKY_FOG) for a Death-Stranding sense of dread. Decorative, not saved/collided.
	import { T, useTask, useThrelte } from '@threlte/core';
	import * as THREE from 'three';
	import { heightAt } from '$lib/terrain';
	import { playerState } from '$lib/playerState.svelte';
	import { worldCurveRadius } from '$lib/curveWorld';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();

	const BUILDINGS = new Set(['house', 'cabin', 'tower']);
	const MAX = 1200;
	// fade in over NEAR…NEAR+FADE so a bloom is OFF where the real building reveals (matched to Scene BUILD_KEEP
	// ≈340 m now that structures reveal far), then glows out to FAR on the horizon. Tunable.
	const NEAR = 330;
	const FADE = 90;
	const FAR = 1700; // far HORIZON reveal (was 560) — distant settlements glow on the curved "wall" (user: show them far)
	const FAR_FADE = 360; // glows fade out over the last FAR_FADE m → a soft band of city-lights at the horizon, not a hard cut

	const geo = new THREE.BufferGeometry();
	const posAttr = new THREE.BufferAttribute(new Float32Array(MAX * 3), 3);
	const phaseAttr = new THREE.BufferAttribute(new Float32Array(MAX), 1);
	const scaleAttr = new THREE.BufferAttribute(new Float32Array(MAX), 1);
	geo.setAttribute('position', posAttr);
	geo.setAttribute('aPhase', phaseAttr);
	geo.setAttribute('aScale', scaleAttr);
	geo.setDrawRange(0, 0);

	// lamp-glows read best in the dark/murk — strong at night + under rain-fog (max dread), a faint haze by day
	// (bright warm orbs in full daylight would look wrong).
	const NIGHT: Record<string, number> = { day: 0, sunset: 0.55, fog: 0.7, night: 1, space: 1 };

	const { renderer } = useThrelte();
	const uniforms = {
		uPlayer: { value: new THREE.Vector2(9999, 9999) },
		uTime: { value: 0 },
		uNight: { value: 0 },
		uH: { value: 800 }, // viewport height → keeps a bloom a roughly constant screen size with distance
		uR: { value: worldCurveRadius() } // world-fold radius → glows ride the curve up onto the horizon "wall"
	};

	const mat = new THREE.ShaderMaterial({
		uniforms,
		transparent: true,
		depthWrite: false,
		blending: THREE.AdditiveBlending,
		vertexShader: /* glsl */ `
			uniform vec2 uPlayer;
			uniform float uTime;
			uniform float uNight;
			uniform float uH;
			uniform float uR;
			attribute float aPhase;
			attribute float aScale;
			varying float vA;
			void main() {
				vec4 wp = modelMatrix * vec4(position, 1.0);
				float d = distance(wp.xz, uPlayer); // FLAT distance (pre-fold) → drives the near/far fade
				float fin = smoothstep(${NEAR}.0, ${NEAR + FADE}.0, d);      // off when close (the real block shows)
				float fout = 1.0 - smoothstep(${FAR - FAR_FADE}.0, ${FAR}.0, d); // soft fade-out band at the horizon
				float flick = 0.82 + 0.18 * sin(uTime * 2.1 + aPhase * 6.2831); // gentle lamp flicker
				vA = fin * fout * flick * (0.22 + 0.78 * uNight); // strong at night/fog, faint by day
				// ride the inception-fold (same math as curveWorld.ts) so a far glow climbs the curved valley WALL
				// instead of floating low under the reared-up terrain. cameraPosition is provided by ShaderMaterial.
				if (uR > 0.5) {
					float ang = (wp.z - cameraPosition.z) / uR;
					wp.z = cameraPosition.z + sin(ang) * uR;
					wp.y += (1.0 - cos(ang)) * uR;
				}
				vec4 mv = viewMatrix * wp;
				gl_Position = projectionMatrix * mv;
				gl_PointSize = clamp(uH * 11.0 * aScale / -mv.z, 9.0, 54.0); // bigger building → bigger bloom
			}
		`,
		fragmentShader: /* glsl */ `
			varying float vA;
			void main() {
				if (vA <= 0.002) discard;
				vec2 c = gl_PointCoord - 0.5;
				float g = smoothstep(0.5, 0.0, length(c)); // soft radial falloff
				g *= g;
				vec3 warm = vec3(1.0, 0.76, 0.42);         // sodium-lamp light
				gl_FragColor = vec4(warm * g * vA, g * vA);
			}
		`
	});
	mat.toneMapped = false;

	const points = new THREE.Points(geo, mat);
	points.frustumCulled = false; // blooms are camera-relative-faded; never cull the whole cloud

	let n = 0; // building cell to fill in `rebuild`
	const pos = posAttr.array as Float32Array;
	const ph = phaseAttr.array as Float32Array;
	const sc = scaleAttr.array as Float32Array;
	/** Add one building's bloom (near OR dormant). */
	function add(o: { kind: string; pos: number[]; scale?: number[] }): void {
		if (!BUILDINGS.has(o.kind) || n >= MAX) return;
		pos[n * 3] = o.pos[0];
		pos[n * 3 + 1] = heightAt(o.pos[0], o.pos[2], world.terrain) + 3.6; // lamp height
		pos[n * 3 + 2] = o.pos[2];
		ph[n] = (n * 0.618033) % 1; // golden-ratio phase spread → no synchronised flicker
		sc[n] = Math.min(2.4, Math.max(o.scale?.[0] ?? 1, o.scale?.[2] ?? 1)); // towers glow bigger
		n++;
	}
	/** PULSE: gather EVERY settlement's lights — near (live world.objects) AND dormant/streamed regions (their
	 *  `statics` keep houses verbatim). Lights deliberately BREAK the streaming rule (user) so a far city still
	 *  glows on the horizon "wall" though its creatures are collapsed. Runs every PULSE_S, not per frame. */
	function rebuild(): void {
		n = 0;
		for (const o of world.objects) add(o);
		if (world.regions) for (const key in world.regions) for (const o of world.regions[key].statics) add(o);
		posAttr.needsUpdate = true;
		phaseAttr.needsUpdate = true;
		scaleAttr.needsUpdate = true;
		geo.setDrawRange(0, n);
	}

	const PULSE_S = 2.5; // re-scan for far settlements every few seconds (cheap; horizon lights don't need per-frame)
	let pulse = PULSE_S; // fire on the first frame
	useTask((dt) => {
		uniforms.uTime.value += dt;
		uniforms.uPlayer.value.set(playerState.pos[0], playerState.pos[2]);
		uniforms.uNight.value = NIGHT[world.sky] ?? 0;
		uniforms.uH.value = renderer.domElement.clientHeight || 800;
		pulse += dt;
		if (pulse >= PULSE_S) {
			pulse = 0;
			rebuild();
		}
	});
</script>

<T is={points} />
