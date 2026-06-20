<script lang="ts">
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { heightAt } from '$lib/terrain';
	import { GROUND_COLOR } from '$lib/kinds';
	import { playerState } from '$lib/playerState.svelte';
	import { wind } from '$lib/wind';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();
	const color = $derived(GROUND_COLOR[world.ground] ?? GROUND_COLOR.grass);
	// how strongly drifting cloud shadows dapple the ground, by sky (clear day = boldest; fog/night = faint,
	// space = none). Ties the existing Clouds layer to the ground so the world feels lit by a moving sky.
	const CLOUD_SHADOW: Record<string, number> = { day: 1, sunset: 0.7, fog: 0.3, night: 0.15, space: 0 };

	// One big grid that FOLLOWS the player and re-displaces from heightAt, so there is always ground
	// underfoot and the world feels endless. The Inception-fold curve (applied in the shader) rears
	// the far ground up into the sky ahead and behind — so the mesh's far edges lift up into the haze
	// and the re-sampling at the boundary stays hidden (no visible "building up bit by bit").
	// SIZE 2000 (was 1200) keeps the edge ~1000 m out — past where the fold + light fog reveal it, so
	// you never see the patch boundary / sky-beyond-land as you walk. Same SEG → same vertex/resample
	// cost (10 m quads; fine on this flat-shaded low-poly terrain, flat near spawn anyway).
	const SIZE = 2000;
	const SEG = 200;
	const QUAD = SIZE / SEG; // 10 m terrain quad
	// Re-centre the (2000 m) mesh only every ~120 m, NOT every quad. Each re-centre re-samples all ~40 k
	// vertices (heightAt = ~10 trig each far from the origin → a ~12 ms hitch), and at 10 m that fired every
	// ~1 s of walking → a periodic exploration stutter. The mesh spans ±1000 m, so at ±60 m off-centre there's
	// still ~940 m of terrain ahead — well past where the curve-fold + fog hide the edge — so re-centring this
	// rarely is invisible but cuts the re-sample hitches ~12×. MUST stay a multiple of QUAD so vertices keep
	// landing on the same world grid (no shimmer when it does re-centre). ⚠️ heightfield is world-anchored, so
	// the terrain looks identical before/after a re-centre — only the (rare) re-sample cost changes.
	const SNAP = QUAD * 12; // 120 m

	let mesh = $state<THREE.Mesh>();

	const geometry = new THREE.PlaneGeometry(SIZE, SIZE, SEG, SEG);
	geometry.rotateX(-Math.PI / 2);

	// Procedural terrain shading — patches MeshStandardMaterial so the flat green mesh gains rock on steep
	// faces and snow on high peaks (so the 26 m ambient mountains read as MOUNTAINS, not green blobs), plus
	// subtle noise mottling everywhere. Slope is taken from screen-space derivatives of the PRE-curve world
	// position (so it follows the real terrain, not the visual Inception-fold), height from that same pos.
	// Rock/snow gated to natural (grass) ground via uNatural; lighting still uses the flat-shaded normals.
	const MAXW = 8; // GLSL cap on ponds whose damp shoreline darkens the bank
	const uniforms = {
		uNatural: { value: 1 },
		uCloud: { value: 1 },
		uWet: { value: 0 },
		uSnow: { value: 0 },
		uSand: { value: 0 },
		uWaterN: { value: 0 },
		uWater: { value: Array.from({ length: MAXW }, () => new THREE.Vector3()) } // x, z, size per pond
	};
	// initial colour is a placeholder — the $effect below sets it from world.ground (and keeps it in sync)
	const material = new THREE.MeshStandardMaterial({ color: GROUND_COLOR.grass, flatShading: true, side: THREE.DoubleSide });
	material.onBeforeCompile = (shader) => {
		shader.uniforms.uNatural = uniforms.uNatural;
		shader.uniforms.uCloud = uniforms.uCloud;
		shader.uniforms.uWet = uniforms.uWet;
		shader.uniforms.uSnow = uniforms.uSnow;
		shader.uniforms.uSand = uniforms.uSand;
		shader.uniforms.uWaterN = uniforms.uWaterN;
		shader.uniforms.uWater = uniforms.uWater;
		shader.uniforms.uTime = wind.uTime; // the shared foliage/wind clock (ticked once by Scene) → drifting shadows
		shader.vertexShader = shader.vertexShader
			.replace('#include <common>', '#include <common>\nvarying vec3 vWorldPos;')
			.replace('#include <begin_vertex>', '#include <begin_vertex>\nvWorldPos = (modelMatrix * vec4(transformed, 1.0)).xyz;');
		shader.fragmentShader = shader.fragmentShader
			.replace(
				'#include <common>',
				/* glsl */ `#include <common>
				varying vec3 vWorldPos;
				uniform float uNatural;
				uniform float uCloud;
				uniform float uWet;
				uniform float uSnow;
				uniform float uSand;
				uniform int uWaterN;
				uniform vec3 uWater[${MAXW}];
				uniform float uTime;
				float terHash(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
				float terNoise(vec2 p){
					vec2 i = floor(p), f = fract(p);
					float a = terHash(i), b = terHash(i + vec2(1.0, 0.0)), c = terHash(i + vec2(0.0, 1.0)), d = terHash(i + vec2(1.0, 1.0));
					vec2 u = f * f * (3.0 - 2.0 * f);
					return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
				}
				float terFbm(vec2 p){ float v = 0.0, a = 0.5; for (int i = 0; i < 4; i++) { v += a * terNoise(p); p *= 2.03; a *= 0.5; } return v; }`
			)
			.replace(
				'#include <color_fragment>',
				/* glsl */ `#include <color_fragment>
				vec3 terN = normalize(cross(dFdx(vWorldPos), dFdy(vWorldPos)));
				float slope = clamp(abs(terN.y), 0.0, 1.0);            // 1 = flat, 0 = cliff
				float mott = terFbm(vWorldPos.xz * 0.08);
				vec3 base = diffuseColor.rgb * (0.82 + 0.34 * mott);    // subtle patchy variation everywhere
				if (uNatural > 0.5) {
					// GRASS to the horizon: mid-frequency green tufts + a finer speckle so distant ground reads as a
					// textured meadow (matched to the 3D blade tone) instead of a bald flat-green plane where the
					// blades stop — this is what melts the grass field seamlessly into the far ground
					float tuft = terFbm(vWorldPos.xz * 0.55);
					base = mix(base, base * vec3(0.78, 1.12, 0.70), 0.24 * tuft);   // greener clumps of grass
					base *= 0.90 + 0.18 * terNoise(vWorldPos.xz * 1.3);             // fine blade-scale speckle
					// BIOME by elevation: lush green valleys → dry golden grass on the high ground (before rock takes
					// over) → the distant hills/plateaus read as varied country, not one flat shade of green
					float elev = smoothstep(3.0, 16.0, vWorldPos.y);
					base = mix(base, base * vec3(1.16, 1.04, 0.60), 0.45 * elev);          // hilltops → dry/golden
					base = mix(base, base * vec3(0.82, 1.06, 0.78), 0.20 * (1.0 - elev));  // valleys → lush
					vec3 rock = vec3(0.30, 0.28, 0.255) * (0.8 + 0.4 * mott);
					base = mix(base, rock, smoothstep(0.72, 0.42, slope)); // steep faces → exposed rock
					base = mix(base, vec3(0.90, 0.94, 1.0), smoothstep(15.0, 27.0, vWorldPos.y) * smoothstep(0.42, 0.72, slope)); // high + flattish → snow
				}
				if (uSnow > 0.5) {
					// SNOW ground (a winter world): wind-blown drifts — low-freq undulation reads as soft dunes,
					// the hollows shaded cool-blue (ambient sky bounce in a pit), crests bright; + a fine crunchy grain
					float drift = terFbm(vWorldPos.xz * 0.06);
					base *= 0.90 + 0.20 * drift;
					base = mix(base, base * vec3(0.86, 0.92, 1.10), 0.5 * (1.0 - drift)); // blue shadow in the hollows
					base *= 0.96 + 0.08 * terNoise(vWorldPos.xz * 2.2);                    // crunchy granularity
				}
				if (uSand > 0.5) {
					// DESERT ground: broad DUNES (low-freq undulation) + fine wind RIPPLES — directional bands warped
					// by noise so they meander like real aeolian ripples — and a warmer tone pooling in the hollows.
					// Matches the sand ZONE decal's rippled look so a desert world reads consistently underfoot.
					float dune = terFbm(vWorldPos.xz * 0.035);
					base *= 0.92 + 0.16 * dune;
					float ripple = sin(dot(vWorldPos.xz, vec2(0.8, 0.6)) * 2.6 + terFbm(vWorldPos.xz * 0.5) * 6.0);
					base *= 0.95 + 0.06 * ripple;                                          // crest catches light / trough shaded
					base = mix(base, base * vec3(1.07, 0.98, 0.82), 0.30 * (1.0 - dune));  // warm sand pooled in the hollows
				}
				// DAMP SHORELINE: the ground darkens in a band just outside each pond, so water meets a wet bank
				// (most visible on sand/snow) instead of abruptly meeting dry, bright land. Inside the waterline is
				// hidden by the Water mesh, so only the ~1.6 m ring on land shows.
				float shore = 0.0;
				for (int i = 0; i < ${MAXW}; i++) {
					if (i >= uWaterN) break;
					vec3 w = uWater[i];
					shore = max(shore, smoothstep(w.z + 1.6, w.z - 0.2, distance(vWorldPos.xz, w.xy)));
				}
				base *= 1.0 - 0.30 * shore;
				// drifting cloud shadows: sample the SAME fbm scale + drift as Clouds.svelte (·0.0022, slow), so the
				// shadow blobs actually match the clouds overhead (and their reflection in the water) — they were
				// ~5× smaller and ~100× faster, a visible disconnect. Offset by the sun angle (light (30,45,20),
				// deck ALT 130 → +(86.7,57.8) m) so each shadow falls where its cloud's shadow would land, not
				// straight below. Only the denser cores (high threshold) darken → thick clouds shadow, thin edges don't.
				float cloud = terFbm((vWorldPos.xz + vec2(86.7, 57.8)) * 0.0022 + vec2(uTime * 0.006, uTime * 0.004));
				base *= 1.0 - 0.30 * uCloud * smoothstep(0.55, 0.80, cloud);
				// RAIN wets the ground (uWet=1 under the 'fog'/rain sky): wet earth darkens, pooling MOST in the
				// flat low spots where puddles gather (slope: 1=flat). The glossy sheen is added as emissive below.
				float wet = uWet * (0.45 + 0.55 * smoothstep(0.80, 0.97, slope));
				base *= mix(1.0, 0.64, wet);
				diffuseColor.rgb = base;`
			)
			.replace(
				'#include <emissivemap_fragment>',
				/* glsl */ `#include <emissivemap_fragment>
				// WET SHEEN: flat wet ground glints with reflected overcast sky at grazing angles. Added as
				// EMISSIVE so it shows through the weak rain sun (a roughness/specular term would be near-invisible
				// in the gloom). Fresnel toward the viewer, pooled in the flats (where the puddles are).
				if (uWet > 0.01) {
					vec3 wN = normalize(cross(dFdx(vWorldPos), dFdy(vWorldPos)));
					float wetS = uWet * smoothstep(0.82, 0.985, clamp(abs(wN.y), 0.0, 1.0));
					vec3 wView = normalize(cameraPosition - vWorldPos);
					float fres = pow(1.0 - clamp(abs(dot(wView, wN)), 0.0, 1.0), 4.0);
					totalEmissiveRadiance += vec3(0.50, 0.56, 0.66) * fres * wetS * 0.6;
				}
				if (uSnow > 0.5) {
					// SNOW SPARKLE: sparse high-freq cells glint, each twinkling on its own phase as the view moves —
					// emissive so the crystals catch light even under a flat winter sky. Brighter on snow facing you.
					vec3 sN = normalize(cross(dFdx(vWorldPos), dFdy(vWorldPos)));
					float face = clamp(dot(normalize(cameraPosition - vWorldPos), sN), 0.0, 1.0);
					float cellH = terHash(floor(vWorldPos.xz * 40.0));
					float glint = step(0.985, cellH) * (0.5 + 0.5 * sin(uTime * 6.0 + cellH * 40.0)) * face;
					totalEmissiveRadiance += vec3(0.80, 0.90, 1.0) * glint * 0.7;
				}`
			);
	};

	$effect(() => {
		material.color.set(color);
		uniforms.uNatural.value = world.ground === 'grass' ? 1 : 0; // rock/snow only on natural ground
		uniforms.uCloud.value = CLOUD_SHADOW[world.sky] ?? 1; // cloud shadows fade with the sky (none in space)
		uniforms.uWet.value = world.sky === 'fog' ? 1 : 0; // the 'fog' sky is the rain weather → wet ground
		uniforms.uSnow.value = world.ground === 'snow' ? 1 : 0; // snowy world → drifts + sparkle (matches Weather's snow)
		uniforms.uSand.value = world.ground === 'sand' ? 1 : 0; // desert world → dunes + wind ripples (matches the sand zone decal)
		// ponds → a damp shoreline ring (reactive on world.zones); cap at MAXW, nearest aren't prioritised since it's soft
		let wn = 0;
		for (const z of world.zones ?? []) {
			if (wn >= MAXW || z.material !== 'water') continue;
			uniforms.uWater.value[wn++].set(z.pos[0], z.pos[2], z.size);
		}
		uniforms.uWaterN.value = wn;
	});

	// committed mesh state (the heights currently displayed match this centre + feature-count)
	let lastCx = NaN;
	let lastCz = NaN;
	let lastLen = -1;
	// Background re-sample: a re-centre's ~40k heightAt calls (~12 ms far from origin) used to run in ONE frame
	// → a dropped frame every time the mesh re-centres. Instead compute the new heights into a buffer a CHUNK at
	// a time over a few frames while the OLD mesh keeps showing (world-anchored + ~940 m of margin → seamless),
	// then swap atomically when ready. The first sample is synchronous (cheap near spawn) so the ground isn't
	// briefly flat on load. ⚠️ no computeVertexNormals — flatShading takes normals from screen-space derivatives.
	const vcount = geometry.attributes.position.count;
	const tmpHeights = new Float32Array(vcount);
	const CHUNK = Math.ceil(vcount / 4); // ~4 frames → ~3 ms each instead of one ~12 ms spike
	let building = false;
	let bIdx = 0;
	let bCx = NaN;
	let bCz = NaN;
	let bLen = -1;

	useTask(() => {
		if (!mesh) return;
		const pos = geometry.attributes.position;
		const cx = Math.round(playerState.pos[0] / SNAP) * SNAP;
		const cz = Math.round(playerState.pos[2] / SNAP) * SNAP;
		const len = world.terrain.length;

		if (Number.isNaN(lastCx)) {
			for (let i = 0; i < vcount; i++) pos.setY(i, heightAt(cx + pos.getX(i), cz + pos.getZ(i), world.terrain));
			pos.needsUpdate = true;
			mesh.position.set(cx, 0, cz);
			lastCx = bCx = cx;
			lastCz = bCz = cz;
			lastLen = bLen = len;
			return;
		}

		if (cx === lastCx && cz === lastCz && len === lastLen) {
			building = false; // committed mesh already matches the target → abandon any in-progress build
			return;
		}

		// (re)aim the background build at the newest target (restarts if the player moved on again mid-build)
		if (cx !== bCx || cz !== bCz || len !== bLen) {
			building = true;
			bIdx = 0;
			bCx = cx;
			bCz = cz;
			bLen = len;
		}

		const end = Math.min(bIdx + CHUNK, vcount);
		for (let i = bIdx; i < end; i++) tmpHeights[i] = heightAt(bCx + pos.getX(i), bCz + pos.getZ(i), world.terrain);
		bIdx = end;

		if (bIdx >= vcount) {
			// ready → swap in the new heights (just array copies, no heightAt) and re-centre. The old mesh showed
			// correct, world-anchored terrain the whole time, so there's NO visible jump — only the cost was spread.
			for (let i = 0; i < vcount; i++) pos.setY(i, tmpHeights[i]);
			pos.needsUpdate = true;
			mesh.position.set(bCx, 0, bCz);
			lastCx = bCx;
			lastCz = bCz;
			lastLen = bLen;
			building = false;
		}
	});
</script>

<!-- DoubleSide so the fold's reared-up far ground (curling back overhead) still renders -->
<T.Mesh bind:ref={mesh} {geometry} {material} receiveShadow castShadow />
