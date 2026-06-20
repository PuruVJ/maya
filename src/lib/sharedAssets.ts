// Shared, module-level geometries + materials. Building objects REUSES these instead of allocating
// a fresh geometry/material per mesh and re-uploading to the GPU (+ recompiling shaders) — which is
// what freezes the frame when a build pops in, especially many at once ("30 cats", a scattered
// forest). Cached assets are immutable + shared; we never mutate them (paint just selects a
// different cached material). See docs/crowd-separation.md §3.3.
import * as THREE from 'three';
import type { Part } from './kinds';
import { wind, swayVertex, WIND_NOISE } from './wind';
import { applyWeather } from './weather';

const geoCache = new Map<string, THREE.BufferGeometry>();

/** A cached geometry for a kind's part (keyed by shape + dimensions → shared across all objects). */
export function partGeo(part: Part): THREE.BufferGeometry {
	const key = part.geo + ':' + part.args.join(',');
	let g = geoCache.get(key);
	if (!g) {
		const a = part.args;
		g =
			part.geo === 'box'
				? new THREE.BoxGeometry(a[0], a[1], a[2])
				: part.geo === 'cyl'
					? new THREE.CylinderGeometry(a[0], a[0], a[1], 12)
					: part.geo === 'cone'
						? new THREE.ConeGeometry(a[0], a[1], 12)
						: part.geo === 'pyramid'
							? new THREE.ConeGeometry(a[0], a[1], 4)
							: new THREE.SphereGeometry(a[0], 12, 10);
		geoCache.set(key, g);
	}
	return g;
}

const matCache = new Map<string, THREE.MeshStandardMaterial>();

/** A cached flat-shaded material for a colour (shared → its shader compiles once, reused forever). */
export function litMat(color: string, emissive = false): THREE.MeshStandardMaterial {
	const key = color + (emissive ? '|e' : '');
	let m = matCache.get(key);
	if (!m) {
		m = new THREE.MeshStandardMaterial({
			color,
			flatShading: true,
			emissive: emissive ? color : '#000000',
			emissiveIntensity: emissive ? 0.8 : 0
		});
		matCache.set(key, m);
	}
	return m;
}

