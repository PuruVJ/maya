import { describe, it, expect, beforeAll } from 'vitest';
import { hash, rand, hashKeys, seedFrom, Rng } from './rng';
import { math } from './math';

describe('hash-based RNG', () => {
	it('is a pure function — same coordinate, same value, every time', () => {
		expect(rand(123, 7)).toBe(rand(123, 7));
		expect(rand(123, 7, 8, 9)).toBe(rand(123, 7, 8, 9));
		expect(hash(42, 1)).toBe(hash(42, 1));
	});

	it('is STATELESS — re-sampling an old coordinate after newer ones gives the identical value', () => {
		const first = rand(99, 5);
		// sample a bunch of unrelated coordinates in between
		for (let t = 0; t < 1000; t++) rand(99, t * 31 + 1);
		expect(rand(99, 5)).toBe(first); // no stream state was advanced
	});

	it('decorrelates across seed, position, and key arity', () => {
		expect(rand(1, 0)).not.toBe(rand(2, 0)); // seed matters
		expect(rand(1, 0)).not.toBe(rand(1, 1)); // position matters
		expect(rand(1)).not.toBe(rand(1, 0)); // [] ≠ [0]
		expect(rand(1, 0)).not.toBe(rand(1, 0, 0)); // [0] ≠ [0,0]
		expect(hashKeys(1, [0, 0])).not.toBe(hashKeys(1, [0, 0, 0]));
	});

	it('outputs are always in [0, 1)', () => {
		for (let t = 0; t < 10000; t++) {
			const v = rand(7, t);
			expect(v).toBeGreaterThanOrEqual(0);
			expect(v).toBeLessThan(1);
		}
	});

	it('is roughly uniform (mean ≈ 0.5, full spread)', () => {
		let sum = 0;
		let min = 1;
		let max = 0;
		const N = 50000;
		for (let t = 0; t < N; t++) {
			const v = rand(0xc0ffee, t);
			sum += v;
			min = Math.min(min, v);
			max = Math.max(max, v);
		}
		expect(sum / N).toBeGreaterThan(0.49);
		expect(sum / N).toBeLessThan(0.51);
		expect(min).toBeLessThan(0.001);
		expect(max).toBeGreaterThan(0.999);
	});

	it('seedFrom is stable for a string and differs across strings/numbers', () => {
		expect(seedFrom('hello-world')).toBe(seedFrom('hello-world'));
		expect(seedFrom('hello-world')).not.toBe(seedFrom('hello-worle'));
		expect(seedFrom(42)).toBe(42);
		expect(seedFrom('')).toBeGreaterThanOrEqual(0);
	});
});

describe('Rng helpers', () => {
	const r = new Rng('seed-A');

	it('range / int / chance / pick stay in bounds and are reproducible', () => {
		for (let t = 0; t < 5000; t++) {
			const x = r.range(-3, 7, t);
			expect(x).toBeGreaterThanOrEqual(-3);
			expect(x).toBeLessThan(7);
			const n = r.int(0, 6, t); // a d6
			expect(Number.isInteger(n)).toBe(true);
			expect(n).toBeGreaterThanOrEqual(0);
			expect(n).toBeLessThan(6);
			expect(r.range(-3, 7, t)).toBe(x); // pure
		}
		const arr = ['a', 'b', 'c', 'd'] as const;
		for (let t = 0; t < 100; t++) expect(arr).toContain(r.pick(arr, t));
	});

	it('chance(p) fires at roughly rate p', () => {
		let hits = 0;
		const N = 20000;
		for (let t = 0; t < N; t++) if (r.chance(0.25, t)) hits++;
		expect(hits / N).toBeGreaterThan(0.23);
		expect(hits / N).toBeLessThan(0.27);
	});

	it('normal() has approximately the requested mean and std', () => {
		let sum = 0;
		let sumSq = 0;
		const N = 40000;
		for (let t = 0; t < N; t++) {
			const v = r.normal(5, 2, t);
			sum += v;
			sumSq += v * v;
		}
		const mean = sum / N;
		const std = Math.sqrt(sumSq / N - mean * mean);
		expect(mean).toBeGreaterThan(4.9);
		expect(mean).toBeLessThan(5.1);
		expect(std).toBeGreaterThan(1.9);
		expect(std).toBeLessThan(2.1);
	});

	it('two RNGs with the same seed agree; different seeds disagree', () => {
		const a = new Rng('same');
		const b = new Rng('same');
		const c = new Rng('different');
		expect(a.rand(1, 2, 3)).toBe(b.rand(1, 2, 3));
		expect(a.rand(1, 2, 3)).not.toBe(c.rand(1, 2, 3));
	});

	it('stream() is a deterministic sequence anchored at a coordinate', () => {
		const s1 = r.stream(10, 20);
		const a = [s1(), s1(), s1()];
		const s2 = r.stream(10, 20); // a fresh stream at the SAME anchor replays identically
		expect([s2(), s2(), s2()]).toEqual(a);
		// ...and matches the addressable form (counter folded as the last key)
		expect(a[0]).toBe(r.rand(10, 20, 0));
		expect(a[1]).toBe(r.rand(10, 20, 1));
	});
});

