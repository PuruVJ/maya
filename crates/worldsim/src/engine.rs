//! The deterministic world ENGINE's PURE MATH — the kind r/h table, `height_at` (terrain), and the `in_water`
//! shoreline test — ported from the old JS `terrain.ts` / `water.ts` / `kinds`. Rust owns ALL compute; JS keeps only
//! render. These are the shared, dependency-free primitives the op engine + the world-gen build on; parity with the
//! JS originals is locked by the tests below.
//!
//! The op→world layer that used to live here (`apply_ops` over a jzon DOM) was replaced by the BINARY engine in
//! `engine_bin.rs` (typed structs, no JSON) — the jzon drop. This module no longer depends on jzon.

use std::f64::consts::PI;

// ───────────────────────── kind footprint table (engine needs only r + h) ─────────────────────────
// Mirrors the r/h fields of src/lib/kinds.ts `KINDS`; the rest of a KindDef (render geometry) stays in JS.
// Unknown kind → FALLBACK (r=1, h=2), matching `kindDef(k) ?? FALLBACK`.
pub fn kind_rh(kind: &str) -> (f64, f64) {
    match kind {
        "tree" => (0.8, 3.0),
        "pine" => (0.8, 4.0),
        "bush" => (0.6, 1.0),
        "flower" => (0.3, 0.6),
        "rock" => (0.9, 1.0),
        "grave" => (0.5, 1.0),
        "house" => (3.0, 3.0),
        "cabin" => (2.5, 3.0),
        "tower" => (1.8, 8.0),
        "well" => (1.2, 1.5),
        "lamp" => (0.4, 3.0),
        "fence" => (0.6, 1.0),
        "bridge" => (2.0, 0.6),
        "person" => (0.5, 1.8),
        "cat" => (0.6, 0.7),
        "lion" => (0.85, 0.95),
        "rabbit" => (0.4, 0.55),
        "kangaroo" => (0.6, 1.4),
        "dinosaur" => (1.4, 2.6),
        _ => (1.0, 2.0), // FALLBACK
    }
}
#[inline]
pub fn kind_r(kind: &str) -> f64 {
    kind_rh(kind).0
}
#[inline]
pub fn kind_h(kind: &str) -> f64 {
    kind_rh(kind).1
}

// ───────────────────────── terrain (mirror of src/lib/terrain.ts) ─────────────────────────
/// A contained terrain feature (hill/mountain/dune patch) — center (x,z), radius, peak height, roughness.
#[derive(Clone, Debug, PartialEq)]
pub struct Feature {
    pub center: [f64; 2],
    pub radius: f64,
    pub height: f64,
    pub rough: f64,
}

fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ⚠️ keep IDENTICAL to terrain.ts `ambient()` (and Grass.svelte GLSL `ambientH`) or the grass floats off the ground
fn ambient(x: f64, z: f64) -> f64 {
    let ramp = smoothstep(70.0, 240.0, x.hypot(z));
    if ramp <= 0.0 {
        return 0.0;
    }
    let reg = (x * 0.0016 + 2.3).sin() * (z * 0.0014 - 1.1).cos();
    let hilly = smoothstep(-0.35, 0.5, reg);
    let ridged = smoothstep(0.45, 0.95, reg);
    let mut h = (6.0 * (x * 0.012 + 1.3).sin() * (z * 0.011 - 0.7).cos()
        + 3.0 * (x * 0.03 - 2.1).sin() * (z * 0.028 + 1.1).cos())
        * (0.4 + hilly);
    let plat = (x * 0.0021 - 0.6).sin() * (z * 0.0019 + 2.0).cos();
    h += 13.0 * smoothstep(0.55, 0.82, plat);
    let m = (x * 0.008 + 4.2).sin() * (z * 0.0075 - 3.3).cos();
    h += (18.0 + 24.0 * ridged) * (m - 0.5).max(0.0);
    h * ramp
}

fn feature_height(x: f64, z: f64, f: &Feature) -> f64 {
    let dx = x - f.center[0];
    let dz = z - f.center[1];
    let d = dx.hypot(dz);
    if d >= f.radius {
        return 0.0;
    }
    let fall = 0.5 * ((PI * d / f.radius).cos() + 1.0); // 1 → 0
    let mut h = f.height * fall;
    if f.rough != 0.0 {
        h += f.rough * f.height * 0.2 * (x * 0.45 + f.center[0]).sin() * (z * 0.45 + f.center[1]).cos() * fall;
    }
    h
}

