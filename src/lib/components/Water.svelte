<script lang="ts">
	// Procedural water for a lake/pond zone — a GPU shader, no assets (see [[shader-first-direction]]).
	// A flat grid whose vertices rise/fall as travelling sine waves (vertex shader), with an ORGANIC blob
	// shoreline carved by an angular-noise discard (so ponds aren't perfect circles), animated specular
	// glints, a soft translucent bank, and concentric RIPPLES that ring out around the player while they
	// wade in it. MeshStandardMaterial patched via onBeforeCompile so scene lighting + fog still apply.
	// Walking/avoidance is handled elsewhere: the player slows + sinks (Player.svelte sets playerState.
	// inWater), and animals treat the pond as an obstacle (Scene.svelte feeds it to agentManager).
	import { untrack } from 'svelte';
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { playerState } from '$lib/playerState.svelte';
	import { wind } from '$lib/wind';
	import { waterSeed, waterSurfaceY } from '$lib/water';
	import type { Zone, TerrainFeature } from '$lib/world';

	let { zone, sky = 'day', terrain = [] }: { zone: Zone; sky?: string; terrain?: TerrainFeature[] } = $props();

	// snapshot the zone once — each Water is keyed by zone id in Scene, so its zone never changes identity
	// (same intentional pattern as Npc/Critter); reading props at init is deliberate, hence untrack
	const Z = untrack(() => ({ size: zone.size, id: zone.id, pos: zone.pos }));

	// flat pond surface height — shared with LakeFish (so fish ride exactly at the surface, never under the
	// opaque water) via waterSurfaceY in water.ts. Sampled once at mount.
	const waterLevel = untrack(() => waterSurfaceY(zone, terrain));

	// the water reflects the SKY via an analytic horizon→zenith gradient sampled by the reflected view vector
	// (no cubemap / no render target). Per-sky horizon + zenith tints, plus sun-glint strength (fades at night).
	// `cloud` = the sky's cloud COVER (matches SkyDome's <Clouds cover>; 0 = no clouds → no cloud reflection)
	const SKY_REFLECT: Record<string, { hor: [number, number, number]; zen: [number, number, number]; glint: number; moon: number; cloud: number }> = {
		day: { hor: [0.66, 0.79, 0.94], zen: [0.24, 0.49, 0.86], glint: 1.0, moon: 0.0, cloud: 0.62 },
		sunset: { hor: [0.99, 0.67, 0.45], zen: [0.42, 0.38, 0.62], glint: 0.85, moon: 0.0, cloud: 0.62 },
		fog: { hor: [0.79, 0.82, 0.85], zen: [0.66, 0.7, 0.75], glint: 0.2, moon: 0.12, cloud: 0.8 },
		night: { hor: [0.11, 0.17, 0.32], zen: [0.02, 0.05, 0.15], glint: 0.22, moon: 1.0, cloud: 0.0 },
		space: { hor: [0.08, 0.08, 0.18], zen: [0.02, 0.02, 0.08], glint: 0.16, moon: 0.7, cloud: 0.0 }
	};

	const SEG = 64; // grid resolution → smooth Gerstner displacement + per-vertex analytic normals
	const geo = new THREE.PlaneGeometry(Z.size * 2, Z.size * 2, SEG, SEG);

	const uniforms = {
		uTime: { value: 0 },
		uPlayer: { value: new THREE.Vector2() },
		uPlayerWet: { value: 0 },
		uSize: { value: Z.size },
		uSeed: { value: waterSeed(Z.id) }, // shared with the gameplay wade check (see water.ts)
		uHorizon: { value: new THREE.Color(0.66, 0.79, 0.94) }, // sky reflection at the horizon (set from `sky`)
		uZenith: { value: new THREE.Color(0.24, 0.49, 0.86) }, //  ...and straight up
		uGlint: { value: 1 }, // sun-glint strength (no sun at night → faint)
		uMoon: { value: 0 }, // moonlight-trail strength (only at night/space)
		uRain: { value: 0 }, // raindrop dimples (only under the rainy/overcast 'fog' sky)
		uCloudCover: { value: 0.62 }, // cloud cover reflected in the surface (matches SkyDome's Clouds)
		uWind: wind.uTime // the SHARED wind clock (not the water's own uTime) → gusts sync with grass/trees
	};
	$effect(() => {
		const s = SKY_REFLECT[sky] ?? SKY_REFLECT.day;
		uniforms.uHorizon.value.setRGB(s.hor[0], s.hor[1], s.hor[2]);
		uniforms.uZenith.value.setRGB(s.zen[0], s.zen[1], s.zen[2]);
		uniforms.uGlint.value = s.glint;
		uniforms.uMoon.value = s.moon;
		uniforms.uRain.value = sky === 'fog' ? 1 : 0; // the Weather layer rains under the 'fog' sky
		uniforms.uCloudCover.value = s.cloud;
	});

	const COMMON = /* glsl */ `
		uniform float uTime;
		uniform vec2 uPlayer;
		uniform float uPlayerWet;
		uniform float uSize;
		uniform float uSeed;
		uniform vec3 uHorizon;
		uniform vec3 uZenith;
		uniform float uGlint;
		uniform float uMoon;
		uniform float uRain;
		uniform float uCloudCover;
		uniform float uWind;
		varying vec2 vLocal;
		varying vec2 vWorldXZ;
		varying vec3 vWorldPos;
		varying vec3 vWNorm;
		varying float vWave;
		#define TAU 6.2831853
		// value-noise fbm IDENTICAL to Clouds.svelte (same hash + 5 octaves ×2.02) so the clouds REFLECTED in
		// the water sit at the same world positions as the ones actually drifting overhead.
		float wHash(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
		float wNoise(vec2 p){ vec2 i = floor(p), f = fract(p); float a = wHash(i), b = wHash(i + vec2(1.0, 0.0)), c = wHash(i + vec2(0.0, 1.0)), d = wHash(i + vec2(1.0, 1.0)); vec2 u = f * f * (3.0 - 2.0 * f); return mix(mix(a, b, u.x), mix(c, d, u.x), u.y); }
		float wFbm(vec2 p){ float v = 0.0, amp = 0.5; for (int i = 0; i < 5; i++) { v += amp * wNoise(p); p *= 2.02; amp *= 0.5; } return v; }
		// a small stack of GERSTNER (trochoidal) waves — cresting closer to real water than plain sines, and
		// cheap enough for a pond. Returns the world-space displacement and writes the ANALYTIC surface normal
		// (GPU-Gems accumulation from the same wave params → no finite differencing, no neighbour samples).
		vec3 gerstner(vec2 p, float t, out vec3 nrm){
			vec2 DIR[4] = vec2[4](vec2(1.0, 0.0), vec2(0.7, 0.71), vec2(-0.6, 0.8), vec2(0.2, -0.98));
			vec4 WLEN = vec4(7.0, 4.5, 3.0, 2.0);          // wavelengths (m)
			vec4 AMP = vec4(0.045, 0.028, 0.018, 0.012);   // amplitudes (m) — gentle pond ripples; kept small so
			// wave TROUGHS never dip below the surface base (mesh sits +0.15 over terrain) and let the ground poke through
			vec4 STE = vec4(0.65, 0.55, 0.45, 0.4);    // steepness (crest sharpness)
			vec4 SPD = vec4(1.1, 1.5, 1.9, 2.3);       // phase speeds
			vec3 disp = vec3(0.0);
			nrm = vec3(0.0, 1.0, 0.0);
			for (int i = 0; i < 4; i++){
				vec2 d = normalize(DIR[i]);
				float k = TAU / WLEN[i];
				float a = AMP[i];
				float st = STE[i];
				float ph = k * dot(d, p) + t * SPD[i];
				float c = cos(ph), s = sin(ph);
				disp.x += st * a * d.x * c;   // horizontal pinch → sharp crests
				disp.z += st * a * d.y * c;
				disp.y += a * s;              // vertical rise/fall
				float wa = k * a;
				nrm.x -= d.x * wa * c;        // analytic normal accumulation
				nrm.z -= d.y * wa * c;
				nrm.y -= st * wa * s;
			}
			return disp;
		}
	`;

	// OPAQUE + writes depth (was transparent/depthWrite:false → you saw the ground AND fireflies straight
	// through it, so it never felt deep). The blob shoreline is still carved by `discard`, so it's an
	// alpha-TESTED opaque surface: it occludes everything behind/below (ground, fireflies, fish-depth) and
	// the sense of depth comes from the dark deep-water colour, not from being see-through.
	const mat = new THREE.MeshStandardMaterial({
		color: 0x3b6fb0,
		roughness: 0.16,
		metalness: 0.0,
		side: THREE.FrontSide
	});
	mat.onBeforeCompile = (shader) => {
		Object.assign(shader.uniforms, uniforms);
		shader.vertexShader = shader.vertexShader
			.replace('#include <common>', '#include <common>\n' + COMMON)
			.replace(
				'#include <beginnormal_vertex>',
				/* glsl */ `
				vec2 wxz = (modelMatrix * vec4(position, 1.0)).xz;
				vec3 gNrm;
				vec3 gDisp = gerstner(wxz, uTime, gNrm);
				// world→object normal (the plane is tilted −90° about X: local +X→world X, +Y→world −Z, +Z→world up)
				vec3 objectNormal = normalize(vec3(gNrm.x, -gNrm.z, gNrm.y));
				vWNorm = normalize(gNrm); // keep the WORLD normal for Fresnel + reflection in the fragment
				`
			)
			.replace(
				'#include <begin_vertex>',
				/* glsl */ `
				vLocal = position.xy;
				vWorldXZ = wxz;
				vWave = gDisp.y;
				vec3 transformed = position + vec3(gDisp.x, -gDisp.z, gDisp.y); // displace in local space (see tilt above)
				vWorldPos = (modelMatrix * vec4(transformed, 1.0)).xyz;
				`
			);
		shader.fragmentShader = shader.fragmentShader
			.replace('#include <common>', '#include <common>\n' + COMMON)
			.replace(
				'#include <color_fragment>',
				/* glsl */ `
				#include <color_fragment>
				float r = length(vLocal);
				float ang = atan(vLocal.y, vLocal.x);
				float e = 0.80 + 0.11 * sin(ang * 3.0 + uSeed) + 0.07 * sin(ang * 5.0 - uSeed * 1.7) + 0.045 * sin(ang * 7.0 + uSeed * 2.3);
				float edge = uSize * e;
				if (r > edge) discard; // organic blob shoreline (matches the CPU wade test in water.ts)

				vec3 N = normalize(vWNorm);
				// FINE RIPPLES: a few high-frequency sines analytically perturb the macro wave normal so the
				// surface sparkles (sun glitter) and the reflection breaks up, instead of a smooth glassy sheen.
				// Cheap, no noise texture; the coarse per-vertex Gerstner normal alone read flat. (Stop-gap detail
				// pass — the full depth-texture water redo is still the planned fix, see water-redo-plan.)
				vec3 rn = vec3(0.0);
				vec2 d0 = vec2(0.80, 0.60); float p0 = 5.71 * dot(d0, vWorldXZ) + uTime * 3.1; float a0 = 5.71 * 0.010;
				rn.x -= d0.x * a0 * cos(p0); rn.z -= d0.y * a0 * cos(p0);
				vec2 d1 = vec2(-0.64, 0.77); float p1 = 8.4 * dot(d1, vWorldXZ) - uTime * 3.7; float a1 = 8.4 * 0.006;
				rn.x -= d1.x * a1 * cos(p1); rn.z -= d1.y * a1 * cos(p1);
				vec2 d2 = vec2(0.2, -0.98); float p2 = 12.0 * dot(d2, vWorldXZ) + uTime * 4.6; float a2 = 12.0 * 0.004;
				rn.x -= d2.x * a2 * cos(p2); rn.z -= d2.y * a2 * cos(p2);
				// WIND: the lake roughens with the same gust that leans the grass/trees (shared uWind clock, the
				// WIND_GUST curve) → glassy when calm, choppier mid-gust. Ripple-NORMAL only (no wave geometry), so
				// troughs can't dip below the surface and expose the lakebed. The sun/moon glint reads N → sparkles more too.
				rn *= (1.0 + 0.4 * sin(uWind * 0.23) + 0.25 * sin(uWind * 0.07 + 1.7));
				N = normalize(N + rn);

				// RAIN: under the rainy 'fog' sky the surface is pocked by raindrops — a moving grid of expanding
				// rings (each cell drops on its own phase) tilts the normal, so the reflection breaks into the
				// jittery dimpled look of rain on water. Costs nothing when it's not raining (branch skipped).
				if (uRain > 0.5) {
					vec2 rc = floor(vWorldXZ * 1.6);
					vec2 rf = fract(vWorldXZ * 1.6) - 0.5;
					float rs = fract(sin(dot(rc, vec2(12.9, 78.2))) * 43758.5);
					float rph = fract(uTime * 1.4 + rs);                 // 0 (drop hits) .. 1 (ring faded)
					float rd = length(rf);
					float ring = sin(rd * 34.0 - rph * 34.0) * exp(-rd * 7.0) * (1.0 - rph) * smoothstep(0.5, 0.0, rd);
					vec2 g = (rf / (rd + 1e-4)) * ring;                  // radial ring → normal tilt
					N = normalize(N + vec3(g.x, 0.0, g.y) * 0.7);
				}

				vec3 V = normalize(cameraPosition - vWorldPos);
				// Schlick Fresnel, F0 = 0.02 (water vs air) — grazing angles reflect the sky, near-normal shows
				// the body. This Fresnel-weighted blend is THE step that stops water reading as a flat dark disc.
				float fres = 0.02 + 0.98 * pow(1.0 - clamp(dot(N, V), 0.0, 1.0), 5.0);

				vec3 R = reflect(-V, N);                                              // reflected view ray
				vec3 sky = mix(uHorizon, uZenith, clamp(R.y, 0.0, 1.0));              // analytic horizon→zenith reflection
				// CLOUD reflection: trace the reflected ray up to the cloud deck (ALT 130) and sample the SAME
				// world-space noise + cover as Clouds.svelte → the lake mirrors the actual clouds drifting above,
				// at matching positions. Only when there ARE clouds and we're looking up at the sky (R.y > 0).
				if (uCloudCover > 0.01 && R.y > 0.04) {
					vec2 cp = vWorldPos.xz + R.xz * ((130.0 - vWorldPos.y) / R.y);
					float n = wFbm(cp * 0.0022 + vec2(uTime * 0.006, uTime * 0.004));
					float lo = mix(0.58, 0.28, uCloudCover);
					float cl = smoothstep(lo, lo + 0.30, n) * smoothstep(0.04, 0.18, R.y); // fade at grazing angles
					sky = mix(sky, mix(uHorizon, vec3(1.0), 0.7), cl * 0.7);              // bright reflected cloud
				}
				// shallow→deep body: the centre goes much darker so the lake reads as DEEP (we can't sample scene
				// depth, so the darkening + full opacity below IS the depth cue) instead of a flat tinted pane.
				// r is distance-from-CENTRE, so deepness must INVERT it: 1 in the middle, 0 (shallow) at the bank
				// — otherwise the shoreline paints dark and the centre light, the opposite of real water.
				float depth = 1.0 - smoothstep(0.0, edge * 0.9, r); // 1 in the deep middle → 0 at the shallow bank
				vec3 body = mix(vec3(0.12, 0.36, 0.46), vec3(0.01, 0.07, 0.16), depth); // light teal shallows → dark navy depths
				vec3 col = mix(body, sky, fres);

				vec3 sun = normalize(vec3(30.0, 45.0, 20.0));                         // matches Scene's directional sun
				float spec = dot(N, normalize(sun + V));
				col += uGlint * (pow(max(spec, 0.0), 140.0) * 2.0 + pow(max(spec, 0.0), 30.0) * 0.25) * vec3(1.0, 0.95, 0.82); // sharp glitter + soft sheen off the rippled facets

				// MOONLIGHT TRAIL — at night a cool shimmering streak runs toward the moon (matches Moon.svelte's
				// direction). The fine ripple normals break it into the classic scattered moon-glitter on water.
				vec3 moonDir = normalize(vec3(30.0, 45.0, 20.0)); // == the dir-light / Moon.svelte direction → the trail points right at the visible moon
				float mspec = dot(N, normalize(moonDir + V));
				col += uMoon * (pow(max(mspec, 0.0), 130.0) * 1.7 + pow(max(mspec, 0.0), 26.0) * 0.18) * vec3(0.74, 0.82, 1.0);

				// LAPPING shoreline foam: the foam line rolls in and out (two angular waves drifting at different
				// rates) and waxes/wanes around the bank, so the shore breathes like real water lapping — not a
				// static ring. Plus white crest tips out on the open water.
				float lapEdge = edge + sin(ang * 7.0 - uTime * 1.5) * 0.45 + sin(ang * 13.0 + uTime * 0.9) * 0.22;
				float foam = smoothstep(0.85, 0.0, abs(r - lapEdge)) * (0.5 + 0.5 * sin(ang * 11.0 - uTime * 2.1));
				foam += smoothstep(0.13, 0.21, vWave);                               // white crest tips
				col = mix(col, vec3(0.90, 0.95, 1.0), clamp(foam, 0.0, 1.0) * 0.6);

				float d = distance(vWorldXZ, uPlayer);
				col += sin(d * 3.5 - uTime * 7.0) * exp(-d * 0.5) * uPlayerWet * smoothstep(7.0, 0.0, d) * 0.16; // wading wake
				diffuseColor.rgb = col;

				// opaque surface (alpha stays 1) — depth comes from the dark body + the bank colour, not see-through
				diffuseColor.a = 1.0;
				`
			);
	};

	const mesh = new THREE.Mesh(geo, mat);
	mesh.rotation.x = -Math.PI / 2;
	mesh.position.set(Z.pos[0], waterLevel, Z.pos[2]); // highest lakebed terrain + margin → covers the whole blob
	mesh.renderOrder = 1;

	useTask((dt) => {
		uniforms.uTime.value += dt;
		uniforms.uPlayer.value.set(playerState.pos[0], playerState.pos[2]);
		uniforms.uPlayerWet.value = playerState.inWater ? 1 : 0;
	});
</script>

<T is={mesh} />
