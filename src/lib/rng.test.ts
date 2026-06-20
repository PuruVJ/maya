import { describe, it, expect } from 'vitest';
import { hash, rand, hashKeys, seedFrom, Rng } from './rng';

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
