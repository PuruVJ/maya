import { describe, it, expect } from 'vitest';
import { SimClock, DT } from './clock';
import { Rng } from './rng';

describe('SimClock', () => {
	it('accumulates real dt into whole fixed ticks', () => {
		const c = new SimClock();
		expect(c.advance(DT * 2.5)).toBe(2); // 2 whole ticks, 0.5 carried
		expect(c.tick).toBe(2);
		expect(c.advance(DT * 0.5)).toBe(1); // carried 0.5 + 0.5 = 1 tick
		expect(c.tick).toBe(3);
		expect(c.time).toBeCloseTo(3 * DT, 10);
	});

	it('scales by rate and does nothing while paused', () => {
		const c = new SimClock();
		c.setRate(2);
		expect(c.advance(DT)).toBe(2); // double-time
		c.pause();
		expect(c.advance(DT * 10)).toBe(0);
		expect(c.tick).toBe(2);
		c.play();
		c.setRate(0); // rate 0 ≈ paused
		expect(c.advance(DT * 10)).toBe(0);
	});

	it('fires onTick once per tick with the right value, and step() emits manually', () => {
		const c = new SimClock();
		const seen: number[] = [];
		const off = c.onTick((t) => seen.push(t));
		c.advance(DT * 3);
		c.step(2);
		expect(seen).toEqual([1, 2, 3, 4, 5]);
		off();
		c.step(); // no longer listening
		expect(seen).toEqual([1, 2, 3, 4, 5]);
	});

	it('caps catch-up so a long stall cannot spiral', () => {
		const c = new SimClock();
		const n = c.advance(DT * 1000); // huge frame gap
		expect(n).toBeLessThanOrEqual(6);
	});

	it('seek jumps and fires onSeek(target, from); a no-op seek stays silent', () => {
		const c = new SimClock();
		c.step(10);
		const jumps: [number, number][] = [];
		c.onSeek((t, from) => jumps.push([t, from]));
		c.seek(3);
		expect(c.tick).toBe(3);
		c.seek(3); // same tick → no event
		expect(jumps).toEqual([[3, 10]]);
	});
});

describe('clock + rng = a world that is a pure function of (seed, tick)', () => {
	// model a tiny "world value" that, at each tick, depends only on (seed, tick) — the property the
	// real sim must preserve. Stepping, then time-travelling back and replaying, must reproduce it exactly.
	const rng = new Rng('world-seed');
	const valueAt = (tick: number) => rng.range(0, 100, tick, 0xabc);

	it('replays identically after seeking backward (time travel)', () => {
		const c = new SimClock();
		const forward: number[] = [];
		const offFwd = c.onTick((t) => forward.push(valueAt(t)));
		c.step(20); // live the world to tick 20
		offFwd(); // stop recording the original timeline before we replay over it

		// time-travel back to tick 5, then replay forward to 20 — must match the original timeline
		c.seek(5);
		const replay: number[] = [];
		const off = c.onTick((t) => replay.push(valueAt(t)));
		c.step(15); // 6..20
		off();

		expect(replay).toEqual(forward.slice(5)); // ticks 6..20 reproduce exactly
		expect(c.tick).toBe(20);
	});

	it('the same tick always yields the same value regardless of how you got there', () => {
		expect(valueAt(12345)).toBe(valueAt(12345));
		// jumping straight to a far tick gives the same value as arriving by stepping
		const c = new SimClock();
		c.seek(12345);
		expect(valueAt(c.tick)).toBe(valueAt(12345));
	});
});