// PARITY GUARD vs Rust (rng.rs): the sim (Rust) and render-side seeding (JS) MUST draw identical values for the
// same (seed, keys), or the simulated world desyncs from the rendered/seeded one. The crate's own tests pin only
// the Rust side to frozen constants — a tweak to rng.ts above would pass them silently. This pins the live JS copy
// to the live Rust one over a sweep (Rust exposes rng_hash/rng_hash_keys/rng_rand/rng_seed_from via wasm).
describe('addressed RNG parity (JS rng.ts ↔ Rust rng.rs)', () => {
	beforeAll(async () => {
		await math.init();
	});

	it('loaded the wasm (otherwise the guard is vacuous)', () => {
		expect(math.ready).toBe(true);
	});

	it('hash(position, seed) matches Rust (incl. negatives + 32-bit extremes)', () => {
		const vals = [0, 1, -1, 7, 42, -97, 1000, 65535, 123456, -2147483648, 2147483647];
		for (const p of vals) for (const s of [0, 1, 42, -5, 999999]) {
			expect(hash(p, s), `hash(${p}, ${s})`).toBe(math.rngHash(p, s));
		}
	});

	it('hashKeys(seed, keys) matches Rust — arity-sensitive ([] ≠ [0] ≠ [0,0])', () => {
		const keysets: number[][] = [[], [0], [0, 0], [1], [1, 2, 3], [-4, 5, -6], [42, -97, 1000, 7]];
		for (const seed of [0, 7, 42, 4204040608 >>> 0]) for (const keys of keysets) {
			expect(hashKeys(seed >>> 0, keys), `hashKeys(${seed}, [${keys}])`).toBe(math.rngHashKeys(seed >>> 0, keys));
		}
	});

	it('rand(seed, ...keys) matches Rust exactly (integer hash / 2^32 is an exact f64)', () => {
		for (const seed of [1, 42, 7777]) for (const keys of [[1], [1, 2], [9, 8, 7]]) {
			const js = rand(seed, ...keys);
			const rs = math.rngRand(seed, keys);
			expect(rs, `rngRand(${seed}, [${keys}])`).not.toBeNull();
			expect(js, `rand parity (${seed}, [${keys}])`).toBe(rs);
		}
	});

	it('seedFrom(string) matches Rust for ASCII seeds (world names / share tokens)', () => {
		// ASCII only: JS charCodeAt (UTF-16) and Rust's UTF-16 code-unit fold coincide within ASCII — and every real
		// seed (a world name / share slug) is ASCII, matching rng.rs's documented contract.
		for (const s of ['', 'a', 'world', 'puruvj-demo', 'seed-12345', 'A Long World Name 0007', '~tilde_99']) {
			expect(seedFrom(s), `seedFrom("${s}")`).toBe(math.rngSeedFrom(s));
		}
	});
});
