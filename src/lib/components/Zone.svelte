<script lang="ts">
	// A flat ground-decal zone (plaza / sand / flowers / ice / lava / grass), given a PROCEDURAL surface
	// instead of a flat colour — flagstone paving, wind ripples, a flower bed, frost cracks, glowing animated
	// lava fissures — all shaders, no textures ([[shader-first-direction]]). MeshStandardMaterial patched via
	// onBeforeCompile so scene lighting + shadows still apply. World-space pattern (parallax-correct). Water
	// is handled by Water.svelte; this covers every other zone material. Decorative, no collider.
	import { untrack } from 'svelte';
	import { T } from '@threlte/core';
	import * as THREE from 'three';
	import { ZONE_COLOR, ZONE_TRANSLUCENT } from '$lib/kinds';
	import { wind } from '$lib/wind';
	import type { Zone } from '$lib/world';

	let { zone }: { zone: Zone } = $props();
	// zones are keyed by id and never change material/shape/size after creation → snapshot once
	const Z = untrack(() => ({ material: zone.material, shape: zone.shape, size: zone.size }));
	const color = ZONE_COLOR[Z.material] ?? '#888888';
	const translucent = ZONE_TRANSLUCENT.has(Z.material);
	const EMISSIVE = Z.material === 'lava';

	// shared GLSL helpers (hash / value-noise / fbm / voronoi-with-cell-id) injected into the fragment shader
	const NOISE = /* glsl */ `
		varying vec2 vZW;
		uniform float uTime;
		float zh(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
		float zn(vec2 p){
			vec2 i = floor(p), f = fract(p);
			float a = zh(i), b = zh(i + vec2(1.0, 0.0)), c = zh(i + vec2(0.0, 1.0)), d = zh(i + vec2(1.0, 1.0));
			vec2 u = f * f * (3.0 - 2.0 * f);
			return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
		}
		float zfbm(vec2 p){ float v = 0.0, a = 0.5; for (int i = 0; i < 4; i++) { v += a * zn(p); p *= 2.03; a *= 0.5; } return v; }
		// returns vec3(F1, F2, cellRandom) — F2-F1 ≈ 0 marks a cell BORDER (grout/crack), cellRandom tints each cell
		vec3 zvor(vec2 p){
			vec2 ip = floor(p), fp = fract(p);
			float f1 = 9.0, f2 = 9.0; vec2 id = vec2(0.0);
			for (int y = -1; y <= 1; y++) for (int x = -1; x <= 1; x++) {
				vec2 g = vec2(float(x), float(y));
				vec2 o = vec2(zh(ip + g), zh(ip + g + 31.7));
				float d = length(g + o - fp);
				if (d < f1) { f2 = f1; f1 = d; id = ip + g; } else if (d < f2) f2 = d;
			}
			return vec3(f1, f2, zh(id));
		}
	`;

	// per-material surface (writes diffuseColor.rgb; lava also sets `lavaGlow` for the emissive pass)
	const SNIPPET: Record<string, string> = {
		plaza: /* glsl */ `
			vec3 v = zvor(vZW * 0.55);
			float grout = smoothstep(0.07, 0.0, v.y - v.x);                 // dark joints between flagstones
			vec3 stone = diffuseColor.rgb * (0.80 + 0.36 * v.z);            // each stone a slightly different tone
			stone *= 0.93 + 0.12 * zn(vZW * 8.0);                           // fine grain
			diffuseColor.rgb = mix(stone, stone * 0.42, grout);`,
		sand: /* glsl */ `
			float rip = sin(vZW.x * 2.6 + sin(vZW.y * 0.6) * 1.8) * 0.5 + 0.5; // wind ripples
			diffuseColor.rgb *= 0.88 + 0.16 * rip;
			diffuseColor.rgb *= 0.94 + 0.10 * zn(vZW * 22.0);`,             // sand grain
		flowers: /* glsl */ `
			vec3 bed = vec3(0.30, 0.48, 0.21) * (0.82 + 0.34 * zn(vZW * 1.6)); // grassy bed (not the flat pink)
			vec3 v = zvor(vZW * 2.4);
			float bloom = smoothstep(0.34, 0.12, v.x);                      // a blossom near each cell centre
			vec3 fcol = v.z < 0.33 ? vec3(0.96, 0.86, 0.32) : v.z < 0.66 ? vec3(0.92, 0.42, 0.62) : vec3(0.86, 0.84, 0.96);
			diffuseColor.rgb = mix(bed, fcol, bloom * 0.92);`,
		ice: /* glsl */ `
			vec3 v = zvor(vZW * 0.8);
			float crack = smoothstep(0.05, 0.0, v.y - v.x);
			diffuseColor.rgb *= 0.92 + 0.16 * zn(vZW * 3.0);
			diffuseColor.rgb = mix(diffuseColor.rgb, vec3(0.82, 0.93, 1.0), crack * 0.5); // bright frost veins
			diffuseColor.rgb += pow(zn(vZW * 30.0 + floor(uTime * 3.0)), 18.0) * 0.6;`,   // twinkle
		lava: /* glsl */ `
			vec3 v = zvor(vZW * 0.5 + zfbm(vZW * 0.5) * 0.3);
			float fis = smoothstep(0.10, 0.0, v.y - v.x);                   // fissure network
			float lavaGlow = fis * (0.6 + 0.4 * sin(uTime * 1.5 + v.z * 20.0)); // pulses; reused by the emissive pass
			vec3 crust = vec3(0.12, 0.06, 0.05) * (0.7 + 0.5 * zn(vZW * 3.0));
			diffuseColor.rgb = mix(crust, vec3(1.0, 0.45, 0.08), lavaGlow);`,
		grass: /* glsl */ `
			diffuseColor.rgb *= 0.85 + 0.30 * zfbm(vZW * 0.5);
			diffuseColor.rgb = mix(diffuseColor.rgb, diffuseColor.rgb * vec3(0.8, 1.12, 0.7), 0.30 * zn(vZW * 1.2));`
	};

	function build(): THREE.MeshStandardMaterial {
		const m = new THREE.MeshStandardMaterial({
			color,
			transparent: translucent,
			opacity: translucent ? 0.82 : 1,
			depthWrite: !translucent,
			roughness: 0.92,
			metalness: 0
		});
		const body = SNIPPET[Z.material];
		if (!body) return m; // unknown material → plain decal (back-compat)
		m.onBeforeCompile = (shader) => {
			shader.uniforms.uTime = wind.uTime; // shared clock (ticked once by Scene) → lava/ice animate
			shader.vertexShader = shader.vertexShader
				.replace('#include <common>', '#include <common>\nvarying vec2 vZW;')
				.replace('#include <begin_vertex>', '#include <begin_vertex>\nvZW = (modelMatrix * vec4(position, 1.0)).xz;');
			shader.fragmentShader = shader.fragmentShader
				.replace('#include <common>', '#include <common>\n' + NOISE)
				.replace('#include <color_fragment>', '#include <color_fragment>\n' + body);
			// lava: the color snippet declares `lavaGlow`; the emissive pass (later in main) re-uses it to glow
			if (EMISSIVE) {
				shader.fragmentShader = shader.fragmentShader.replace(
					'#include <emissivemap_fragment>',
					'#include <emissivemap_fragment>\ntotalEmissiveRadiance += lavaGlow * vec3(1.0, 0.35, 0.05) * 2.2;'
				);
			}
		};
		return m;
	}
	const mat = build();
</script>

<!-- a flat ground decal; sits just above the terrain at its centre. Decorative, no collider. -->
<T.Group position={[zone.pos[0], zone.pos[1] + 0.04, zone.pos[2]]} rotation.x={-Math.PI / 2}>
	<T.Mesh material={mat}>
		{#if Z.shape === 'rect'}
			<T.PlaneGeometry args={[Z.size * 2, Z.size * 2]} />
		{:else if Z.shape === 'ring'}
			<T.RingGeometry args={[Z.size * 0.55, Z.size, 48]} />
		{:else}
			<T.CircleGeometry args={[Z.size, 48]} />
		{/if}
	</T.Mesh>
</T.Group>
