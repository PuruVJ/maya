// Shared WEATHER — uniforms Scene sets from the sky/ground, read by every wettable/snowable material so the
// WHOLE world responds to the weather together, the same way `wind.uTime` is the one shared foliage clock.
//   uWet  : 1 under the rainy 'fog' sky    → surfaces darken + pick up a grazing sheen
//   uSnow : 1 when the ground is 'snow'     → up-facing surfaces get a snow cap
// Terrain, roads (Path) and Building wet/snow themselves directly; this lets the shared materials opt in with a
// single applyWeather() call. See [[shader-first-direction]].
import type { IUniform } from 'three';

export const weather = { uWet: { value: 0 }, uSnow: { value: 0 }, uFlash: { value: 0 } };
// uFlash: transient 0→~1.5 lightning pulse (Scene's rain storm) → the cloud deck flashes white, so the
// strike has a SOURCE in the sky, not just a ground-level ambient lift. 0 except during a fog-sky strike.

/**
 * Add rain + snow response to a MeshStandardMaterial's patched shader — call INSIDE onBeforeCompile, AFTER your
 * own patches (it appends to the kept chunk tokens). RAIN darkens the surface + adds a grazing wet SHEEN as
 * emissive (so it survives the weak rain sun — the trick Terrain/Path/Building use). SNOW mixes white onto
 * UP-facing surfaces (world normal.y), applied at <alphamap_fragment> so it lands AFTER the material's own
 * colour (sits on top, not under the texture). Injects its own world-up varying, so it needs no varyings from
 * the material; the wet sheen uses the built-in view `normal` + `vViewPosition`.
 */
export function applyWeather(
	shader: { uniforms: Record<string, IUniform>; fragmentShader: string; vertexShader: string },
	darken = 0.26,
	sheen = 0.5
): void {
	shader.uniforms.uWet = weather.uWet;
	shader.uniforms.uSnow = weather.uSnow;
	shader.vertexShader = shader.vertexShader
		.replace('#include <common>', '#include <common>\nvarying float vWUp;')
		.replace('#include <begin_vertex>', '#include <begin_vertex>\nvWUp = normalize(mat3(modelMatrix) * normal).y;');
	shader.fragmentShader = shader.fragmentShader
		.replace('#include <common>', '#include <common>\nuniform float uWet;\nuniform float uSnow;\nvarying float vWUp;')
		.replace(
			'#include <color_fragment>',
			`#include <color_fragment>\n\t\t\t\tdiffuseColor.rgb *= 1.0 - ${darken.toFixed(2)} * uWet; // rain-darkened`
		)
		.replace(
			'#include <alphamap_fragment>',
			/* glsl */ `if (uSnow > 0.01) {
				// snow settling on UP-facing surfaces (world normal.y) — applied here, after the material's own
				// colour, so it sits ON TOP of the texture rather than getting multiplied away beneath it.
				diffuseColor.rgb = mix(diffuseColor.rgb, vec3(0.93, 0.95, 0.99), uSnow * smoothstep(0.42, 0.74, vWUp));
			}
			#include <alphamap_fragment>`
		)
		.replace(
			'#include <emissivemap_fragment>',
			/* glsl */ `#include <emissivemap_fragment>
			if (uWet > 0.01) {
				float wetFres = pow(1.0 - clamp(dot(normalize(normal), normalize(vViewPosition)), 0.0, 1.0), 4.0);
				totalEmissiveRadiance += uWet * wetFres * vec3(0.34, 0.38, 0.45) * ${sheen.toFixed(2)};
			}`
		);
}