// Cached shaded material for CREATURES (cats/people/etc.) — like litMat but the flat body colour gains a
// soft top-light AO (undersides darker, tops catch the sky), faint coat mottling AND a per-species
// procedural COAT PATTERN (no textures, native-generation): tabby stripes, reptile scales, fur grain.
// World normal from mat3(modelMatrix)·normal (good enough under the critters' ~uniform scale), so the
// gradient stays correct as legs swing / bodies bob. `pattern` picks the coat; distinct GLSL is injected
// per pattern (no runtime branch) and the cache is keyed by colour+pattern. Default 'plain' → unchanged,
// so Npc and untextured calls are unaffected. Scoped to Critter/Npc (NOT props). See [[shader-first-direction]].
export type CoatPattern = 'plain' | 'stripe' | 'scale' | 'fur' | 'soft';
const COAT: Record<CoatPattern, string> = {
	plain: '',
	// soft mammal (rabbit/kangaroo): gentle dappled fur + a pale COUNTER-SHADED belly (the light underside
	// real rabbits/roos have) — subtle, so it reads as soft fur rather than a pattern
	soft: /* glsl */ `
		float crSoft = crNoise(vCreatureP.xz * 5.0 + vCreatureP.y * 2.0);
		diffuseColor.rgb *= 0.93 + 0.12 * crSoft;                              // gentle dappled fur
		diffuseColor.rgb *= 1.0 + 0.18 * smoothstep(0.1, -0.5, vCreatureN.y);  // pale soft belly`,
	// tabby: darker bands running across the back (along body length z), wobbled by noise so they're organic
	stripe: /* glsl */ `
		float crBand = sin(vCreatureP.z * 13.0 + crNoise(vCreatureP.xy * 3.0) * 3.0);
		diffuseColor.rgb *= 1.0 - 0.30 * smoothstep(0.15, 0.75, crBand);`,
	// reptile: cellular scale plates (dark grooves between scales) + paler counter-shaded belly
	scale: /* glsl */ `
		float crD = crCell((vCreatureP.xz + vCreatureP.yy * 0.6) * 9.0);
		diffuseColor.rgb *= 1.0 - 0.26 * smoothstep(0.18, 0.62, crD);     // grooves between scales
		diffuseColor.rgb *= 1.0 + 0.14 * smoothstep(0.15, -0.45, vCreatureN.y); // lighter belly`,
	// fur: fine streaky grain stretched along the body (lots of detail across x, smeared along z)
	fur: /* glsl */ `
		float crGrain = crNoise(vec2(vCreatureP.x * 34.0, vCreatureP.z * 7.0));
		diffuseColor.rgb *= 0.88 + 0.22 * crGrain;`
};
// shared NIGHT level (0 day … 1 night) → drives a cool moonlit RIM on every creature so living things separate
// from the dark backdrop and read as solid forms (one uniform object, like wind.uTime, referenced by every
// cached creatureMat). Set by setEyeshine alongside the eyeshine.
export const creatureNight = { value: 0 };
const creatureCache = new Map<string, THREE.MeshStandardMaterial>();
export function creatureMat(color: string, pattern: CoatPattern = 'plain'): THREE.MeshStandardMaterial {
	const key = color + '|' + pattern;
	let m = creatureCache.get(key);
	if (m) return m;
	m = new THREE.MeshStandardMaterial({ color, flatShading: true });
	m.onBeforeCompile = (shader) => {
		shader.uniforms.uNight = creatureNight;
		shader.vertexShader = shader.vertexShader
			.replace('#include <common>', '#include <common>\nvarying vec3 vCreatureN;\nvarying vec3 vCreatureP;')
			.replace(
				'#include <begin_vertex>',
				'#include <begin_vertex>\nvCreatureN = normalize(mat3(modelMatrix) * normal);\nvCreatureP = position;'
			);
		shader.fragmentShader = shader.fragmentShader
			.replace(
				'#include <common>',
				/* glsl */ `#include <common>
				uniform float uNight;
				varying vec3 vCreatureN;
				varying vec3 vCreatureP;
				float crHash(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
				float crNoise(vec2 p){
					vec2 i = floor(p), f = fract(p);
					float a = crHash(i), b = crHash(i + vec2(1.0, 0.0)), c = crHash(i + vec2(0.0, 1.0)), d = crHash(i + vec2(1.0, 1.0));
					vec2 u = f * f * (3.0 - 2.0 * f);
					return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
				}
				// distance to nearest cell point (Worley) → scale-plate grooves; bounded 3×3 so it stays cheap
				float crCell(vec2 p){
					vec2 ip = floor(p), fp = fract(p);
					float d = 1.0;
					for (int y = -1; y <= 1; y++) for (int x = -1; x <= 1; x++) {
						vec2 g = vec2(float(x), float(y));
						vec2 o = vec2(crHash(ip + g), crHash(ip + g + 17.1));
						d = min(d, length(g + o - fp));
					}
					return d;
				}`
			)
			.replace(
				'#include <color_fragment>',
				/* glsl */ `#include <color_fragment>
				float crAO = 0.72 + 0.28 * (vCreatureN.y * 0.5 + 0.5);          // undersides darker, tops lit
				float crMott = crNoise(vCreatureP.xz * 6.0 + vCreatureP.y * 4.0); // faint coat tone variation
				diffuseColor.rgb *= crAO * (0.93 + 0.14 * crMott);${COAT[pattern]}`
			)
			.replace(
				'#include <emissivemap_fragment>',
				/* glsl */ `#include <emissivemap_fragment>
				// cool MOONLIT RIM — a grazing edge-light (Fresnel) only at night, so a creature lifts off the dark
				// backdrop and reads as a solid form. Additive emissive, night-gated → daytime is byte-identical.
				float crRim = pow(1.0 - clamp(dot(normalize(normal), normalize(vViewPosition)), 0.0, 1.0), 2.5);
				totalEmissiveRadiance += vec3(0.40, 0.50, 0.72) * (crRim * uNight * 0.55);`
			);
	};
	creatureCache.set(key, m);
	return m;
}

