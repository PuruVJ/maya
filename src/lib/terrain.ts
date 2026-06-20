// Localized, deterministic terrain. heightAt = ambient wilderness relief + the user's contained
// features (hill/mountain/dune patches). The ambient layer is flat near spawn (buildable) and
// ramps into rolling hills + sparse mountains far out, so the distance feels vast — all from a
// cheap analytic function (the player-following mesh re-samples it; no stored geometry).
import type { TerrainFeature } from './world';

function smoothstep(a: number, b: number, x: number): number {
	const t = Math.max(0, Math.min(1, (x - a) / (b - a)));
	return t * t * (3 - 2 * t);
}

// ambient wilderness: 0 near spawn (buildable), then BIOME-VARIED relief far out so walking takes you
// through distinct regions — plains, rolling hills, raised plateaus, craggy mountain country. A slow
// `reg` field (km-scale) sets each region's mood; a second field raises broad tablelands. Purely analytic
// & deterministic. ⚠️ MIRRORED in Grass.svelte's GLSL `ambientH()` — keep the two IDENTICAL or grass floats.
function ambient(x: number, z: number): number {
	const ramp = smoothstep(70, 240, Math.hypot(x, z));
	if (ramp <= 0) return 0;
	// regional character (very low frequency → big regions you walk between)
	const reg = Math.sin(x * 0.0016 + 2.3) * Math.cos(z * 0.0014 - 1.1);
	const hilly = smoothstep(-0.35, 0.5, reg); // 0 plains → 1 hilly
	const ridged = smoothstep(0.45, 0.95, reg); // high regions → tall craggy peaks
	// rolling hills, scaled by how hilly this region is
	let h =
		(6 * Math.sin(x * 0.012 + 1.3) * Math.cos(z * 0.011 - 0.7) +
			3 * Math.sin(x * 0.03 - 2.1) * Math.cos(z * 0.028 + 1.1)) *
		(0.4 + hilly);
	// broad flat-topped plateaus where a second slow field is high
	const plat = Math.sin(x * 0.0021 - 0.6) * Math.cos(z * 0.0019 + 2.0);
	h += 13 * smoothstep(0.55, 0.82, plat);
	// sparse mountain peaks, taller in ridged regions
	const m = Math.sin(x * 0.008 + 4.2) * Math.cos(z * 0.0075 - 3.3);
	h += (18 + 24 * ridged) * Math.max(0, m - 0.5);
	return h * ramp;
}

// a contained feature's smooth radial bump (+ optional rolling ripple)
function featureHeight(x: number, z: number, f: TerrainFeature): number {
	const dx = x - f.center[0];
	const dz = z - f.center[1];
	const d = Math.hypot(dx, dz);
	if (d >= f.radius) return 0;
	const fall = 0.5 * (Math.cos((Math.PI * d) / f.radius) + 1); // 1 → 0
	let h = f.height * fall;
	if (f.rough) {
		h += f.rough * f.height * 0.2 * Math.sin(x * 0.45 + f.center[0]) * Math.cos(z * 0.45 + f.center[1]) * fall;
	}
	return h;
}

export function heightAt(x: number, z: number, features?: TerrainFeature[]): number {
	let h = ambient(x, z);
	if (features) for (const f of features) h += featureHeight(x, z, f);
	return h;
}
