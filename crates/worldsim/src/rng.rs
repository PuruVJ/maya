//! Stateless squirrel-noise RNG — bit-exact port of `src/lib/rng.ts`. The same `(seed, keys)` always
//! yields the same value with no stream state, so the world is a pure function of (seed, clock-tick).
//! Core hash is Squirrel Eiserloh's noise function (GDC 2017); all lanes are 32-bit wrapping `u32`,
//! which reproduces the JS `Math.imul` / `| 0` / `>>> 0` arithmetic exactly.

const NOISE1: u32 = 0xb529_7a4d;
const NOISE2: u32 = 0x68e3_1da4;
const NOISE3: u32 = 0x1b56_c4e9;
const FNV_OFFSET: u32 = 0x811c_9dc5;
const FNV_PRIME: u32 = 0x0100_0193;

/// Hash a single 32-bit position with a seed → a well-mixed `u32`. The atom everything builds on.
#[inline]
pub fn hash(position: i32, seed: i32) -> u32 {
    let mut m = (position as u32).wrapping_mul(NOISE1);
    m = m.wrapping_add(seed as u32);
    m ^= m >> 8;
    m = m.wrapping_add(NOISE2);
    m ^= m << 8;
    m = m.wrapping_mul(NOISE3);
    m ^= m >> 8;
    m
}

/// Fold a list of integer keys into one `u32` (order-sensitive; the arity is folded in so a trailing
/// zero key can't alias a shorter coordinate).
pub fn hash_keys(seed: u32, keys: &[i32]) -> u32 {
    let mut h = seed;
    for &k in keys {
        h = hash(k, h as i32);
    }
    hash(keys.len() as i32, h as i32)
}

/// A float in `[0, 1)` at coordinate `(seed, keys)`.
#[inline]
pub fn rand(seed: u32, keys: &[i32]) -> f64 {
    hash_keys(seed, keys) as f64 / 4_294_967_296.0 // / 2^32
}

/// Map a string seed → a stable `u32` (FNV-1a over UTF-16 code units, matching JS `charCodeAt`).
pub fn seed_from(s: &str) -> u32 {
    let mut h = FNV_OFFSET;
    for u in s.encode_utf16() {
        h ^= u as u32;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    // Reference values produced by src/lib/rng.ts (the JS source of truth). These MUST match bit-for-bit:
    // it is exactly what guarantees a world evolves identically in JS, the WASM worker, and the CF server tick.
    #[test]
    fn hash_parity() {
        assert_eq!(hash(123, 0), 492_293_964);
        assert_eq!(hash(1, 1), 3_586_333_332);
        assert_eq!(hash(-1, 100), 1_076_820_394); // negative position exercises the u32 wrap
    }
    #[test]
    fn hash_keys_parity() {
        assert_eq!(hash_keys(42, &[1, 2, 3]), 2_609_341_440);
        assert_eq!(hash_keys(7, &[]), 3_589_541_006);
        assert_eq!(hash_keys(0, &[5, 5]), 3_162_176_880);
    }
    #[test]
    fn seed_from_parity() {
        assert_eq!(seed_from("worldgen-agents"), 4_204_040_608); // the live sim's base seed
        assert_eq!(seed_from("hello"), 1_335_831_723);
    }
    #[test]
    fn rand_matches_hash_keys() {
        assert!((0.0..1.0).contains(&rand(42, &[1, 2, 3])));
        assert_eq!(rand(42, &[1, 2, 3]), 2_609_341_440.0 / 4_294_967_296.0);
    }
}
