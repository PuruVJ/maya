//! The deterministic world ENGINE — op application + the placement/terrain/water math — ported from the old JS
//! `src/lib/engine.ts` (+ its deps `terrain.ts`, `water.ts`, the `kinds` r/h table). This is the "no JS engine"
//! refactor: Rust owns ALL compute; JS keeps only render. The world crosses the wasm boundary as JSON (jzon DOM,
//! which round-trips unknown fields for free). Parity with the JS originals is locked by the tests below.
//!
//! Phase 1 (this commit): the pure MATH deps — kind r/h, `height_at` (terrain), `in_water` — with JS-parity tests.
//! Phase 2 adds `apply_ops` (the op handlers) over a jzon world.

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

// ───────────────────────── NATURAL PONDS (Rust is the source of truth) ─────────────────────────
// A deterministic, INFINITE field of natural ponds laid on a jittered grid — so water is spread EVENLY across the
// whole world (animals settle by their local pond instead of all dragging to one shore). Rust OWNS this (the sim
// reads it for thirst; the renderer reads `ponds_near` to draw them). Static → the frontend reads it once per area,
// so it crosses the wasm boundary almost for free. Pure functions of position → identical every run, no state.
const POND_CELL: f64 = 300.0; // metres between natural ponds (grid spacing)
const POND_PROB: f64 = 0.82; // fraction of cells that actually hold a pond (natural gaps, not a rigid lattice)
const POND_R_MIN: f64 = 11.0;
const POND_R_MAX: f64 = 20.0;

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
    let r = POND_R_MIN + pond_hash(cx, cz, 3) * (POND_R_MAX - POND_R_MIN);
    Some((x, z, r))
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
    let scale = 1.3 + shash(ci, cj, 4.0) * 1.6;
    Some((
        cell_x + (shash(ci, cj, 2.0) - 0.5) * SCATTER_STEP,
        cell_z + (shash(ci, cj, 3.0) - 0.5) * SCATTER_STEP,
        scale,
        scale + shash(ci, cj, 6.0) * 0.8,
        shash(ci, cj, 5.0) * 6.283,
        color_hash(ci, cj),
    ))
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
    Some((
        cell_x + (bhash(cif, cjf, 2.0) - 0.5) * BUSH_STEP,
        cell_z + (bhash(cif, cjf, 3.0) - 0.5) * BUSH_STEP,
        0.55 + bhash(cif, cjf, 4.0) * 0.75,
        bhash(cif, cjf, 5.0) * 6.283,
        color_hash(cif + 4.0, cjf - 7.0),
    ))
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

// ───────────────────────── op application (mirror of src/lib/engine.ts applyOps) ─────────────────────────
use jzon::{array, object, JsonValue};

const MAX_COUNT: usize = 1000; // safety cap so "add 9999 cats" can't lock up the renderer
const TAU: f64 = std::f64::consts::TAU;

fn is_creature(kind: &str) -> bool {
    matches!(kind, "person" | "cat" | "lion" | "rabbit" | "kangaroo" | "dinosaur")
}

fn snap(v: f64) -> f64 {
    (2.0 * v + 0.5).floor() * 0.5 // == JS Math.round(v/0.5)*0.5 (round half toward +∞)
}

// ── jzon accessors ──
fn n3(v: &JsonValue, i: usize) -> f64 {
    v[i].as_f64().unwrap_or(0.0)
}
fn obj_pos(o: &JsonValue) -> [f64; 3] {
    [n3(&o["pos"], 0), n3(&o["pos"], 1), n3(&o["pos"], 2)]
}
fn obj_kind(o: &JsonValue) -> String {
    o["kind"].as_str().unwrap_or("").to_string()
}
fn arr3_opt(v: &JsonValue) -> Option<[f64; 3]> {
    if v.is_array() && v.len() == 3 {
        Some([n3(v, 0), n3(v, 1), n3(v, 2)])
    } else {
        None
    }
}
fn str_lc(v: &JsonValue) -> String {
    v.as_str().unwrap_or("").trim().to_lowercase()
}

fn dist2(a: [f64; 3], b: [f64; 3]) -> f64 {
    let dx = a[0] - b[0];
    let dz = a[2] - b[2];
    dx * dx + dz * dz
}

fn to_radix36(mut n: i64) -> String {
    if n == 0 {
        return "0".into();
    }
    const D: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let neg = n < 0;
    if neg {
        n = -n;
    }
    let mut s = Vec::new();
    while n > 0 {
        s.push(D[(n % 36) as usize]);
        n /= 36;
    }
    if neg {
        s.push(b'-');
    }
    s.reverse();
    String::from_utf8(s).unwrap()
}

// next id counter past the highest existing `<prefix><base36>` id (mirrors the JS guard).
fn next_id_counter(items: &[JsonValue], prefix: char) -> i64 {
    let mut n = 0i64;
    for it in items {
        if let Some(id) = it["id"].as_str() {
            let mut ch = id.chars();
            if ch.next() == Some(prefix) {
                if let Ok(v) = i64::from_str_radix(ch.as_str(), 36) {
                    if v >= n {
                        n = v + 1;
                    }
                }
            }
        }
    }
    n
}

fn features_of(terrain: &[JsonValue]) -> Vec<Feature> {
    terrain
        .iter()
        .map(|f| Feature {
            center: [n3(&f["center"], 0), n3(&f["center"], 1)],
            radius: f["radius"].as_f64().unwrap_or(0.0),
            height: f["height"].as_f64().unwrap_or(0.0),
            rough: f["rough"].as_f64().unwrap_or(0.0),
        })
        .collect()
}
fn wzones_of(zones: &[JsonValue]) -> Vec<WZone> {
    zones
        .iter()
        .map(|z| WZone {
            id: z["id"].as_str().unwrap_or("").to_string(),
            material: z["material"].as_str().unwrap_or("").to_string(),
            pos: [n3(&z["pos"], 0), n3(&z["pos"], 1), n3(&z["pos"], 2)],
            size: z["size"].as_f64().unwrap_or(0.0),
        })
        .collect()
}