/// World-Y of the ground at (x,z): ambient wilderness relief + the contained features. (terrain.ts `heightAt`.)
pub fn height_at(x: f64, z: f64, features: &[Feature]) -> f64 {
    let mut h = ambient(x, z);
    for f in features {
        h += feature_height(x, z, f);
    }
    h
}

// ───────────────────────── water (mirror of src/lib/water.ts) ─────────────────────────
/// Just the water-zone fields `in_water` needs (the renderer owns the rest of a Zone).
pub struct WZone {
    pub id: String,
    pub material: String,
    pub pos: [f64; 3],
    pub size: f64,
}

// `pub` so lib.rs can re-export these to JS as the SINGLE source of truth for the pond shoreline — the render
// (water.ts) keeps a native copy for the player's per-frame wade check, and a vitest parity test pins it to these.
pub fn water_seed(id: &str) -> f64 {
    let mut s: i64 = 0;
    for b in id.bytes() {
        s = (s * 31 + b as i64) % 1000; // ASCII ids → byte == JS charCodeAt
    }
    s as f64 * 0.013
}

pub fn water_edge_factor(seed: f64, ang: f64) -> f64 {
    0.8 + 0.11 * (ang * 3.0 + seed).sin() + 0.07 * (ang * 5.0 - seed * 1.7).sin() + 0.045 * (ang * 7.0 + seed * 2.3).sin()
}

/// Is (x,z) inside the organic, blob-shaped surface of any water zone? (water.ts `inWater` — must match the shader.)
pub fn in_water(zones: &[WZone], x: f64, z: f64) -> bool {
    for zo in zones {
        if zo.material != "water" {
            continue;
        }
        let lx = x - zo.pos[0];
        let ly = zo.pos[2] - z; // z flipped by the -90° tilt, matching Water's vLocal
        let r2 = lx * lx + ly * ly;
        if r2 >= zo.size * zo.size {
            continue;
        }
        let edge = zo.size * water_edge_factor(water_seed(&zo.id), ly.atan2(lx));
        if r2 < edge * edge {
            return true;
        }
    }
    false
}

/// Like `in_water`, but each zone carries a PRE-COMPUTED water-seed instead of an id string — for the BINARY
/// worldgen ABI (the StructureStore path, docs/world-data-architecture.md), where passing id strings across the
/// wasm boundary is awkward. Each zone = (px, pz, size, seed) where seed = `water_seed(id)` (computed JS-side).
/// Byte-identical to `in_water` for the same (size, seed).
pub fn in_water_seeded(zones: &[(f64, f64, f64, f64)], x: f64, z: f64) -> bool {
    for &(px, pz, size, seed) in zones {
        let lx = x - px;
        let ly = pz - z;
        let r2 = lx * lx + ly * ly;
        if r2 >= size * size {
            continue;
        }
        let edge = size * water_edge_factor(seed, ly.atan2(lx));
        if r2 < edge * edge {
            return true;
        }
    }
    false
}

// ───────────────────────── NATURAL PONDS (Rust is the source of truth) ─────────────────────────
// A deterministic, INFINITE field of natural ponds laid on a jittered grid — so water is spread EVENLY across the
// whole world (animals settle by their local pond instead of all dragging to one shore). Rust OWNS this (the sim
// reads it for thirst; the renderer reads `ponds_near` to draw them). Static → the frontend reads it once per area,
// so it crosses the wasm boundary almost for free. Pure functions of position → identical every run, no state.
const POND_CELL: f64 = 300.0; // metres between natural ponds (grid spacing)
const POND_PROB: f64 = 0.82; // fraction of cells that actually hold a pond (natural gaps, not a rigid lattice)
const POND_R_MIN: f64 = 11.0;
const POND_R_MAX: f64 = 20.0;
const POND_SPAWN_CLEAR: f64 = 120.0; // keep procedural ponds OUT of the curated spawn/home area (like SCATTER_CLEAR
// for trees) — cell (0,0) always rolls a pond, which otherwise lands a random lake right on top of the demo's home.

