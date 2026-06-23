import { describe, it, expect } from 'vitest';
import { Vitals } from './vitals.svelte';

// Vitals is the TFR NUMERATOR: a rolling per-species birth tally. EcoStats turns ratePerSec() into a TFR estimate
// (rate ÷ fertile females × fertile window). These guard the windowing + per-species split + speed-independence.
describe('vitals — rolling per-species birth rate', () => {
	it('no births → empty rate map (→ TFR reads 0, not NaN)', () => {
		const v = new Vitals();
		expect(v.ratePerSec(100)).toEqual({});
	});

	it('rate = births ÷ observed span, split per species', () => {
		const v = new Vitals();
		v.birth('rabbit', 10);
		v.birth('rabbit', 25);
		v.birth('rabbit', 40); // 3 rabbits, oldest at t=10
		v.birth('person', 40); // 1 person
		const r = v.ratePerSec(40); // observed span = now - oldest = 30 s
		expect(r.rabbit).toBeCloseTo(3 / 30, 6);
		expect(r.person).toBeCloseTo(1 / 30, 6);
	});

	it('evicts births older than the 90 s window', () => {
		const v = new Vitals();
		v.birth('cat', 5); // will be > 90 s old at the query below → evicted
		v.birth('cat', 50);
		v.birth('cat', 80);
		const r = v.ratePerSec(100); // cut = 100-90 = 10 → the t=5 birth drops
		// remaining 2 births, oldest now t=50 → span = 50, rate = 2/50
		expect(r.cat).toBeCloseTo(2 / 50, 6);
	});

	it('a lone old birth decays toward 0 as its span grows, then evicts', () => {
		const v = new Vitals();
		v.birth('lion', 0);
		const early = v.ratePerSec(10).lion; // span 10 → 0.1/s
		const later = v.ratePerSec(60).lion; // span 60 → ~0.0167/s (decayed)
		expect(early).toBeGreaterThan(later);
		expect(v.ratePerSec(200).lion ?? 0).toBe(0); // past the window → fully evicted
	});

	it('is time-unit agnostic (feed sim-seconds → rate is per sim-second)', () => {
		// same birth COUNT over the same NUMBER of sim-seconds → same rate, regardless of wall-clock speed
		const v = new Vitals();
		for (let t = 1; t <= 10; t++) v.birth('kangaroo', t * 2); // 10 births across 18 sim-seconds (t=2..20)
		expect(v.ratePerSec(20).kangaroo).toBeCloseTo(10 / 18, 6);
	});
});
