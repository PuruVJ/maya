// Shared wind clock for all foliage shaders (grass uses its own; ambient forest + placed trees share
// this). A single uniform object, advanced ONCE per frame by Scene.svelte — components only READ it, so
// the sway stays in sync and we never double-count dt. Plain object (hot path), not reactive state.
export const wind = { uTime: { value: 0 } };

// A GLOBAL GUST factor (a GLSL expression in `uTime`) — slow swells that make the whole landscape surge
// together when a gust rolls through, then calm. Centred on 1.0 (≈0.35 calm … ≈1.65 gust) so it MODULATES
// existing sway amplitudes without changing their average. Injected verbatim wherever `uTime` is in scope
// (grass, foliage sway, chimney smoke); since every system reads the same shared clock with the same
// coefficients, their gusts stay in phase with no extra uniform. See [[shader-first-direction]].
export const WIND_GUST = '(1.0 + 0.4 * sin(uTime * 0.23) + 0.25 * sin(uTime * 0.07 + 1.7))';

// hash/noise/fbm for dappled foliage — injected into tree fragment shaders.
export const WIND_NOISE = /* glsl */ `
	float windHash(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
	float windNoise(vec2 p){
		vec2 i = floor(p), f = fract(p);
		float a = windHash(i), b = windHash(i + vec2(1.0, 0.0)), c = windHash(i + vec2(0.0, 1.0)), d = windHash(i + vec2(1.0, 1.0));
		vec2 u = f * f * (3.0 - 2.0 * f);
		return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
	}
	float windFbm(vec2 p){ float v = 0.0, a = 0.5; for (int i = 0; i < 3; i++) { v += a * windNoise(p); p *= 2.03; a *= 0.5; } return v; }
`;

// shared vertex-shader sway: bends a mesh by local height, phased per-object from its world base (read
// from modelMatrix), so trees lean in a rolling breeze rather than in lockstep. `transformed` must exist.
export function swayVertex(amount = 0.05): string {
	return /* glsl */ `
		vLocalPos = position;
		vec2 wb = vec2(modelMatrix[3][0], modelMatrix[3][2]);
		float wph = uTime * 0.9 + wb.x * 0.18 + wb.y * 0.15;
		float wsway = sin(wph) + 0.4 * sin(wph * 2.1 + 1.3);
		float wgust = ${WIND_GUST};                                  // whole canopy leans harder as a gust passes
		float wlever = max(position.y, 0.0) * ${amount.toFixed(3)} * wgust;
		transformed.x += wsway * wlever;
		transformed.z += cos(wph * 0.8) * wlever * 0.6;
	`;
}