/// deterministic [0,1) hash of a grid cell + salt (no state, no transcendentals → identical across worker/main).
fn pond_hash(cx: i32, cz: i32, salt: i32) -> f64 {
    let mut h = (cx as i64)
        .wrapping_mul(73_856_093)
        ^ (cz as i64).wrapping_mul(19_349_663)
        ^ (salt as i64).wrapping_mul(83_492_791);
    h = h.wrapping_mul(-7_046_029_254_386_353_131); // 0x9E3779B97F4A7C15 as i64 (golden-ratio mix)
    ((h >> 33) & 0xFF_FFFF) as f64 / 16_777_216.0
}

/// The natural pond in grid cell (cx,cz), or None if this cell has none. Returns (centre_x, centre_z, radius).
pub fn natural_pond_in_cell(cx: i32, cz: i32) -> Option<(f64, f64, f64)> {
    if pond_hash(cx, cz, 0) >= POND_PROB {
        return None;
    }
    let jx = (pond_hash(cx, cz, 1) - 0.5) * POND_CELL * 0.6; // jitter off the lattice so it reads natural
    let jz = (pond_hash(cx, cz, 2) - 0.5) * POND_CELL * 0.6;
    let x = cx as f64 * POND_CELL + jx;
    let z = cz as f64 * POND_CELL + jz;
    if x * x + z * z < POND_SPAWN_CLEAR * POND_SPAWN_CLEAR {
        return None; // no procedural pond in the curated home area (the demo's own lake lives here instead)
    }
    let r = POND_R_MIN + pond_hash(cx, cz, 3) * (POND_R_MAX - POND_R_MIN);
    Some((x, z, r))
}

/// Is (x,z) inside (or right at the lip of) a natural pond? Keeps ambient scatter (trees/bushes) OUT of the water —
/// they were spawning in ponds. Checks the 3×3 pond cells around the point (a pond jitters within its own cell).
pub fn in_natural_pond(x: f64, z: f64) -> bool {
    let cx = (x / POND_CELL).floor() as i32;
    let cz = (z / POND_CELL).floor() as i32;
    for ci in (cx - 1)..=(cx + 1) {
        for cj in (cz - 1)..=(cz + 1) {
            if let Some((px, pz, r)) = natural_pond_in_cell(ci, cj) {
                if (x - px).hypot(z - pz) < r + 1.5 {
                    return true; // inside + a small shoreline margin so trunks aren't standing at the waterline
                }
            }
        }
    }
    false
}

/// All natural ponds whose surface comes within `reach` of (px,pz) — for the RENDERER (draw the nearby water) and
/// for seeding fish/etc. Flat-friendly tuple list; the caller maps it to whatever it needs.
pub fn ponds_near(px: f64, pz: f64, reach: f64) -> Vec<(f64, f64, f64)> {
    let c0 = ((px - reach) / POND_CELL).floor() as i32;
    let c1 = ((px + reach) / POND_CELL).floor() as i32;
    let d0 = ((pz - reach) / POND_CELL).floor() as i32;
    let d1 = ((pz + reach) / POND_CELL).floor() as i32;
    let mut out = Vec::new();
    for cx in c0..=c1 {
        for cz in d0..=d1 {
            if let Some((x, z, r)) = natural_pond_in_cell(cx, cz) {
                if (x - px).hypot(z - pz) <= reach + r {
                    out.push((x, z, r));
                }
            }
        }
    }
    out
}

/// The nearest natural pond to (x,z) — scanning the 3×3 cells around it (a pond can't be more than ~1 cell away).
/// Returns (centre_x, centre_z, radius). For the sim's thirst (it then computes the edge distance + drink reach).
pub fn nearest_natural_pond(x: f64, z: f64) -> Option<(f64, f64, f64)> {
    let cx = (x / POND_CELL).floor() as i32;
    let cz = (z / POND_CELL).floor() as i32;
    let mut best: Option<(f64, f64, f64)> = None;
    let mut best_d = f64::INFINITY;
    for dcx in -1..=1 {
        for dcz in -1..=1 {
            if let Some((wx, wz, r)) = natural_pond_in_cell(cx + dcx, cz + dcz) {
                let d = (wx - x).hypot(wz - z) - r; // distance to the pond's edge
                if d < best_d {
                    best_d = d;
                    best = Some((wx, wz, r));
                }
            }
        }
    }
    best
}

