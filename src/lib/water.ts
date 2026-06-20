// Pond geometry shared between the WATER SHADER (Water.svelte) and gameplay (Player wade/sink). The pond
// isn't a circle — its shoreline is carved by angular noise (an organic blob), so collision must use the
// SAME edge function as the shader or you'd slow on dry-looking land (pinches) and stand in visible water
// without slowing (bulges). Keep `waterEdgeFactor` identical to the GLSL in Water.svelte.
import type { Zone, TerrainFeature } from './world';
import { heightAt } from './terrain';

/**
 * World-Y of a pond's flat surface. The lakebed terrain varies (a grown lake can reach rising ground), so
 * the surface sits at the highest rim across the blob — CLAMPED so it covers a slope without towering over a
 * wading player — plus a small lip. SHARED by Water.svelte (mesh height) and LakeFish.svelte (so fish ride
 * exactly at the surface and never sink under the now-opaque water). Keep the two in lockstep via this.
 */
export function waterSurfaceY(zone: Zone, terrain: TerrainFeature[] | undefined): number {
	const centre = heightAt(zone.pos[0], zone.pos[2], terrain);
	let rim = centre;
	for (let i = 0; i < 12; i++) {
		const a = (i / 12) * Math.PI * 2;
		const r = zone.size * 0.85;
		rim = Math.max(rim, heightAt(zone.pos[0] + Math.cos(a) * r, zone.pos[2] + Math.sin(a) * r, terrain));
	}
	return Math.min(rim, centre + 0.4) + 0.08; // cover up to ~0.4 m of rim rise, no more (don't flood the player)
}

/** Per-pond seed from its id (matches Water.svelte's uSeed) → each shoreline wobbles differently. */
export function waterSeed(id: string): number {
	let s = 0;
	for (let i = 0; i < id.length; i++) s = (s * 31 + id.charCodeAt(i)) % 1000;
	return s * 0.013;
}

/** Shoreline radius as a fraction of `size` at this angle (≈0.80–1.03). Mirrors the GLSL `e`. */
export function waterEdgeFactor(seed: number, ang: number): number {
	return (
		0.8 +
		0.11 * Math.sin(ang * 3 + seed) +
		0.07 * Math.sin(ang * 5 - seed * 1.7) +
		0.045 * Math.sin(ang * 7 + seed * 2.3)
	);
}

/** Is (x, z) inside the (organic, blob-shaped) surface of any water zone? Matches what's drawn. */
export function inWater(zones: Zone[] | undefined, x: number, z: number): boolean {
	for (const zo of zones ?? []) {
		if (zo.material !== 'water') continue;
		const lx = x - zo.pos[0]; // local coords matching Water's vLocal (world x, and z flipped by the -90° tilt)
		const ly = zo.pos[2] - z;
		const r2 = lx * lx + ly * ly;
		if (r2 >= zo.size * zo.size) continue; // outside the max radius → definitely dry
		const edge = zo.size * waterEdgeFactor(waterSeed(zo.id), Math.atan2(ly, lx));
		if (r2 < edge * edge) return true;
	}
	return false;
}
