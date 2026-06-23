// PARITY GUARD: the pond shoreline shape lives in TWO places on purpose — Rust (engine.rs water_edge_factor /
// water_seed, used by the sim's thirst/wade collision) and JS (water.ts, the render mesh + the player's per-frame
// wade check, which must run before/without the wasm). They MUST agree exactly, or you'd wade on dry-looking land
// (pinches) or stand in visible water without slowing (bulges). Rather than route the hot render/wade path through
// wasm, we keep the JS copy native and PIN it to Rust here: load the wasm math and assert the two implementations
// produce identical values across a sweep of seeds × angles (and ids). A typo in either copy fails this test.
import { describe, it, expect, beforeAll } from 'vitest';
import { math } from './math';
import { waterSeed, waterEdgeFactor } from './water';

const EPS = 1e-12; // both compute the same f64 sin() expression → expect bit-for-bit-ish agreement, not just close

describe('water shoreline parity (JS render copy ↔ Rust source of truth)', () => {
	beforeAll(async () => {
		await math.init();
	});

	it('loads the wasm math (otherwise this guard is vacuous)', () => {
		expect(math.ready).toBe(true);
	});

	it('waterSeed matches Rust for a spread of object ids', () => {
		// Zone ids are ASCII by construction (generated slugs like "pond", "lake-1"). Within ASCII, Rust's
		// per-BYTE hash (id.bytes()) and JS's per-code-unit hash (charCodeAt) coincide — the contract the Rust
		// comment calls out ("ASCII ids → byte == JS charCodeAt"). Non-ASCII would diverge (UTF-8 vs UTF-16), but
		// no real id is non-ASCII, so the guard covers exactly the inputs that occur.
		const ids = ['pond', 'lake-1', 'z42', 'A', '', 'a-very-long-pond-id-0007', 'pond_99', 'water~3'];
		for (const id of ids) {
			const rust = math.waterSeed(id);
			expect(rust, `wasm waterSeed(${id})`).not.toBeNull();
			expect(waterSeed(id), `seed parity for "${id}"`).toBeCloseTo(rust as number, 9);
		}
	});

	it('waterEdgeFactor matches Rust across seeds × angles', () => {
		for (let s = 0; s < 13; s++) {
			const seed = s * 0.013 * 11; // realistic seed magnitudes (waterSeed yields ~0..13)
			for (let a = 0; a < 24; a++) {
				const ang = (a / 24) * Math.PI * 2 - Math.PI; // full circle incl. negatives (atan2 range)
				const rust = math.waterEdgeFactor(seed, ang);
				expect(rust, `wasm edge(${seed},${ang})`).not.toBeNull();
				expect(Math.abs(waterEdgeFactor(seed, ang) - (rust as number)), `edge parity @ seed=${seed} ang=${ang}`).toBeLessThan(EPS);
			}
		}
	});

	it('the composed shoreline radius (seed-from-id → edge) agrees end to end', () => {
		for (const id of ['pond', 'lake-7', 'z3', 'shore_a']) {
			const jsSeed = waterSeed(id);
			const rustSeed = math.waterSeed(id) as number;
			for (let a = 0; a < 16; a++) {
				const ang = (a / 16) * Math.PI * 2;
				expect(Math.abs(waterEdgeFactor(jsSeed, ang) - (math.waterEdgeFactor(rustSeed, ang) as number)), `composed @ ${id} ang=${ang}`).toBeLessThan(EPS);
			}
		}
	});
});
