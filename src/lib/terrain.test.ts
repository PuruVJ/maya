// PARITY GUARD: the ambient terrain height field lives in TWO places — Rust (engine.rs `height_at`, the sim's
// grounding/placement) and JS (terrain.ts `heightAt`, the render mesh + object grounding, run per-frame and before
// the wasm loads). They MUST agree or objects would sit at one height in the sim and another on screen (floating /
// sunken), and placement would disagree with the visible ground. The Rust crate has its own frozen-constant test,
// but that pins only the RUST side — a tweak to terrain.ts would pass it silently. This pins the LIVE JS copy to
// the LIVE Rust one across a sweep (ambient field, no features — the shared core). A drift on either side fails.
import { describe, it, expect, beforeAll } from 'vitest';
import { math } from './math';
import { heightAt } from './terrain';

const EPS = 1e-9; // both evaluate the same sin/cos/smoothstep field; f64 trig agrees to well under this

describe('ambient terrain parity (JS heightAt ↔ Rust height_at)', () => {
	beforeAll(async () => {
		await math.init();
	});

	it('loaded the wasm (otherwise the guard is vacuous)', () => {
		expect(math.ready).toBe(true);
	});

	it('heightAt matches Rust across a sweep — spawn-flat → far hills, incl. negatives + fractionals', () => {
		const coords = [-600, -300.5, -123.25, -40, 0, 40, 50, 100, 123.5, 220, 401, 777.75];
		let checked = 0;
		for (const x of coords) {
			for (const z of coords) {
				const rust = math.terrainHeight(x, z);
				expect(rust, `wasm terrainHeight(${x},${z})`).not.toBeNull();
				if (rust === null) continue;
				expect(Math.abs(heightAt(x, z) - rust), `height parity @ (${x}, ${z})`).toBeLessThan(EPS);
				checked++;
			}
		}
		expect(checked, 'sweep should cover the whole grid').toBe(coords.length * coords.length);
	});

	it('is flat (0) near spawn on both sides — the buildable apron', () => {
		for (const [x, z] of [[0, 0], [10, -10], [30, 20]] as [number, number][]) {
			expect(heightAt(x, z)).toBe(0);
			expect(math.terrainHeight(x, z)).toBe(0);
		}
	});
});
