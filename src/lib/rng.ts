// Stateless, seedable, hash-based RNG — "random that isn't random". The same (seed, ...keys) ALWAYS
// yields the same value, with NO stream state to advance: you can sample any coordinate directly, in any
// order, without having stepped through the ones before it. That property is the whole point — it lets
// the living world become a PURE FUNCTION of (seed, clock-tick): feed it a point on the clock (see
// clock.ts) and get a deterministic-but-erratic outcome you can reproduce, trace, and predict later. That
// is what makes time travel + shareable evolving worlds possible (see docs/self-sustaining-world.md).
//
// Core hash is Squirrel Eiserloh's noise function (GDC 2017, "Noise-Based RNG") — fast, well-distributed,
// reversible-free. All math is kept in 32-bit lanes via `Math.imul` / `| 0` / `>>> 0`.

const NOISE1 = 0xb5297a4d;
const NOISE2 = 0x68e31da4;
const NOISE3 = 0x1b56c4e9;
const U32 = 2 ** 32;

/** Hash a single 32-bit integer position with a seed → a well-mixed uint32. The atom everything builds on. */
export function hash(position: number, seed = 0): number {
	let m = Math.imul(position | 0, NOISE1);
	m = (m + (seed | 0)) | 0;
	m ^= m >>> 8;
	m = (m + NOISE2) | 0;
	m ^= m << 8;
	m = Math.imul(m, NOISE3);
	m ^= m >>> 8;
	return m >>> 0;
}

/** Fold an arbitrary list of integer keys into one uint32 (order-sensitive; [] ≠ [0] ≠ [0,0]). */
export function hashKeys(seed: number, keys: number[]): number {
	let h = seed | 0;
	for (let i = 0; i < keys.length; i++) h = hash(keys[i], h);
	return hash(keys.length, h); // fold the arity so a trailing zero key can't alias a shorter coord
}

/** A float in [0, 1) at coordinate (seed, ...keys). Keys are treated as 32-bit ints — quantise floats first. */
export function rand(seed: number, ...keys: number[]): number {
	return hashKeys(seed, keys) / U32;
}

/** Map a string (or number) seed → a stable uint32 (FNV-1a) so a world name / share token can seed everything. */
export function seedFrom(s: string | number): number {
	if (typeof s === 'number') return s >>> 0;
	let h = 0x811c9dc5;
	for (let i = 0; i < s.length; i++) {
		h ^= s.charCodeAt(i);
		h = Math.imul(h, 0x01000193);
	}
	return h >>> 0;
}

/** A reusable RNG bound to one seed — ergonomic for the sim/director, and consistent with SimClock's
 *  class style. Every method stays a PURE function of the keys you pass (so it's still fully addressable +
 *  time-travelable); the instance holds only the immutable seed — no mutable stream state. */
export class Rng {
	/** The resolved uint32 seed this RNG is bound to. */
	readonly seed: number;
	// two fixed sub-channels so a Gaussian draws two INDEPENDENT uniforms from the same coordinate
	static readonly #G1 = 0x9e3779b9 | 0;
	static readonly #G2 = 0x85ebca6b | 0;

	constructor(seedInput: string | number) {
		this.seed = seedFrom(seedInput);
	}

	/** [0, 1) at this coordinate. */
	rand(...keys: number[]): number {
		return rand(this.seed, ...keys);
	}
	/** [lo, hi) at this coordinate. */
	range(lo: number, hi: number, ...keys: number[]): number {
		return lo + (hi - lo) * this.rand(...keys);
	}
	/** integer in [loIncl, hiExcl) at this coordinate. */
	int(loIncl: number, hiExcl: number, ...keys: number[]): number {
		return loIncl + Math.floor(this.rand(...keys) * (hiExcl - loIncl));
	}
	/** true with probability p at this coordinate. */
	chance(p: number, ...keys: number[]): boolean {
		return this.rand(...keys) < p;
	}
	/** deterministic element of arr at this coordinate. */
	pick<T>(arr: readonly T[], ...keys: number[]): T {
		return arr[Math.floor(this.rand(...keys) * arr.length)];
	}
	/** A signed unit drift in (-1, 1), handy for "nudge a value a little" at this coordinate. */
	signed(...keys: number[]): number {
		return this.rand(...keys) * 2 - 1;
	}
	/** Gaussian (Box–Muller) with the given mean/std — for trait mutation drift, etc. */
	normal(mean: number, std: number, ...keys: number[]): number {
		const u1 = Math.max(1e-12, this.rand(...keys, Rng.#G1));
		const u2 = this.rand(...keys, Rng.#G2);
		return mean + std * Math.sqrt(-2 * Math.log(u1)) * Math.cos(2 * Math.PI * u2);
	}
	/** A STATEFUL sub-stream anchored at these keys (folds an auto-incrementing counter) — for "give me N
	 *  draws here" where you don't need to address each one. Still 100% deterministic + reproducible. */
	stream(...keys: number[]): () => number {
		let i = 0;
		return () => this.rand(...keys, i++);
	}
}
