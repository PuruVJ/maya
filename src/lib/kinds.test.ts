// PARITY GUARD: a kind's footprint (radius r, height h) lives in TWO places — Rust (engine.rs `kind_rh`, used by
// the sim's collision-free placement + grounding) and JS (the KINDS table here, which ALSO carries render geometry).
// They MUST agree, or an object would be PLACED with one footprint and DRAWN/collided with another (overlaps, or
// gaps the sim thinks are blocked). Rather than route the JS render table through wasm, we keep it native and pin it
// to Rust here: load the wasm and assert every kind's [r, h] matches, plus the unknown-kind fallback. A drift fails.
import { describe, it, expect, beforeAll } from 'vitest';
import { math } from './math';
import { KINDS, kindDef } from './kinds';

describe('kind footprint parity (JS KINDS ↔ Rust kind_rh)', () => {
	beforeAll(async () => {
		await math.init();
	});

	it('loaded the wasm (otherwise the guard is vacuous)', () => {
		expect(math.ready).toBe(true);
	});

	it('every kind in KINDS matches Rust kind_rh exactly', () => {
		const names = Object.keys(KINDS);
		expect(names.length, 'KINDS should be non-empty').toBeGreaterThan(10);
		for (const name of names) {
			const def = KINDS[name];
			const rust = math.kindRh(name);
			expect(rust, `wasm kind_rh(${name})`).not.toBeNull();
			if (!rust) continue;
			expect(def.r, `${name} radius`).toBe(rust[0]);
			expect(def.h, `${name} height`).toBe(rust[1]);
		}
	});

	it('an unknown kind falls back to the same [r, h] on both sides', () => {
		const rust = math.kindRh('totally-not-a-real-kind');
		expect(rust).not.toBeNull();
		if (!rust) return;
		const js = kindDef('totally-not-a-real-kind'); // KINDS[k] ?? FALLBACK
		expect(js.r, 'fallback radius').toBe(rust[0]);
		expect(js.h, 'fallback height').toBe(rust[1]);
	});
});