// Cached shaded material for generic PROPS (rocks/crates/wells/fences/…) — like litMat but with the same
// top-light AO + faint mottle as creatureMat, so static props read as rounded, grounded volumes instead of
// flat cut-outs (rocks especially). Emissive is preserved (glow is added separately, unaffected by the AO).
const propCache = new Map<string, THREE.MeshStandardMaterial>();
export function propMat(color: string, emissive = false): THREE.MeshStandardMaterial {
	const key = color + (emissive ? '|e' : '');
	let m = propCache.get(key);
	if (m) return m;
	m = new THREE.MeshStandardMaterial({
		color,
		flatShading: true,
		emissive: emissive ? color : '#000000',
		emissiveIntensity: emissive ? 0.8 : 0
	});
	m.onBeforeCompile = (shader) => {
		shader.vertexShader = shader.vertexShader
			.replace('#include <common>', '#include <common>\nvarying vec3 vPropN;\nvarying vec3 vPropP;')
			.replace(
				'#include <begin_vertex>',
				'#include <begin_vertex>\nvPropN = normalize(mat3(modelMatrix) * normal);\nvPropP = position;'
			);
		shader.fragmentShader = shader.fragmentShader
			.replace(
				'#include <common>',
				/* glsl */ `#include <common>
				varying vec3 vPropN;
				varying vec3 vPropP;
				float prHash(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
				float prNoise(vec2 p){
					vec2 i = floor(p), f = fract(p);
					float a = prHash(i), b = prHash(i + vec2(1.0, 0.0)), c = prHash(i + vec2(0.0, 1.0)), d = prHash(i + vec2(1.0, 1.0));
					vec2 u = f * f * (3.0 - 2.0 * f);
					return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
				}`
			)
			.replace(
				'#include <color_fragment>',
				/* glsl */ `#include <color_fragment>
				float prAO = 0.78 + 0.22 * (vPropN.y * 0.5 + 0.5);             // tops lit, undersides shaded
				float prM = prNoise(vPropP.xz * 3.0 + vPropP.y * 2.0);          // faint surface variation
				diffuseColor.rgb *= prAO * (0.94 + 0.12 * prM);`
			);
		applyWeather(shader, 0.25);
	};
	propCache.set(key, m);
	return m;
}

// Cached BOULDER material for rock props — propMat's top-light AO + faceting (flatShading), PLUS dark crevice
// CRACKS (thresholded noise veins) and mossy LICHEN clinging to up-facing surfaces, so a scattered rock reads
// as a weathered stone, not a smooth grey ball. Pure shader, no textures.
const rockCache = new Map<string, THREE.MeshStandardMaterial>();
export function rockMat(color: string): THREE.MeshStandardMaterial {
	let m = rockCache.get(color);
	if (m) return m;
	m = new THREE.MeshStandardMaterial({ color, flatShading: true });
	m.onBeforeCompile = (shader) => {
		shader.vertexShader = shader.vertexShader
			.replace('#include <common>', '#include <common>\nvarying vec3 vRockN;\nvarying vec3 vRockP;')
			.replace(
				'#include <begin_vertex>',
				'#include <begin_vertex>\nvRockN = normalize(mat3(modelMatrix) * normal);\nvRockP = position;'
			);
		shader.fragmentShader = shader.fragmentShader
			.replace(
				'#include <common>',
				/* glsl */ `#include <common>
				varying vec3 vRockN;
				varying vec3 vRockP;
				float rkHash(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
				float rkNoise(vec2 p){
					vec2 i = floor(p), f = fract(p);
					float a = rkHash(i), b = rkHash(i + vec2(1.0, 0.0)), c = rkHash(i + vec2(0.0, 1.0)), d = rkHash(i + vec2(1.0, 1.0));
					vec2 u = f * f * (3.0 - 2.0 * f);
					return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
				}`
			)
			.replace(
				'#include <color_fragment>',
				/* glsl */ `#include <color_fragment>
				float rAO = 0.74 + 0.26 * (vRockN.y * 0.5 + 0.5);                          // tops catch light
				float grain = rkNoise(vRockP.xz * 2.5 + vRockP.y * 2.0);                   // facet tone variation
				vec3 rcol = diffuseColor.rgb * rAO * (0.88 + 0.22 * grain);
				float crack = smoothstep(0.46, 0.30, rkNoise(vRockP.xz * 9.0 + vRockP.yy * 7.0)); // dark crevices
				rcol *= 1.0 - 0.45 * crack;
				float lich = smoothstep(0.62, 0.82, rkNoise(vRockP.xz * 3.5 - vRockP.y * 1.5)) * smoothstep(0.15, 0.7, vRockN.y);
				rcol = mix(rcol, vec3(0.42, 0.5, 0.34), lich * 0.55);                      // mossy lichen on the top
				diffuseColor.rgb = rcol;`
			);
		applyWeather(shader, 0.3);
	};
	rockCache.set(color, m);
	return m;
}