// ───────────────────────── AMBIENT SCATTER (Rust owns the forest field) ─────────────────────────
// Deterministic placement of ambient TREES (clumped into forests) + BUSHES — ported from src/lib/scatter.ts so
// Rust is the single source of truth (render + collision both read the SAME field). Window queries (trees_near /
// bushes_near) so it crosses the wasm boundary ONCE per forest rebuild, not per cell (cheap serialization). Uses
// f64::sin like the JS original — not bit-identical to JS's Math.sin, so the forest is a hair reshuffled vs the old
// JS placement, but consistent (render == collision). JS keeps only the render-side avoidance (paths/lakes it owns).
const SCATTER_STEP: f64 = 16.0; // forest grid cell (m)
const SCATTER_CLEAR: f64 = 70.0; // spawn/build area kept tree-free (radius from origin)
const BUSH_STEP: f64 = 11.0;

fn sfract(v: f64) -> f64 {
    v - v.floor()
}
fn shash(i: f64, j: f64, s: f64) -> f64 {
    sfract((i * 127.1 + j * 311.7 + s * 74.7).sin() * 43758.5453)
}
fn bhash(i: f64, j: f64, s: f64) -> f64 {
    sfract((i * 157.3 + j * 271.9 + s * 53.1).sin() * 43758.5453)
}
fn forest(x: f64, z: f64) -> f64 {
    (x * 0.018 + 2.0).sin() * (z * 0.016 - 1.0).cos() + 0.4 * (x * 0.05).sin() * (z * 0.045).cos()
}
fn color_hash(a: f64, b: f64) -> f64 {
    sfract((a * 12.9898 + b * 78.233).sin() * 43758.5453)
}

/// The tree in cell (ci,cj): (x, z, scale, scaleY, rot, colorHash), or None. Mirrors scatter.ts `treeAt`.
fn tree_in_cell(ci: i32, cj: i32) -> Option<(f64, f64, f64, f64, f64, f64)> {
    let (ci, cj) = (ci as f64, cj as f64);
    let cell_x = ci * SCATTER_STEP;
    let cell_z = cj * SCATTER_STEP;
    if cell_x * cell_x + cell_z * cell_z < SCATTER_CLEAR * SCATTER_CLEAR {
        return None; // keep spawn clear
    }
    if forest(cell_x, cell_z) + (shash(ci, cj, 1.0) - 0.5) < 0.35 {
        return None; // forest clumps only
    }
    let tx = cell_x + (shash(ci, cj, 2.0) - 0.5) * SCATTER_STEP;
    let tz = cell_z + (shash(ci, cj, 3.0) - 0.5) * SCATTER_STEP;
    if in_natural_pond(tx, tz) {
        return None; // trees don't grow in the water
    }
    let scale = 1.3 + shash(ci, cj, 4.0) * 1.6;
    Some((tx, tz, scale, scale + shash(ci, cj, 6.0) * 0.8, shash(ci, cj, 5.0) * 6.283, color_hash(ci, cj)))
}

/// The bush in cell (ci,cj): (x, z, scale, rot, colorHash), or None. Mirrors scatter.ts `bushAt` (offset color hash).
fn bush_in_cell(ci: i32, cj: i32) -> Option<(f64, f64, f64, f64, f64)> {
    let (cif, cjf) = (ci as f64, cj as f64);
    let cell_x = cif * BUSH_STEP;
    let cell_z = cjf * BUSH_STEP;
    if cell_x * cell_x + cell_z * cell_z < SCATTER_CLEAR * SCATTER_CLEAR {
        return None;
    }
    if bhash(cif, cjf, 1.0) > 0.2 {
        return None; // ~1 in 5 cells
    }
    let bx = cell_x + (bhash(cif, cjf, 2.0) - 0.5) * BUSH_STEP;
    let bz = cell_z + (bhash(cif, cjf, 3.0) - 0.5) * BUSH_STEP;
    if in_natural_pond(bx, bz) {
        return None; // bushes don't grow in the water either
    }
    Some((bx, bz, 0.55 + bhash(cif, cjf, 4.0) * 0.75, bhash(cif, cjf, 5.0) * 6.283, color_hash(cif + 4.0, cjf - 7.0)))
}