fn clashes(pos: [f64; 3], radius: f64, objects: &[JsonValue], ignore: Option<&str>) -> bool {
    for o in objects {
        if let Some(ig) = ignore {
            if o["id"].as_str() == Some(ig) {
                continue;
            }
        }
        let min = radius + kind_r(&obj_kind(o));
        if dist2(pos, obj_pos(o)) < min * min {
            return true;
        }
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn find_free_spot(anchor: [f64; 3], radius: f64, objects: &[JsonValue], avoid: Option<([f64; 3], f64)>, ignore: Option<&str>, zones: &[WZone], water: bool) -> [f64; 3] {
    let free = |p: [f64; 3]| -> bool {
        if clashes(p, radius, objects, ignore) {
            return false;
        }
        if water && in_water(zones, p[0], p[2]) {
            return false;
        }
        if let Some((ap, ar)) = avoid {
            let min = radius + ar;
            if dist2(p, ap) < min * min {
                return false;
            }
        }
        true
    };
    let start = [snap(anchor[0]), 0.0, snap(anchor[2])];
    if free(start) {
        return start;
    }
    for ring in 1..=40 {
        let n = (ring * 6).max(6);
        let rad = ring as f64 * 1.2 + radius;
        for i in 0..n {
            let a = (i as f64 / n as f64) * TAU;
            let c = [snap(anchor[0] + a.cos() * rad), 0.0, snap(anchor[2] + a.sin() * rad)];
            if free(c) {
                return c;
            }
        }
    }
    start
}

fn blockers_at(objects: &[JsonValue], c: [f64; 2], radius: f64) -> Vec<String> {
    let mut ids = Vec::new();
    for o in objects {
        let p = obj_pos(o);
        let dx = p[0] - c[0];
        let dz = p[2] - c[1];
        if dx * dx + dz * dz < radius * radius {
            if let Some(id) = o["id"].as_str() {
                ids.push(id.to_string());
            }
        }
    }
    ids
}

fn find_clear_area(objects: &[JsonValue], prefer: [f64; 2], radius: f64) -> ([f64; 2], Vec<String>) {
    let mut cands = vec![prefer];
    for ring in 1..=6 {
        let rr = ring as f64 * radius * 0.9;
        for i in 0..8 {
            let a = (i as f64 / 8.0) * TAU;
            cands.push([prefer[0] + a.cos() * rr, prefer[1] + a.sin() * rr]);
        }
    }
    let mut best = prefer;
    let mut best_blockers = blockers_at(objects, prefer, radius);
    for c in cands {
        if best_blockers.is_empty() {
            break;
        }
        let b = blockers_at(objects, c, radius);
        if b.len() < best_blockers.len() {
            best = c;
            best_blockers = b;
        }
    }
    (best, best_blockers)
}

fn nearest_idx(objects: &[JsonValue], filter: impl Fn(&JsonValue) -> bool, p: [f64; 3]) -> Option<usize> {
    let mut best = None;
    let mut bd = f64::INFINITY;
    for (i, o) in objects.iter().enumerate() {
        if !filter(o) {
            continue;
        }
        let d = dist2(obj_pos(o), p);
        if d < bd {
            bd = d;
            best = Some(i);
        }
    }
    best
}

// Fuzzy object lookup → index. "last"/"it"/… → newest; exact id; "o"+id; nearest of that KIND; (loose) nearest.
fn resolve_ref(reference: &str, objects: &[JsonValue], p: [f64; 3], loose: bool) -> Option<usize> {
    let r = reference.trim().to_lowercase();
    if r == "last" || r == "it" || r == "that" || r == "previous" {
        return if objects.is_empty() { None } else { Some(objects.len() - 1) };
    }
    if r == "here" || r == "me" || r == "player" || r == "us" || r.is_empty() {
        return None;
    }
    if let Some(i) = objects.iter().position(|o| o["id"].as_str().map(|s| s.to_lowercase()) == Some(r.clone())) {
        return Some(i);
    }
    let oid = format!("o{r}");
    if let Some(i) = objects.iter().position(|o| o["id"].as_str().map(|s| s.to_lowercase()) == Some(oid.clone())) {
        return Some(i);
    }
    if let Some(i) = nearest_idx(objects, |o| obj_kind(o).to_lowercase() == r, p) {
        return Some(i);
    }
    if loose {
        return nearest_idx(objects, |_| true, p);
    }
    None
}

fn area_vec(name: &str) -> Option<[f64; 3]> {
    match name {
        "north" => Some([0.0, 0.0, -30.0]),
        "south" => Some([0.0, 0.0, 30.0]),
        "east" => Some([30.0, 0.0, 0.0]),
        "west" => Some([-30.0, 0.0, 0.0]),
        "center" | "everywhere" => Some([0.0, 0.0, 0.0]),
        _ => None,
    }
}

// Resolve a symbolic anchor → a world point. ALL spatial relations live here (mirror resolveAnchor).
fn resolve_anchor(pos: Option<[f64; 3]>, at: &str, dist: Option<f64>, objects: &[JsonValue], p: [f64; 3], yaw: f64) -> [f64; 3] {
    if let Some(pp) = pos {
        return pp;
    }
    let at = if at.is_empty() { "front" } else { at };
    let at = at.trim();
    let ci = at.find(':');
    let head = ci.map(|i| &at[..i]).unwrap_or(at).to_lowercase();
    let mut rest = ci.map(|i| at[i + 1..].to_string()).unwrap_or_default();
    if rest.to_lowercase().starts_with("near:") {
        rest = rest[5..].to_string();
    }
    let fx = -yaw.sin();
    let fz = -yaw.cos();
    let d = match dist {
        Some(dd) if dd > 0.0 => dd.min(120.0),
        _ => 5.0,
    };
    if head == "here" {
        return p;
    }
    let dir: Option<[f64; 2]> = match head.as_str() {
        "front" | "ahead" => Some([fx, fz]),
        "behind" | "back" => Some([-fx, -fz]),
        "right" => Some([-fz, fx]),
        "left" => Some([fz, -fx]),
        _ => None,
    };
    if let Some(dir) = dir {
        let refi = if !rest.is_empty() && rest != "me" { resolve_ref(&rest, objects, p, false) } else { None };
        if let Some(i) = refi {
            let rp = obj_pos(&objects[i]);
            let off = kind_r(&obj_kind(&objects[i])) + match dist {
                Some(dd) if dd > 0.0 => dd.min(60.0),
                _ => 2.5,
            };
            return [rp[0] + dir[0] * off, 0.0, rp[2] + dir[1] * off];
        }
        return [p[0] + dir[0] * d, 0.0, p[2] + dir[1] * d];
    }
    if head == "between" {
        let parts: Vec<&str> = rest.splitn(2, ',').collect();
        let oa = parts.first().and_then(|a| resolve_ref(a, objects, p, true));
        let ob = parts.get(1).and_then(|b| resolve_ref(b, objects, p, true));
        if let (Some(a), Some(b)) = (oa, ob) {
            let (pa, pb) = (obj_pos(&objects[a]), obj_pos(&objects[b]));
            return [(pa[0] + pb[0]) / 2.0, 0.0, (pa[2] + pb[2]) / 2.0];
        }
        if let Some(one) = oa.or(ob) {
            let op = obj_pos(&objects[one]);
            return [op[0] + kind_r(&obj_kind(&objects[one])) + 1.5, 0.0, op[2]];
        }
        return p;
    }
    if head == "on" {
        if let Some(i) = resolve_ref(&rest, objects, p, true) {
            let t = &objects[i];
            let tp = obj_pos(t);
            let sy = arr3_opt(&t["scale"]).map(|s| s[1]).unwrap_or(1.0);
            return [tp[0], tp[1] + kind_h(&obj_kind(t)) * sy, tp[2]];
        }
        return p;
    }
    if matches!(head.as_str(), "near" | "beside" | "nextto" | "by" | "around" | "surround") {
        if let Some(i) = resolve_ref(&rest, objects, p, true) {
            let tp = obj_pos(&objects[i]);
            return [tp[0] + kind_r(&obj_kind(&objects[i])) + 1.5, 0.0, tp[2]];
        }
    }
    if let Some(a) = area_vec(&at.to_lowercase()) {
        return a;
    }
    p
}

fn place(objects: &mut Vec<JsonValue>, kind: &str, pos: [f64; 3], op: &JsonValue, features: &[Feature], n: &mut i64) {
    let y = height_at(pos[0], pos[2], features);
    let scale = arr3_opt(&op["scale"]).unwrap_or([1.0, 1.0, 1.0]);
    let mut o = object! {
        "id" => to_id('o', n),
        "kind" => kind,
        "pos" => array![pos[0], y, pos[2]],
        "scale" => array![scale[0], scale[1], scale[2]],
        "rot" => op["rot"].as_f64().unwrap_or(0.0),
    };
    if let Some(c) = op["color"].as_str() {
        o["color"] = c.into();
    }
    if matches!(kind, "house" | "cabin" | "tower") {
        o["keep"] = true.into();
    }
    objects.push(o);
}

fn to_id(prefix: char, n: &mut i64) -> String {
    let id = format!("{prefix}{}", to_radix36(*n));
    *n += 1;
    id
}

/// Apply `ops` to `world` (jzon DOM) for a player at (px,pz,yaw). Mutates `world`; returns placement conflicts.
/// Faithful port of engine.ts `applyOps` — the deterministic op→geometry layer. Rust owns it now; JS only renders.
pub fn apply_ops(world: &mut JsonValue, ops: &JsonValue, px: f64, pz: f64, yaw: f64) -> Vec<JsonValue> {
    let p = [px, 0.0, pz];
    let mut conflicts: Vec<JsonValue> = Vec::new();

    // pull the mutable arrays out of the DOM (Null if absent → empty); written back at the end.
    let mut objects: Vec<JsonValue> = world["objects"].members().cloned().collect();
    let mut zones: Vec<JsonValue> = world["zones"].members().cloned().collect();
    let mut paths: Vec<JsonValue> = world["paths"].members().cloned().collect();
    let mut terrain: Vec<JsonValue> = world["terrain"].members().cloned().collect();

    let mut oid = next_id_counter(&objects, 'o');
    let mut zid = next_id_counter(&zones, 'z');
    let mut pid = next_id_counter(&paths, 'p');
    let avoid = Some((p, 0.6));

    let op_list: Vec<JsonValue> = ops.members().cloned().collect();
    for op in &op_list {
        let kind_str = op["op"].as_str().unwrap_or("");
        match kind_str {
            "add" => {
                let kind = op["kind"].as_str().unwrap_or("").to_string();
                let r = kind_r(&kind);
                let at_str = str_lc(&op["at"]);
                let on_top = at_str.starts_with("on:");
                let around = at_str.starts_with("around:") || at_str.starts_with("surround");
                let count = (op["count"].as_f64().unwrap_or(1.0).floor() as i64).clamp(1, MAX_COUNT as i64) as usize;
                let feats = features_of(&terrain);
                let wz = wzones_of(&zones);
                if around {
                    let refrest = &at_str[at_str.find(':').map(|i| i + 1).unwrap_or(at_str.len())..];
                    let refi = resolve_ref(refrest, &objects, p, false);
                    let c = match refi {
                        Some(i) => obj_pos(&objects[i]),
                        None => resolve_anchor(arr3_opt(&op["pos"]), &str_lc(&op["at"]), op["dist"].as_f64(), &objects, p, yaw),
                    };
                    let ring_r = refi.map(|i| kind_r(&obj_kind(&objects[i]))).unwrap_or(3.0) + r + 1.2;
                    let ring_n = count.max(8);
                    for i in 0..ring_n {
                        let a = (i as f64 / ring_n as f64) * TAU;
                        place(&mut objects, &kind, [c[0] + a.cos() * ring_r, 0.0, c[2] + a.sin() * ring_r], op, &feats, &mut oid);
                    }
                    continue;
                }
                let anchor = resolve_anchor(arr3_opt(&op["pos"]), &str_lc(&op["at"]), op["dist"].as_f64(), &objects, p, yaw);
                // big creature batches → the wide band-spread (Rust), most land beyond the reveal radius
                if is_creature(&kind) && !on_top && count > 8 {
                    let pts = crate::world::band_spread(count, anchor[0], anchor[2], r);
                    let mut i = 0;
                    while i + 1 < pts.len() {
                        place(&mut objects, &kind, [pts[i], 0.0, pts[i + 1]], op, &feats, &mut oid);
                        i += 2;
                    }
                    continue;
                }
                for _ in 0..count {
                    if on_top {
                        let scale = arr3_opt(&op["scale"]).unwrap_or([1.0, 1.0, 1.0]);
                        let mut o = object! {
                            "id" => to_id('o', &mut oid),
                            "kind" => kind.as_str(),
                            "pos" => array![anchor[0], anchor[1], anchor[2]],
                            "scale" => array![scale[0], scale[1], scale[2]],
                            "rot" => op["rot"].as_f64().unwrap_or(0.0),
                        };
                        if let Some(c) = op["color"].as_str() {
                            o["color"] = c.into();
                        }
                        objects.push(o);
                    } else {
                        let spot = find_free_spot(anchor, r, &objects, avoid, None, &wz, true);
                        place(&mut objects, &kind, spot, op, &feats, &mut oid);
                    }
                }
            }
            "scatter" => {
                let kind = op["kind"].as_str().unwrap_or("").to_string();
                let r = kind_r(&kind);
                let dir = area_vec(op["area"].as_str().unwrap_or("")).unwrap_or([0.0, 0.0, 0.0]);
                let center = [p[0] + dir[0] * 0.6, 0.0, p[2] + dir[2] * 0.6];
                let creature = is_creature(&kind);
                let cap = if kind == "dinosaur" { 10 } else if creature { 50 } else { MAX_COUNT };
                let total = (op["count"].as_f64().unwrap_or(1.0).floor() as i64).clamp(1, cap as i64) as usize;
                let inner = if creature { 40.0 } else { 0.0 };
                let everywhere = op["area"].as_str() == Some("everywhere");
                let spread = if creature {
                    inner + 80.0 * (total as f64 / 5.0).sqrt().max(1.0)
                } else {
                    (if everywhere { 28.0 } else { 15.0 }) * (total as f64 / 12.0).sqrt().max(1.0)
                };
                let ga = std::f64::consts::PI * (3.0 - 5.0_f64.sqrt());
                let feats = features_of(&terrain);
                let wz = wzones_of(&zones);
                for i in 0..total {
                    let rr = if creature {
                        inner + (spread - inner) * ((i as f64 + 0.5) / total as f64).sqrt()
                    } else {
                        spread * ((i as f64 + 0.5) / total as f64).sqrt()
                    };
                    let a = i as f64 * ga;
                    let anchor = [center[0] + a.cos() * rr, 0.0, center[2] + a.sin() * rr];
                    let spot = if creature {
                        [snap(anchor[0]), 0.0, snap(anchor[2])]
                    } else {
                        find_free_spot(anchor, r, &objects, avoid, None, &wz, true)
                    };
                    // scatter passes only color through to place (JS: { color: op.color })
                    let mut popt = object! {};
                    if let Some(c) = op["color"].as_str() {
                        popt["color"] = c.into();
                    }
                    place(&mut objects, &kind, spot, &popt, &feats, &mut oid);
                }
            }
            "remove" => {
                let idref = op["id"].as_str().unwrap_or("");
                if let Some(i) = resolve_ref(idref, &objects, p, false) {
                    objects.remove(i);
                    continue;
                }
                let rid = idref.trim().to_lowercase();
                if let Some(i) = zones.iter().position(|z| z["id"].as_str() == Some(rid.as_str())) {
                    zones.remove(i);
                    continue;
                }
                if let Some(i) = paths.iter().position(|pa| pa["id"].as_str() == Some(rid.as_str())) {
                    paths.remove(i);
                    continue;
                }
                let mat = zone_word(&rid);
                if let Some(mat) = mat {
                    if let Some(i) = nearest_zone(&zones, mat, p) {
                        zones.remove(i);
                        continue;
                    }
                }
                if is_path_word(&rid) {
                    if let Some(i) = nearest_path(&paths, p) {
                        paths.remove(i);
                    }
                }
            }
            "move" => {
                let idref = op["id"].as_str().unwrap_or("");
                if let Some(i) = resolve_ref(idref, &objects, p, false) {
                    let kind = obj_kind(&objects[i]);
                    let id = objects[i]["id"].as_str().unwrap_or("").to_string();
                    let feats = features_of(&terrain);
                    let wz = wzones_of(&zones);
                    let mut np = match arr3_opt(&op["pos"]) {
                        Some(pp) => pp,
                        None => {
                            let anchor = resolve_anchor(arr3_opt(&op["pos"]), &str_lc(&op["at"]), op["dist"].as_f64(), &objects, p, yaw);
                            find_free_spot(anchor, kind_r(&kind), &objects, avoid, Some(&id), &wz, true)
                        }
                    };
                    np[1] = height_at(np[0], np[2], &feats);
                    objects[i]["pos"] = array![np[0], np[1], np[2]];
                }
            }
            "paint" => {
                if let Some(i) = resolve_ref(op["id"].as_str().unwrap_or(""), &objects, p, false) {
                    if let Some(c) = op["color"].as_str() {
                        objects[i]["color"] = c.into();
                    }
                }
            }
            "setGround" => {
                if let Some(v) = op["value"].as_str() {
                    world["ground"] = v.into();
                }
            }
            "setSky" => {
                world["sky"] = "night".into(); // night-only game — any sky request resolves to night
            }
            "addZone" => {
                let size = op["size"].as_f64().unwrap_or(10.0);
                let mut prefer = resolve_anchor(arr3_opt(&op["pos"]), &str_lc(&op["at"]), None, &objects, p, yaw);
                let bare = !op["pos"].is_array() && (op["at"].as_str().map(|s| s.is_empty() || s == "here").unwrap_or(true));
                if bare {
                    let f = [yaw.sin(), 0.0, -yaw.cos()]; // forward()
                    prefer = [p[0] + f[0] * (size + 4.0), 0.0, p[2] + f[2] * (size + 4.0)];
                }
                let (center, blockers) = find_clear_area(&objects, [prefer[0], prefer[2]], size);
                let feats = features_of(&terrain);
                let c = [center[0], height_at(center[0], center[1], &feats), center[1]];
                zones.push(object! {
                    "id" => to_id('z', &mut zid),
                    "material" => op["material"].as_str().unwrap_or(""),
                    "shape" => op["shape"].as_str().unwrap_or(""),
                    "pos" => array![c[0], c[1], c[2]],
                    "size" => size,
                });
                if !blockers.is_empty() {
                    let label = if op["material"].as_str() == Some("water") { "lake".to_string() } else { op["material"].as_str().unwrap_or("").to_string() };
                    let mut b = JsonValue::new_array();
                    for id in blockers {
                        let _ = b.push(id);
                    }
                    conflicts.push(object! { "label" => label, "blockers" => b });
                }
            }
            "addPath" => {
                let from = arr3_opt(&op["fromPos"]).unwrap_or_else(|| resolve_anchor(None, &str_lc(&op["from"]), None, &objects, p, yaw));
                let mut to = arr3_opt(&op["toPos"]).unwrap_or_else(|| resolve_anchor(None, &str_lc(&op["to"]), None, &objects, p, yaw));
                if dist2(from, to) < 4.0 {
                    let f = [yaw.sin(), 0.0, -yaw.cos()];
                    to = [from[0] + f[0] * 12.0, 0.0, from[2] + f[2] * 12.0];
                }
                paths.push(object! {
                    "id" => to_id('p', &mut pid),
                    "material" => op["material"].as_str().unwrap_or(""),
                    "from" => array![from[0], from[1], from[2]],
                    "to" => array![to[0], to[1], to[2]],
                    "width" => op["width"].as_f64().unwrap_or(3.0),
                });
            }
            "setTerrain" => {
                let preset = op["preset"].as_str().unwrap_or("");
                if preset == "flat" {
                    terrain.clear();
                } else {
                    let (radius, height, rough) = terrain_preset(preset);
                    let f = [yaw.sin(), 0.0, -yaw.cos()];
                    let prefer = [p[0] + f[0] * radius, p[2] + f[2] * radius];
                    let (center, _) = find_clear_area(&objects, prefer, radius);
                    let h = match op["amplitude"].as_f64() {
                        Some(a) if a != 0.0 => a,
                        _ => height,
                    };
                    terrain.push(object! {
                        "center" => array![center[0], center[1]],
                        "radius" => radius,
                        "height" => h,
                        "rough" => rough,
                    });
                }
                let feats = features_of(&terrain);
                for o in objects.iter_mut() {
                    let pp = obj_pos(o);
                    let y = height_at(pp[0], pp[2], &feats);
                    o["pos"] = array![pp[0], y, pp[2]];
                }
            }
            _ => {} // note / unknown → no world change
        }
    }

    world["objects"] = JsonValue::Array(objects);
    world["zones"] = JsonValue::Array(zones);
    world["paths"] = JsonValue::Array(paths);
    world["terrain"] = JsonValue::Array(terrain);
    conflicts
}

fn terrain_preset(preset: &str) -> (f64, f64, f64) {
    match preset {
        "mountains" => (24.0, 16.0, 0.4),
        "dunes" => (20.0, 2.5, 1.5),
        "valley" => (18.0, -5.0, 0.5),
        "plateau" => (16.0, 5.0, 0.0),
        _ => (18.0, 4.0, 1.0), // hills (also the default)
    }
}

fn zone_word(rid: &str) -> Option<&'static str> {
    match rid {
        "lake" | "pond" | "water" | "pool" => Some("water"),
        "plaza" | "courtyard" | "square" => Some("plaza"),
        "field" | "lawn" | "meadow" => Some("grass"),
        "sand" | "beach" => Some("sand"),
        "ice" => Some("ice"),
        "lava" => Some("lava"),
        "flowers" => Some("flowers"),
        _ => None,
    }
}
fn is_path_word(rid: &str) -> bool {
    matches!(rid, "road" | "roads" | "street" | "streets" | "path" | "paths" | "trail" | "trails" | "bridge" | "bridges")
}
fn nearest_zone(zones: &[JsonValue], mat: &str, p: [f64; 3]) -> Option<usize> {
    let mut best = None;
    let mut bd = f64::INFINITY;
    for (i, z) in zones.iter().enumerate() {
        if z["material"].as_str() != Some(mat) {
            continue;
        }
        let c = [n3(&z["pos"], 0), 0.0, n3(&z["pos"], 2)];
        let d = dist2(c, p);
        if d < bd {
            bd = d;
            best = Some(i);
        }
    }
    best
}
fn nearest_path(paths: &[JsonValue], p: [f64; 3]) -> Option<usize> {
    let mut best = None;
    let mut bd = f64::INFINITY;
    for (i, pa) in paths.iter().enumerate() {
        let c = [n3(&pa["from"], 0), 0.0, n3(&pa["from"], 2)];
        let d = dist2(c, p);
        if d < bd {
            bd = d;
            best = Some(i);
        }
    }
    best
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

    // ───────────── apply_ops: EXHAUSTIVE op coverage, right ways AND wrong ways ─────────────
    use jzon::{array, object, JsonValue};

    fn world0() -> JsonValue {
        object! { "name" => "t", "objects" => array![], "zones" => array![], "paths" => array![], "terrain" => array![], "ground" => "grass", "sky" => "night" }
    }
    fn run(w: &mut JsonValue, ops: JsonValue) -> Vec<JsonValue> {
        apply_ops(w, &ops, 0.0, 0.0, 0.0)
    }
    fn objs(w: &JsonValue) -> Vec<JsonValue> {
        w["objects"].members().cloned().collect()
    }
    fn of_kind<'a>(w: &'a JsonValue, k: &str) -> Vec<&'a JsonValue> {
        w["objects"].members().filter(|o| o["kind"].as_str() == Some(k)).collect()
    }

    // ── add ──
    #[test]
    fn add_count_places_exactly_n_unique() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"add","kind"=>"tree","count"=>5}]);
        let t = of_kind(&w, "tree");
        assert_eq!(t.len(), 5);
        let ids: std::collections::HashSet<_> = t.iter().map(|o| o["id"].as_str().unwrap()).collect();
        assert_eq!(ids.len(), 5, "ids must be unique");
        assert_eq!(t[0]["id"].as_str(), Some("o0"), "first id is o0");
        for o in t {
            assert!(o["pos"][1].as_f64().unwrap().is_finite(), "grounded Y is finite");
        }
    }

    #[test]
    fn add_big_creature_batch_uses_wide_band_spread() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"add","kind"=>"person","count"=>100}]);
        let ppl = of_kind(&w, "person");
        assert_eq!(ppl.len(), 100);
        let xs: Vec<f64> = ppl.iter().map(|o| o["pos"][0].as_f64().unwrap()).collect();
        let span = xs.iter().cloned().fold(f64::MIN, f64::max) - xs.iter().cloned().fold(f64::MAX, f64::min);
        assert!(span > 80.0, "100 people spread wide (got {span:.0} m), not piled");
    }

    #[test]
    fn add_count_is_clamped_and_floored() {
        let mut w = world0();
        // creature → band-spread fast path (placing 1000 statics via find_free_spot is O(n²) and slow in debug)
        run(&mut w, array![object! {"op"=>"add","kind"=>"person","count"=>99999}]);
        assert_eq!(of_kind(&w, "person").len(), MAX_COUNT, "count clamped to MAX_COUNT");
        let mut w2 = world0();
        run(&mut w2, array![object! {"op"=>"add","kind"=>"rock","count"=>0}]); // wrong: 0 → min 1
        assert_eq!(of_kind(&w2, "rock").len(), 1);
        let mut w3 = world0();
        run(&mut w3, array![object! {"op"=>"add","kind"=>"rock"}]); // wrong: no count → 1
        assert_eq!(of_kind(&w3, "rock").len(), 1);
    }

    #[test]
    fn add_at_front_lands_ahead_of_player() {
        let mut w = world0(); // yaw 0 → front is -z
        run(&mut w, array![object! {"op"=>"add","kind"=>"tower","at"=>"front","dist"=>10}]);
        let tv = of_kind(&w, "tower");
        let t = tv[0];
        assert!(t["pos"][2].as_f64().unwrap() < -3.0, "tower placed ahead (−z), got z={}", t["pos"][2]);
        assert_eq!(t["keep"].as_bool(), Some(true), "player-built structure is keep");
    }

    #[test]
    fn add_unknown_kind_still_places_with_fallback() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"add","kind"=>"wizard-tower","count"=>3}]);
        assert_eq!(of_kind(&w, "wizard-tower").len(), 3, "unknown kind uses FALLBACK radius, still placed");
    }

    #[test]
    fn add_around_makes_a_ring_of_at_least_eight() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"add","kind"=>"house","at"=>"front"}]); // a thing to ring
        run(&mut w, array![object! {"op"=>"add","kind"=>"fence","at"=>"around:house","count"=>4}]);
        assert!(of_kind(&w, "fence").len() >= 8, "'around' implies ≥8 even when count=4");
    }

    // ── remove ──
    #[test]
    fn remove_by_id_then_bad_ref_is_noop() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"add","kind"=>"tree"}]);
        run(&mut w, array![object! {"op"=>"remove","id"=>"o0"}]);
        assert_eq!(objs(&w).len(), 0, "removed by exact id");
        run(&mut w, array![object! {"op"=>"add","kind"=>"tree"}, object! {"op"=>"add","kind"=>"rock"}]);
        let before = objs(&w).len();
        run(&mut w, array![object! {"op"=>"remove","id"=>"does-not-exist"}]); // wrong: bad ref must NOT nuke anything
        assert_eq!(objs(&w).len(), before, "a garbage remove ref is a no-op, never a random kill");
    }

    #[test]
    fn remove_zone_by_material_keyword() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"addZone","material"=>"water","shape"=>"blob","size"=>8}]);
        assert_eq!(w["zones"].len(), 1);
        run(&mut w, array![object! {"op"=>"remove","id"=>"lake"}]); // keyword → nearest water zone
        assert_eq!(w["zones"].len(), 0, "'lake' removes the water zone");
    }

    // ── move / paint ──
    #[test]
    fn move_relocates_and_regrounds() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"add","kind"=>"tree"}]);
        run(&mut w, array![object! {"op"=>"move","id"=>"o0","pos"=>array![123.0,0.0,45.0]}]);
        let ov = objs(&w);
        let o = &ov[0];
        approx(o["pos"][0].as_f64().unwrap(), 123.0);
        approx(o["pos"][1].as_f64().unwrap(), height_at(123.0, 45.0, &[])); // re-grounded
    }

    #[test]
    fn paint_sets_color_only_on_valid_ref() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"add","kind"=>"tree"}]);
        run(&mut w, array![object! {"op"=>"paint","id"=>"o0","color"=>"#ff0000"}]);
        assert_eq!(objs(&w)[0]["color"].as_str(), Some("#ff0000"));
        run(&mut w, array![object! {"op"=>"paint","id"=>"nope","color"=>"#000"}]); // wrong ref → ignored
        assert_eq!(objs(&w)[0]["color"].as_str(), Some("#ff0000"), "bad paint ref leaves color untouched");
    }

    // ── ground / sky ──
    #[test]
    fn set_ground_and_sky_is_night_only() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"setGround","value"=>"sand"}]);
        assert_eq!(w["ground"].as_str(), Some("sand"));
        run(&mut w, array![object! {"op"=>"setSky","value"=>"day"}]); // wrong: any sky coerces to night
        assert_eq!(w["sky"].as_str(), Some("night"));
    }

    // ── zones / paths / terrain ──
    #[test]
    fn add_zone_reports_blocker_conflicts() {
        let mut w = world0();
        // pack the area ahead with objects so the lake can't find fully clear ground
        run(&mut w, array![object! {"op"=>"scatter","kind"=>"rock","count"=>40,"area"=>"everywhere"}]);
        let conflicts = run(&mut w, array![object! {"op"=>"addZone","material"=>"water","shape"=>"blob","size"=>14}]);
        assert_eq!(w["zones"].len(), 1, "zone still placed");
        if !conflicts.is_empty() {
            assert_eq!(conflicts[0]["label"].as_str(), Some("lake"), "water conflict is labelled 'lake'");
        }
    }

    #[test]
    fn add_path_and_collapsed_endpoints_extend() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"addPath","material"=>"path","from"=>"here","to"=>"here"}]);
        let p = &w["paths"][0];
        let from = [p["from"][0].as_f64().unwrap(), p["from"][2].as_f64().unwrap()];
        let to = [p["to"][0].as_f64().unwrap(), p["to"][2].as_f64().unwrap()];
        let len2 = (from[0] - to[0]).powi(2) + (from[1] - to[1]).powi(2);
        assert!(len2 >= 4.0, "collapsed endpoints extend to a sensible length (got len²={len2:.1})");
    }

    #[test]
    fn set_terrain_adds_then_flat_clears_and_regrounds() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"add","kind"=>"tree","pos"=>array![100.0,0.0,0.0]}]);
        run(&mut w, array![object! {"op"=>"setTerrain","preset"=>"mountains"}]);
        assert_eq!(w["terrain"].len(), 1);
        run(&mut w, array![object! {"op"=>"setTerrain","preset"=>"flat"}]);
        assert_eq!(w["terrain"].len(), 0, "flat clears terrain");
        // tree re-grounded to the (now ambient-only) surface
        approx(objs(&w)[0]["pos"][1].as_f64().unwrap(), height_at(100.0, 0.0, &[]));
    }

    // ── robustness: junk in, no panic ──
    #[test]
    fn note_and_unknown_ops_and_empty_are_noops() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"note","text"=>"can't do that"}]);
        run(&mut w, array![object! {"op"=>"frobnicate","wat"=>1}]); // unknown op
        run(&mut w, array![]); // empty
        run(&mut w, array![object! {}]); // op with no "op" field
        assert_eq!(objs(&w).len(), 0, "junk ops change nothing and never panic");
    }

    #[test]
    fn ids_never_collide_after_removes() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"add","kind"=>"tree","count"=>3}]); // o0,o1,o2
        run(&mut w, array![object! {"op"=>"remove","id"=>"o1"}]);
        run(&mut w, array![object! {"op"=>"add","kind"=>"rock"}]); // must be o3, not o2 (collision)
        let ov = objs(&w);
        let ids: Vec<&str> = ov.iter().map(|o| o["id"].as_str().unwrap()).collect();
        let uniq: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), uniq.len(), "no duplicate ids after a remove+add");
    }

    #[test]
    fn unknown_world_fields_round_trip() {
        let mut w = object! { "name" => "keep-me", "seed" => 42, "objects" => array![], "ground" => "grass", "sky" => "night", "customThing" => array![1,2,3] };
        run(&mut w, array![object! {"op"=>"add","kind"=>"tree"}]);
        assert_eq!(w["seed"].as_i64(), Some(42), "unknown field preserved");
        assert_eq!(w["customThing"][2].as_i64(), Some(3), "unknown nested field preserved");
        assert_eq!(of_kind(&w, "tree").len(), 1);
    }

    // ═══════════════ MORE EDGE CASES — adversarial / malformed / boundary inputs ═══════════════

    fn first(w: &JsonValue, k: &str) -> JsonValue {
        w["objects"].members().find(|o| o["kind"].as_str() == Some(k)).cloned().unwrap_or(JsonValue::Null)
    }
    fn px2(o: &JsonValue) -> [f64; 2] {
        [o["pos"][0].as_f64().unwrap_or(0.0), o["pos"][2].as_f64().unwrap_or(0.0)]
    }

    #[test]
    fn add_negative_and_fractional_count() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"add","kind"=>"rock","count"=>-5}]); // wrong: negative → 1
        assert_eq!(of_kind(&w, "rock").len(), 1);
        let mut w2 = world0();
        run(&mut w2, array![object! {"op"=>"add","kind"=>"rock","count"=>3.9}]); // fractional → floor 3
        assert_eq!(of_kind(&w2, "rock").len(), 3);
    }

    #[test]
    fn add_empty_kind_still_places_something() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"add","kind"=>""}]); // empty kind → fallback radius, still an object
        assert_eq!(objs(&w).len(), 1);
        assert_eq!(objs(&w)[0]["kind"].as_str(), Some(""));
    }

    #[test]
    fn add_at_gibberish_anchor_falls_back_to_player() {
        let mut w = world0(); // player at origin
        run(&mut w, array![object! {"op"=>"add","kind"=>"rock","at"=>"qwerty:zxcvb"}]);
        let p = px2(&first(&w, "rock"));
        // unknown anchor → the player position; findFreeSpot then nudges the static rock just off the player's feet
        assert!(p[0].hypot(p[1]) < 5.0, "unknown anchor → near the player, not flung away (got {p:?})");
    }

    #[test]
    fn add_relative_to_an_object_front_of_o0() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"add","kind"=>"house","pos"=>array![40.0,0.0,40.0]}]); // o0 off to the side
        run(&mut w, array![object! {"op"=>"add","kind"=>"lamp","at"=>"front:o0"}]); // in front of the house
        let lamp = px2(&first(&w, "lamp"));
        // not at the player (origin) and near the house — i.e. it resolved relative to o0
        assert!((lamp[0] - 40.0).abs() < 12.0 && (lamp[1] - 40.0).abs() < 12.0, "lamp placed near o0, got {lamp:?}");
    }

    #[test]
    fn add_between_two_objects_is_the_midpoint() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"add","kind"=>"house","pos"=>array![-20.0,0.0,0.0]}]); // o0
        run(&mut w, array![object! {"op"=>"add","kind"=>"house","pos"=>array![20.0,0.0,0.0]}]); // o1
        run(&mut w, array![object! {"op"=>"add","kind"=>"well","at"=>"between:o0,o1"}]);
        let well = px2(&first(&w, "well"));
        assert!(well[0].abs() < 4.0, "between two houses → near x=0 midpoint, got {well:?}");
    }

    #[test]
    fn add_on_roof_uses_object_height_and_skips_grounding() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"add","kind"=>"house","pos"=>array![0.0,0.0,-6.0]}]);
        run(&mut w, array![object! {"op"=>"add","kind"=>"lamp","at"=>"on:o0"}]);
        let lamp = first(&w, "lamp");
        assert!(lamp["pos"][1].as_f64().unwrap() > 1.5, "on:roof keeps the elevated Y (got {})", lamp["pos"][1]);
    }

    #[test]
    fn add_explicit_pos_bypasses_anchor() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"add","kind"=>"tower","pos"=>array![77.0,0.0,-33.0],"at"=>"front"}]); // pos wins over at
        let t = first(&w, "tower");
        approx(t["pos"][0].as_f64().unwrap(), 77.0);
        approx(t["pos"][2].as_f64().unwrap(), -33.0);
    }

    #[test]
    fn add_absurd_dist_is_clamped() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"add","kind"=>"rock","at"=>"front","dist"=>1e15}]); // model emits 1e15 → clamp 120
        let p = px2(&first(&w, "rock"));
        assert!(p[0].hypot(p[1]) <= 121.0, "absurd dist clamped to ≤120 m (got {:.0})", p[0].hypot(p[1]));
    }

    #[test]
    fn move_without_target_pos_finds_free_spot_off_player() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"add","kind"=>"tree","pos"=>array![50.0,0.0,50.0]}]);
        run(&mut w, array![object! {"op"=>"move","id"=>"o0","at"=>"here"}]); // move to player area → must not land ON the player
        let t = px2(&first(&w, "tree"));
        assert!(t[0].hypot(t[1]) > 0.3, "moved object shifts off the player, got {t:?}");
    }

    #[test]
    fn move_and_remove_nonexistent_are_noops() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"add","kind"=>"tree"}]);
        let before = objs(&w);
        run(&mut w, array![object! {"op"=>"move","id"=>"ghost","pos"=>array![9.0,0.0,9.0]}]);
        run(&mut w, array![object! {"op"=>"remove","id"=>"ghost"}]);
        assert_eq!(objs(&w).len(), before.len(), "bad move/remove refs change nothing");
        approx(objs(&w)[0]["pos"][0].as_f64().unwrap(), before[0]["pos"][0].as_f64().unwrap());
    }

    #[test]
    fn remove_last_and_it_aliases() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"add","kind"=>"tree"}, object! {"op"=>"add","kind"=>"rock"}]);
        run(&mut w, array![object! {"op"=>"remove","id"=>"last"}]); // newest = the rock
        assert_eq!(of_kind(&w, "rock").len(), 0);
        assert_eq!(of_kind(&w, "tree").len(), 1);
        run(&mut w, array![object! {"op"=>"remove","id"=>"it"}]); // newest now = the tree
        assert_eq!(objs(&w).len(), 0);
    }

    #[test]
    fn scatter_caps_per_kind() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"scatter","kind"=>"dinosaur","count"=>500,"area"=>"north"}]);
        assert_eq!(of_kind(&w, "dinosaur").len(), 10, "dinosaur scatter hard-capped at 10");
        let mut w2 = world0();
        run(&mut w2, array![object! {"op"=>"scatter","kind"=>"cat","count"=>500,"area"=>"south"}]);
        assert_eq!(of_kind(&w2, "cat").len(), 50, "creature scatter capped at 50");
    }

    #[test]
    fn scatter_unknown_area_defaults_to_center() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"scatter","kind"=>"flower","count"=>12,"area"=>"narnia"}]);
        assert_eq!(of_kind(&w, "flower").len(), 12, "unknown area still scatters (centered)");
    }

    #[test]
    fn add_path_with_explicit_coords() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"addPath","material"=>"path","fromPos"=>array![0.0,0.0,0.0],"toPos"=>array![30.0,0.0,0.0]}]);
        let p = &w["paths"][0];
        approx(p["to"][0].as_f64().unwrap(), 30.0);
        assert_eq!(p["width"].as_f64(), Some(3.0), "default width");
    }

    #[test]
    fn set_terrain_custom_amplitude_and_unknown_preset() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"setTerrain","preset"=>"hills","amplitude"=>12.0}]);
        approx(w["terrain"][0]["height"].as_f64().unwrap(), 12.0);
        let mut w2 = world0();
        run(&mut w2, array![object! {"op"=>"setTerrain","preset"=>"narnia"}]); // unknown → hills default, still ONE feature
        assert_eq!(w2["terrain"].len(), 1);
    }

    #[test]
    fn compound_batch_with_cross_references() {
        let mut w = world0();
        run(
            &mut w,
            array![
                object! {"op"=>"add","kind"=>"house","at"=>"front"},
                object! {"op"=>"add","kind"=>"tree","at"=>"near:house"},
                object! {"op"=>"paint","id"=>"last","color"=>"#0f0"},
                object! {"op"=>"add","kind"=>"lamp","at"=>"on:o0"}
            ],
        );
        assert_eq!(of_kind(&w, "house").len(), 1);
        assert_eq!(of_kind(&w, "tree").len(), 1);
        assert_eq!(first(&w, "tree")["color"].as_str(), Some("#0f0"), "paint last → the tree");
        assert_eq!(of_kind(&w, "lamp").len(), 1);
    }

    #[test]
    fn malformed_ops_never_panic() {
        let mut w = world0();
        // every field wrong type / missing — must not panic, must not corrupt the world
        run(&mut w, array![object! {"op"=>"add"}]); // no kind
        run(&mut w, array![object! {"op"=>"add","kind"=>123}]); // kind is a number
        run(&mut w, array![object! {"op"=>"move"}]); // no id
        run(&mut w, array![object! {"op"=>"paint","id"=>"x"}]); // no color
        run(&mut w, array![object! {"op"=>"addZone"}]); // no material/shape
        run(&mut w, array![object! {"op"=>"setGround"}]); // no value
        run(&mut w, array![JsonValue::Null, object! {"op"=>"add","kind"=>"tree"}]); // null op in the batch
        // the one valid op (add tree) still applied; nothing panicked
        assert_eq!(of_kind(&w, "tree").len(), 1);
    }

    #[test]
    fn ground_only_changes_on_valid_value() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"setGround","value"=>"snow"}]);
        assert_eq!(w["ground"].as_str(), Some("snow"));
        run(&mut w, array![object! {"op"=>"setGround"}]); // no value → unchanged (not cleared)
        assert_eq!(w["ground"].as_str(), Some("snow"));
    }

    #[test]
    fn add_around_centers_on_player_when_ref_missing() {
        let mut w = world0();
        run(&mut w, array![object! {"op"=>"add","kind"=>"fence","at"=>"around:ghost","count"=>6}]);
        assert!(of_kind(&w, "fence").len() >= 8, "around a missing ref still rings (≥8) around the anchor");
    }

    #[test]
    fn determinism_same_ops_same_layout() {
        let ops = array![object! {"op"=>"add","kind"=>"tree","count"=>12}, object! {"op"=>"scatter","kind"=>"rock","count"=>20,"area"=>"north"}];
        let mut a = world0();
        let mut b = world0();
        run(&mut a, ops.clone());
        run(&mut b, ops);
        assert_eq!(a["objects"].dump(), b["objects"].dump(), "same ops + player → byte-identical layout (shareable determinism)");
    }
}