// Cached STONE-MASONRY material (the well's shaft, and any stone cylinder) — horizontal COURSES of blocks
// with staggered (running-bond) vertical joints and dark mortar, per-block tone, top-light AO. Pure shader.
const stoneCache = new Map<string, THREE.MeshStandardMaterial>();
export function stoneMat(color: string): THREE.MeshStandardMaterial {
	let m = stoneCache.get(color);
	if (m) return m;
	m = new THREE.MeshStandardMaterial({ color, flatShading: true });
	m.onBeforeCompile = (shader) => {
		shader.vertexShader = shader.vertexShader
			.replace('#include <common>', '#include <common>\nvarying vec3 vStoneN;\nvarying vec3 vStoneP;')
			.replace(
				'#include <begin_vertex>',
				'#include <begin_vertex>\nvStoneN = normalize(mat3(modelMatrix) * normal);\nvStoneP = position;'
			);
		shader.fragmentShader = shader.fragmentShader
			.replace(
				'#include <common>',
				/* glsl */ `#include <common>
				varying vec3 vStoneN;
				varying vec3 vStoneP;
				float stHash(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }`
			)
			.replace(
				'#include <color_fragment>',
				/* glsl */ `#include <color_fragment>
				float radius = length(vStoneP.xz);
				float arc = atan(vStoneP.x, vStoneP.z);                       // angle around the shaft
				float course = vStoneP.y / 0.26;                             // horizontal courses (~0.26 m tall)
				float cF = fract(course);
				float bx = arc * radius / 0.55 + step(0.5, fract(course * 0.5)) * 0.5; // blocks, staggered per course
				float bF = fract(bx);
				float mortar = max(smoothstep(0.07, 0.0, min(cF, 1.0 - cF)), smoothstep(0.06, 0.0, min(bF, 1.0 - bF)));
				float tone = 0.84 + 0.30 * stHash(floor(vec2(bx, course))); // each block a touch different
				diffuseColor.rgb *= (0.78 + 0.22 * (vStoneN.y * 0.5 + 0.5)) * tone; // top-light AO + block tone
				diffuseColor.rgb = mix(diffuseColor.rgb, diffuseColor.rgb * 0.42, mortar); // recessed dark mortar joints`
			);
		applyWeather(shader, 0.24);
	};
	stoneCache.set(color, m);
	return m;
}

// Cached WEATHERED-WOOD material (fences, bridges) — top-light AO + plank-to-plank tone variation, a fine
// grain texture, and occasional dark KNOTS, so wood props read as aged timber instead of flat brown. The
// grain is isotropic (non-directional) on purpose: a true along-the-plank grain needs each part's long axis
// passed in, which is fiddly — the knots + tone are what sell it as wood. Pure shader, no textures.
const woodCache = new Map<string, THREE.MeshStandardMaterial>();
export function woodMat(color: string): THREE.MeshStandardMaterial {
	let m = woodCache.get(color);
	if (m) return m;
	m = new THREE.MeshStandardMaterial({ color, flatShading: true });
	m.onBeforeCompile = (shader) => {
		shader.vertexShader = shader.vertexShader
			.replace('#include <common>', '#include <common>\nvarying vec3 vWoodN;\nvarying vec3 vWoodP;')
			.replace(
				'#include <begin_vertex>',
				'#include <begin_vertex>\nvWoodN = normalize(mat3(modelMatrix) * normal);\nvWoodP = position;'
			);
		shader.fragmentShader = shader.fragmentShader
			.replace(
				'#include <common>',
				/* glsl */ `#include <common>
				varying vec3 vWoodN;
				varying vec3 vWoodP;
				float wdHash(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
				float wdNoise(vec2 p){
					vec2 i = floor(p), f = fract(p);
					float a = wdHash(i), b = wdHash(i + vec2(1.0, 0.0)), c = wdHash(i + vec2(0.0, 1.0)), d = wdHash(i + vec2(1.0, 1.0));
					vec2 u = f * f * (3.0 - 2.0 * f);
					return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
				}`
			)
			.replace(
				'#include <color_fragment>',
				/* glsl */ `#include <color_fragment>
				float wAO = 0.78 + 0.22 * (vWoodN.y * 0.5 + 0.5);                          // tops catch light
				float plank = wdNoise(vWoodP.xz * 1.2 + vWoodP.y * 1.0);                   // plank-to-plank tone
				vec3 wcol = diffuseColor.rgb * wAO * (0.86 + 0.24 * plank);
				wcol *= 0.92 + 0.12 * wdNoise(vWoodP.xz * 14.0 + vWoodP.y * 11.0);          // fine grain texture
				float knot = smoothstep(0.84, 0.92, wdNoise(vWoodP.xz * 2.2 + vWoodP.y * 1.8));
				wcol = mix(wcol, wcol * vec3(0.5, 0.38, 0.28), knot * 0.7);                 // dark knots
				diffuseColor.rgb = wcol;`
			);
		applyWeather(shader, 0.26);
	};
	woodCache.set(color, m);
	return m;
}