/// All trees within `reach` of (px,pz) — flat [x, z, scale, scaleY, rot, colorHash] × n. The renderer + collision
/// read this (one wasm call per rebuild); JS still culls trees on its player-made paths/lakes (data it owns).
pub fn trees_near(px: f64, pz: f64, reach: f64) -> Vec<f64> {
    let span = ((reach + SCATTER_STEP * 0.5) / SCATTER_STEP).ceil() as i32;
    let cx = (px / SCATTER_STEP).round() as i32;
    let cz = (pz / SCATTER_STEP).round() as i32;
    let r2 = reach * reach;
    let mut out = Vec::new();
    for ci in (cx - span)..=(cx + span) {
        for cj in (cz - span)..=(cz + span) {
            if let Some((x, z, s, sy, rot, ch)) = tree_in_cell(ci, cj) {
                if (x - px).powi(2) + (z - pz).powi(2) <= r2 {
                    out.extend_from_slice(&[x, z, s, sy, rot, ch]);
                }
            }
        }
    }
    out
}

/// All bushes within `reach` of (px,pz) — flat [x, z, scale, rot, colorHash] × n.
pub fn bushes_near(px: f64, pz: f64, reach: f64) -> Vec<f64> {
    let span = ((reach + BUSH_STEP * 0.5) / BUSH_STEP).ceil() as i32;
    let cx = (px / BUSH_STEP).round() as i32;
    let cz = (pz / BUSH_STEP).round() as i32;
    let r2 = reach * reach;
    let mut out = Vec::new();
    for ci in (cx - span)..=(cx + span) {
        for cj in (cz - span)..=(cz + span) {
            if let Some((x, z, s, rot, ch)) = bush_in_cell(ci, cj) {
                if (x - px).powi(2) + (z - pz).powi(2) <= r2 {
                    out.extend_from_slice(&[x, z, s, rot, ch]);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-4, "expected {b}, got {a}");
    }

    #[test]
    fn kind_table_matches_js() {
        assert_eq!(kind_rh("tree"), (0.8, 3.0));
        assert_eq!(kind_rh("house"), (3.0, 3.0));
        assert_eq!(kind_rh("lion"), (0.85, 0.95));
        assert_eq!(kind_rh("nonsense-kind"), (1.0, 2.0)); // FALLBACK
    }

    #[test]
    fn height_at_matches_js() {
        // reference values captured from src/lib/terrain.ts heightAt (see the engine-port commit)
        approx(height_at(0.0, 0.0, &[]), 0.0);
        approx(height_at(50.0, 0.0, &[]), 0.0); // flat near spawn (buildable)
        approx(height_at(100.0, 40.0, &[]), 1.347053);
        approx(height_at(200.0, -150.0, &[]), 4.269131);
        approx(height_at(-300.0, 220.0, &[]), 11.296692);
        approx(height_at(123.5, -67.25, &[]), 2.384213);
        let feat = [Feature { center: [100.0, 0.0], radius: 24.0, height: 16.0, rough: 0.4 }];
        approx(height_at(100.0, 0.0, &feat), 17.652015);
        approx(height_at(110.0, 5.0, &feat), 10.940461);
        approx(height_at(123.0, 0.0, &feat), 2.489905);
    }

    #[test]
    fn in_water_matches_js() {
        let zones = [WZone { id: "z0".into(), material: "water".into(), pos: [50.0, 0.0, 0.0], size: 10.0 }];
        assert!(in_water(&zones, 50.0, 0.0)); // dead centre
        assert!(!in_water(&zones, 58.0, 0.0)); // inside max radius but past the blob edge
        assert!(!in_water(&zones, 61.0, 0.0)); // outside max radius
        assert!(!in_water(&zones, 50.0, 9.0));
        assert!(!in_water(&zones, 50.0, 11.0));
        assert!(in_water(&zones, 44.0, 3.0));
    }
}
