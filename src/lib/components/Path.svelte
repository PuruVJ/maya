<script lang="ts">
	// A road/river stroke between two points. Built as a DRAPED strip — subdivided along its length with
	// each vertex grounded on heightAt — so it hugs the terrain instead of floating. ROADS (material 'path')
	// get a procedural asphalt SHADER (dark mottled tarmac, a dashed centre line, worn lighter shoulders);
	// rivers (water/ice) stay a plain translucent stroke. Grass is carved out beneath by the grass shader.
	// Decorative, no collider.
	import { untrack } from 'svelte';
	import { T } from '@threlte/core';
	import * as THREE from 'three';
	import { heightAt } from '$lib/terrain';
	import { ZONE_COLOR } from '$lib/kinds';
	import { wind } from '$lib/wind';
	import type { Path, World } from '$lib/world';

	let { path, world }: { path: Path; world: World } = $props();

	// each Path is keyed by id and never moves → snapshot once (same intentional pattern as Npc/Critter)
	const P = untrack(() => ({ material: path.material, width: path.width }));
	const isRoad = P.material === 'path';
	const translucent = P.material === 'water' || P.material === 'ice';
	const uWet = { value: 0 }; // 1 under the rainy 'fog' sky → wet asphalt (darker + sky sheen), driven below
	$effect(() => {
		uWet.value = world.sky === 'fog' ? 1 : 0;
	});

	// drape a strip over the terrain, with an `aRoad` attribute = (u across 0..1, v metres along) for the shader
	const geo = untrack(() => {
		const fx = path.from[0];
		const fz = path.from[2];
		const dx = path.to[0] - fx;
		const dz = path.to[2] - fz;
		const len = Math.hypot(dx, dz) || 0.001;
		const ux = dx / len;
		const uz = dz / len;
		const perpX = -uz;
		const perpZ = ux;
		const hw = path.width / 2;
		const N = Math.max(2, Math.ceil(len / 3));
		const pos: number[] = [];
		const road: number[] = [];
		const idx: number[] = [];
		for (let i = 0; i <= N; i++) {
			const t = i / N;
			const cx = fx + ux * len * t;
			const cz = fz + uz * len * t;
			const lx = cx + perpX * hw;
			const lz = cz + perpZ * hw;
			const rx = cx - perpX * hw;
			const rz = cz - perpZ * hw;
			pos.push(lx, heightAt(lx, lz, world.terrain) + 0.06, lz);
			pos.push(rx, heightAt(rx, rz, world.terrain) + 0.06, rz);
			road.push(0, len * t, 1, len * t); // (u, v) for left then right vertex
		}
		for (let i = 0; i < N; i++) {
			const a = i * 2;
			idx.push(a, a + 1, a + 2, a + 1, a + 3, a + 2);
		}
		const g = new THREE.BufferGeometry();
		g.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3));
		g.setAttribute('aRoad', new THREE.Float32BufferAttribute(road, 2));
		g.setIndex(idx);
		g.computeVertexNormals();
		return g;
	});

	// asphalt road shader (only for roads); rivers use the plain coloured stroke below
	const roadMat = untrack(() => {
		if (!isRoad) return null;
		const m = new THREE.MeshStandardMaterial({ color: 0xffffff, roughness: 0.92, metalness: 0 });
		m.onBeforeCompile = (shader) => {
			shader.uniforms.uWidth = { value: P.width };
			shader.uniforms.uWet = uWet;
			shader.vertexShader = shader.vertexShader
				.replace('#include <common>', '#include <common>\nattribute vec2 aRoad;\nvarying vec2 vRoad;\nvarying vec3 vWorldPos;')
				.replace('#include <begin_vertex>', '#include <begin_vertex>\nvRoad = aRoad;\nvWorldPos = (modelMatrix * vec4(position, 1.0)).xyz;');
			shader.fragmentShader = shader.fragmentShader
				.replace(
					'#include <common>',
					/* glsl */ `#include <common>
					varying vec2 vRoad;
					varying vec3 vWorldPos;
					uniform float uWidth;
					uniform float uWet;
					float rdHash(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
					float rdNoise(vec2 p){
						vec2 i = floor(p), f = fract(p);
						float a = rdHash(i), b = rdHash(i + vec2(1.0, 0.0)), c = rdHash(i + vec2(0.0, 1.0)), d = rdHash(i + vec2(1.0, 1.0));
						vec2 u = f * f * (3.0 - 2.0 * f);
						return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
					}`
				)
				.replace(
					'#include <color_fragment>',
					/* glsl */ `#include <color_fragment>
					float u = vRoad.x;          // 0..1 across the road
					float v = vRoad.y;          // metres along it
					float grit = rdNoise(vec2(u * 9.0, v * 0.8)) * 0.5 + rdNoise(vec2(u * 30.0, v * 3.0)) * 0.5;
					vec3 col = vec3(0.135, 0.135, 0.15) * (0.82 + 0.32 * grit);   // mottled tarmac
					float worn = 1.0 - smoothstep(0.0, 0.07, u) * smoothstep(1.0, 0.93, u);
					col += worn * 0.05;                                          // lighter, worn shoulders
					float lineW = clamp(0.18 / uWidth, 0.025, 0.12);
					float dash = step(0.5, fract(v / 3.0));                      // 1.5 m on, 1.5 m off
					float marking = step(abs(u - 0.5), lineW) * dash;
					col = mix(col, vec3(0.8, 0.74, 0.4) * (0.7 + 0.5 * grit), marking); // worn dashed centre line
					// RAIN: wet asphalt darkens (the classic dark, glossy wet street); the painted line keeps its
					// brightness (wet paint glistens rather than soaking dark).
					col *= mix(1.0, 0.58, uWet * (1.0 - marking));
					diffuseColor.rgb = col;`
				)
				.replace(
					'#include <emissivemap_fragment>',
					/* glsl */ `#include <emissivemap_fragment>
					if (uWet > 0.01) {
						// wet sheen: the road mirrors the overcast sky at grazing angles → a glistening surface, added
						// as EMISSIVE so it survives the weak rain sun (matches the terrain's wet sheen).
						vec3 rN = normalize(cross(dFdx(vWorldPos), dFdy(vWorldPos)));
						vec3 rV = normalize(cameraPosition - vWorldPos);
						float rFres = pow(1.0 - clamp(dot(rN, rV), 0.0, 1.0), 4.0);
						totalEmissiveRadiance += uWet * rFres * vec3(0.34, 0.38, 0.45) * 0.55;
					}`
				);
		};
		return m;
	});

	// flowing-water shader for RIVER paths (material 'water') — was a flat translucent stroke. Ripples scroll
	// DOWNSTREAM (along +v) at two speeds for a living current; mid-channel reads deep, banks shallow + foamy,
	// and the brightest moving crests glint. Shares the global wind clock (wind.uTime) so it animates without
	// its own useTask. 'ice'/other materials keep the plain stroke below.
	const riverMat = untrack(() => {
		if (P.material !== 'water') return null;
		const m = new THREE.MeshStandardMaterial({ color: 0xffffff, roughness: 0.32, metalness: 0, transparent: true, opacity: 0.86, depthWrite: false });
		m.onBeforeCompile = (shader) => {
			shader.uniforms.uTime = wind.uTime;
			shader.vertexShader = shader.vertexShader
				.replace('#include <common>', '#include <common>\nattribute vec2 aRoad;\nvarying vec2 vRoad;')
				.replace('#include <begin_vertex>', '#include <begin_vertex>\nvRoad = aRoad;');
			shader.fragmentShader = shader.fragmentShader
				.replace(
					'#include <common>',
					/* glsl */ `#include <common>
					varying vec2 vRoad;
					uniform float uTime;
					float rvHash(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
					float rvNoise(vec2 p){
						vec2 i = floor(p), f = fract(p);
						float a = rvHash(i), b = rvHash(i + vec2(1.0, 0.0)), c = rvHash(i + vec2(0.0, 1.0)), d = rvHash(i + vec2(1.0, 1.0));
						vec2 u = f * f * (3.0 - 2.0 * f);
						return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
					}`
				)
				.replace(
					'#include <color_fragment>',
					/* glsl */ `#include <color_fragment>
					float u = vRoad.x;          // 0..1 across the channel
					float v = vRoad.y;          // metres downstream
					float flow = rvNoise(vec2(u * 4.0, v * 0.5 - uTime * 1.1)) * 0.6
					           + rvNoise(vec2(u * 11.0, v * 1.4 - uTime * 1.9)) * 0.4; // current scrolling downstream
					float edge = abs(u - 0.5) * 2.0;                          // 0 mid-channel → 1 at the banks
					vec3 col = mix(vec3(0.09, 0.25, 0.30), vec3(0.22, 0.45, 0.46), clamp(edge * 0.65 + flow * 0.22, 0.0, 1.0));
					float foam = smoothstep(0.86, 0.99, edge) + smoothstep(0.80, 0.98, flow) * 0.5; // banks + crest flecks
					col = mix(col, vec3(0.86, 0.93, 0.93), clamp(foam, 0.0, 0.65));
					diffuseColor.rgb = col;`
				)
				.replace(
					'#include <emissivemap_fragment>',
					/* glsl */ `#include <emissivemap_fragment>
					// moving crests catch the light → a travelling glint, so the current reads wet + alive
					totalEmissiveRadiance += smoothstep(0.72, 1.0, flow) * vec3(0.28, 0.40, 0.42) * 0.3;`
				);
		};
		return m;
	});
</script>

{#if roadMat}
	<T.Mesh geometry={geo} material={roadMat} receiveShadow />
{:else if riverMat}
	<T.Mesh geometry={geo} material={riverMat} receiveShadow />
{:else}
	<T.Mesh geometry={geo} receiveShadow>
		<T.MeshStandardMaterial color={ZONE_COLOR[P.material] ?? '#9a9a9a'} opacity={translucent ? 0.8 : 1} transparent={translucent} depthWrite={!translucent} />
	</T.Mesh>
{/if}