// Cached FOLIAGE material — leafy shrub/bush: the SAME wind sway as a tree canopy (per-object phase from the
// model matrix, shared wind clock) plus an fbm leaf-dapple + darker underside, so a PLACED bush reads as
// living foliage that bends in the breeze instead of a flat green ball (matching the ambient-scatter bushes).
// One shared program across all bushes (identical source); the per-bush phase lives in the shader.
const foliageCache = new Map<string, THREE.MeshStandardMaterial>();
export function foliageMat(color: string): THREE.MeshStandardMaterial {
	let m = foliageCache.get(color);
	if (m) return m;
	m = new THREE.MeshStandardMaterial({ color, flatShading: true });
	m.onBeforeCompile = (shader) => {
		shader.uniforms.uTime = wind.uTime;
		shader.vertexShader = shader.vertexShader
			.replace('#include <common>', '#include <common>\nuniform float uTime;\nvarying vec3 vLocalPos;')
			.replace('#include <begin_vertex>', '#include <begin_vertex>\n' + swayVertex(0.07));
		shader.fragmentShader = shader.fragmentShader
			.replace('#include <common>', '#include <common>\nvarying vec3 vLocalPos;\n' + WIND_NOISE)
			.replace(
				'#include <color_fragment>',
				/* glsl */ `#include <color_fragment>
				float fol = windFbm(vLocalPos.xz * 1.7 + vLocalPos.y * 0.7);
				diffuseColor.rgb *= 0.76 + 0.4 * fol;                                  // dappled leaf clumps
				diffuseColor.rgb *= 0.7 + 0.32 * smoothstep(-0.6, 0.6, vLocalPos.y);   // darker underside`
			);
		applyWeather(shader, 0.15, 0.0); // bushes catch snow + a light rain-darken, but NO glossy sheen on leaves
	};
	foliageCache.set(color, m);
	return m;
}

// Cached FLOWER-BLOOM material — a vivid petal colour with a yellow EYE at the top and a few petal LOBES
// (radial), so a placed flower's bloom reads as a flower, not a coloured ball. Per-flower colour variety is
// chosen in Prop (hash of the id) and passed in here, so a scatter is a mixed wildflower patch.
const flowerCache = new Map<string, THREE.MeshStandardMaterial>();
export function flowerMat(color: string): THREE.MeshStandardMaterial {
	let m = flowerCache.get(color);
	if (m) return m;
	m = new THREE.MeshStandardMaterial({ color, flatShading: true });
	m.onBeforeCompile = (shader) => {
		shader.vertexShader = shader.vertexShader
			.replace('#include <common>', '#include <common>\nvarying vec3 vFlN;\nvarying vec3 vFlP;')
			.replace(
				'#include <begin_vertex>',
				'#include <begin_vertex>\nvFlN = normalize(mat3(modelMatrix) * normal);\nvFlP = position;'
			);
		shader.fragmentShader = shader.fragmentShader
			.replace('#include <common>', '#include <common>\nvarying vec3 vFlN;\nvarying vec3 vFlP;')
			.replace(
				'#include <color_fragment>',
				/* glsl */ `#include <color_fragment>
				float eye = smoothstep(0.5, 0.85, vFlN.y);                       // yellow centre at the top
				vec3 fcol = mix(diffuseColor.rgb, vec3(0.97, 0.82, 0.22), eye);
				float petals = 0.5 + 0.5 * sin(atan(vFlP.x, vFlP.z) * 5.0);       // 5 petal lobes
				fcol *= 1.0 - 0.20 * (1.0 - petals) * (1.0 - eye);               // shade between petals (not the eye)
				fcol *= 0.84 + 0.16 * (vFlN.y * 0.5 + 0.5);                       // gentle top-light
				diffuseColor.rgb = fcol;`
			);
	};
	flowerCache.set(color, m);
	return m;
}

// Shared foliage palette so the AMBIENT forest (instanceColor) and PLACED broadleaf trees (Tree.svelte) draw
// from the SAME colours → a planted "make forest" blends with the wild forest around it. `leafColorHex(h)`
// with h in 0..1 → mostly varied greens, ~8% autumn. Pines stay evergreen (they don't call this).
export const LEAF_GREENS = ['#3f7a44', '#356b3b', '#2e5e35', '#4a8a40', '#588a3c', '#6a8a36'];
export const LEAF_AUTUMN = ['#a86a2e', '#b58a32', '#9a5a2a'];
export function leafColorHex(h: number): string {
	return h < 0.92
		? LEAF_GREENS[Math.min(LEAF_GREENS.length - 1, Math.floor((h / 0.92) * LEAF_GREENS.length))]
		: LEAF_AUTUMN[Math.min(LEAF_AUTUMN.length - 1, Math.floor(((h - 0.92) / 0.08) * LEAF_AUTUMN.length))];
}

// Shared glossy-dark EYE material — low roughness so the directional light leaves a tiny catch-light, which
// reads as a living eye. Used by PEOPLE (humans have no tapetum lucidum → no eyeshine, so it never glows).
export const EYE_MAT = new THREE.MeshStandardMaterial({ color: '#16131c', roughness: 0.22, metalness: 0.0 });

// ANIMAL eyes glow at NIGHT — eyeshine (a tapetum lucidum reflecting the moonlight). Two tones: PREY a cool
// pale glint, PREDATORS a warmer, brighter amber so a hunter watching you from the dark reads as a threat.
// Driven by `setEyeshine(night)` (Scene calls it from the sky) — emissiveIntensity ramps 0→glow after dark.
export const EYE_PREY_MAT = new THREE.MeshStandardMaterial({ color: '#16131c', roughness: 0.22, emissive: '#b9d8c0' });
export const EYE_PRED_MAT = new THREE.MeshStandardMaterial({ color: '#1a1410', roughness: 0.22, emissive: '#ffb347' });
// a predator actively CHARGING you (m.hunting) swaps to this — a hot red glare so the beast bearing down on you
// burns its eyes at you (pairs with the danger vignette). Critter swaps to it per-agent in its hot loop.
export const EYE_HUNT_MAT = new THREE.MeshStandardMaterial({ color: '#1a0c0c', roughness: 0.22, emissive: '#ff1e10' });
EYE_PREY_MAT.emissiveIntensity = 0;
EYE_PRED_MAT.emissiveIntensity = 0;
EYE_HUNT_MAT.emissiveIntensity = 0;
/** Ramp the animal eyeshine with how nocturnal it is (0 day … 1 night). Shared mats → one call lights every
 *  animal's eyes (predators brighter, a charging hunter blazes red). Day → 0 (dark eyes with a catch-light). */
export function setEyeshine(night: number): void {
	const n = Math.max(0, Math.min(1, night));
	EYE_PREY_MAT.emissiveIntensity = 0.7 * n;
	EYE_PRED_MAT.emissiveIntensity = 1.15 * n;
	EYE_HUNT_MAT.emissiveIntensity = 1.7 * n;
	creatureNight.value = n; // the moonlit rim on every creature ramps with the night too
}

// Unit primitives — scale per body part via <T.Mesh scale={[w,h,d]}>. One geometry each, shared by
// every animal part (scaling a shared geometry is free — no extra GPU upload), so the distinct
// per-species Critter designs cost nothing extra to vary. sphere/cyl base radius 0.5 → scale = size.
export const PRIM = {
	box: new THREE.BoxGeometry(1, 1, 1),
	sphere: new THREE.SphereGeometry(0.5, 12, 10),
	cone: new THREE.ConeGeometry(0.5, 1, 8),
	cyl: new THREE.CylinderGeometry(0.5, 0.5, 1, 10)
};

// shared critter part geometries (one each, reused across every cat/person)
export const CAT = {
	body: new THREE.BoxGeometry(0.42, 0.34, 0.95),
	head: new THREE.SphereGeometry(0.26, 12, 10),
	ear: new THREE.ConeGeometry(0.09, 0.2, 4),
	tail: new THREE.CylinderGeometry(0.05, 0.05, 0.55, 8),
	leg: new THREE.BoxGeometry(0.11, 0.22, 0.11)
};
export const NPC = {
	torso: new THREE.CylinderGeometry(0.26, 0.26, 0.85, 12),
	head: new THREE.SphereGeometry(0.24, 12, 10),
	arm: new THREE.BoxGeometry(0.12, 0.62, 0.12),
	leg: new THREE.BoxGeometry(0.18, 0.7, 0.18)
};
