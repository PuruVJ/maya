//! BINARY apply_ops engine (docs/world-data-architecture.md — the jzon-drop endgame). A typed-STRUCT port of
//! `engine::apply_ops`, built ALONGSIDE the jzon version and parity-tested. It is UNWIRED — the live path stays on
//! `engine::apply_ops` (jzon) until this is proven byte-identical and the wiring is approved. No JSON DOM, no jzon.
//!
//! The world crosses the wasm boundary as flat typed arrays + parallel string vecs (ids/kinds/materials/colours), NOT
//! a JSON string: JS packs `[x,y,z,sx,sy,sz,rot,keep,gene]×n` etc. + the strings, Rust decodes to these structs, runs
//! the engine, and returns a binary DELTA (built incrementally — see the op-match step). This file grows in steps:
//! (1) structs + decode [HERE], (2) collision helpers, (3) ref/anchor resolution, (4) the op match, (5) parity test.

use crate::engine::{height_at, in_water, kind_h, kind_r, WZone}; // shared, pure (no jzon) — reused from the live engine
pub use crate::engine::Feature as EFeat; // terrain feature is identical to engine::Feature → reuse it (+ height_at)

/// A world object — structures AND creatures (apply_ops round-trips both; creatures are obstacles + ref targets, and
/// `gene` rides along for a creature `add`). `color` is `None` when JS passed an empty string.
#[derive(Clone, Debug, PartialEq)]
pub struct EObj {
    pub id: String,
    pub kind: String,
    pub pos: [f64; 3],
    pub scale: [f64; 3],
    pub rot: f64,
    pub color: Option<String>,
    pub keep: bool,
    pub gene: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EZone {
    pub id: String,
    pub material: String,
    pub shape: String,
    pub pos: [f64; 3],
    pub size: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EPath {
    pub id: String,
    pub material: String,
    pub from: [f64; 3],
    pub to: [f64; 3],
    pub width: f64,
}

/// Decode objects: parallel `ids`/`kinds`/`colors` (color `""` = none) + a flat num SoA, stride 9 =
/// `[x, y, z, sx, sy, sz, rot, keep, gene]`. Extra/short string vecs are tolerated (default "").
pub fn decode_objs(ids: &[String], kinds: &[String], colors: &[String], num: &[f64]) -> Vec<EObj> {
    num.chunks_exact(9)
        .enumerate()
        .map(|(i, c)| EObj {
            id: ids.get(i).cloned().unwrap_or_default(),
            kind: kinds.get(i).cloned().unwrap_or_default(),
            pos: [c[0], c[1], c[2]],
            scale: [c[3], c[4], c[5]],
            rot: c[6],
            keep: c[7] != 0.0,
            gene: c[8],
            color: colors.get(i).filter(|s| !s.is_empty()).cloned(),
        })
        .collect()
}

/// Decode zones: `ids`/`materials`/`shapes` + num stride 4 = `[x, y, z, size]`.
pub fn decode_zones(ids: &[String], materials: &[String], shapes: &[String], num: &[f64]) -> Vec<EZone> {
    num.chunks_exact(4)
        .enumerate()
        .map(|(i, c)| EZone {
            id: ids.get(i).cloned().unwrap_or_default(),
            material: materials.get(i).cloned().unwrap_or_default(),
            shape: shapes.get(i).cloned().unwrap_or_default(),
            pos: [c[0], c[1], c[2]],
            size: c[3],
        })
        .collect()
}

/// Decode paths: `ids`/`materials` + num stride 7 = `[fromX, fromY, fromZ, toX, toY, toZ, width]`.
pub fn decode_paths(ids: &[String], materials: &[String], num: &[f64]) -> Vec<EPath> {
    num.chunks_exact(7)
        .enumerate()
        .map(|(i, c)| EPath {
            id: ids.get(i).cloned().unwrap_or_default(),
            material: materials.get(i).cloned().unwrap_or_default(),
            from: [c[0], c[1], c[2]],
            to: [c[3], c[4], c[5]],
            width: c[6],
        })
        .collect()
}

/// Decode terrain features: num stride 5 = `[centerX, centerZ, radius, height, rough]`.
pub fn decode_terrain(num: &[f64]) -> Vec<EFeat> {
    num.chunks_exact(5).map(|c| EFeat { center: [c[0], c[1]], radius: c[2], height: c[3], rough: c[4] }).collect()
}

// ── encode (the inverse of decode_*) — the apply_ops result rides back to JS as the SAME parallel arrays ──────────────
/// Objects → `(ids, kinds, colors, num)`, colors `""` for `None` (inverse of `decode_objs`, num stride 9).
pub fn encode_objs(objs: &[EObj]) -> (Vec<String>, Vec<String>, Vec<String>, Vec<f64>) {
    let mut ids = Vec::with_capacity(objs.len());
    let mut kinds = Vec::with_capacity(objs.len());
    let mut colors = Vec::with_capacity(objs.len());
    let mut num = Vec::with_capacity(objs.len() * 9);
    for o in objs {
        ids.push(o.id.clone());
        kinds.push(o.kind.clone());
        colors.push(o.color.clone().unwrap_or_default());
        num.extend_from_slice(&[o.pos[0], o.pos[1], o.pos[2], o.scale[0], o.scale[1], o.scale[2], o.rot, if o.keep { 1.0 } else { 0.0 }, o.gene]);
    }
    (ids, kinds, colors, num)
}

/// Zones → `(ids, materials, shapes, num)` (inverse of `decode_zones`, num stride 4 = `[x, y, z, size]`).
pub fn encode_zones(zones: &[EZone]) -> (Vec<String>, Vec<String>, Vec<String>, Vec<f64>) {
    let mut ids = Vec::with_capacity(zones.len());
    let mut materials = Vec::with_capacity(zones.len());
    let mut shapes = Vec::with_capacity(zones.len());
    let mut num = Vec::with_capacity(zones.len() * 4);
    for z in zones {
        ids.push(z.id.clone());
        materials.push(z.material.clone());
        shapes.push(z.shape.clone());
        num.extend_from_slice(&[z.pos[0], z.pos[1], z.pos[2], z.size]);
    }
    (ids, materials, shapes, num)
}

/// Paths → `(ids, materials, num)` (inverse of `decode_paths`, num stride 7 = `[fx, fy, fz, tx, ty, tz, width]`).
pub fn encode_paths(paths: &[EPath]) -> (Vec<String>, Vec<String>, Vec<f64>) {
    let mut ids = Vec::with_capacity(paths.len());
    let mut materials = Vec::with_capacity(paths.len());
    let mut num = Vec::with_capacity(paths.len() * 7);
    for p in paths {
        ids.push(p.id.clone());
        materials.push(p.material.clone());
        num.extend_from_slice(&[p.from[0], p.from[1], p.from[2], p.to[0], p.to[1], p.to[2], p.width]);
    }
    (ids, materials, num)
}

/// Terrain → num (inverse of `decode_terrain`, stride 5 = `[cx, cz, radius, height, rough]`).
pub fn encode_terrain(terrain: &[EFeat]) -> Vec<f64> {
    let mut num = Vec::with_capacity(terrain.len() * 5);
    for f in terrain {
        num.extend_from_slice(&[f.center[0], f.center[1], f.radius, f.height, f.rough]);
    }
    num
}

// ── collision / placement helpers (struct ports of engine.rs; byte-identical math) ──────────────────────────────────
const TAU: f64 = std::f64::consts::TAU;

/// JS `Math.round(v/0.5)*0.5` — round half toward +∞ (duplicated from engine.rs `snap`, kept private there).
fn snap(v: f64) -> f64 {
    (2.0 * v + 0.5).floor() * 0.5
}
fn dist2(a: [f64; 3], b: [f64; 3]) -> f64 {
    let dx = a[0] - b[0];
    let dz = a[2] - b[2];
    dx * dx + dz * dz
}

/// True if `pos` (with bounding `radius`) overlaps any object (radius = kind_r), skipping `ignore`'s id.
pub fn clashes(pos: [f64; 3], radius: f64, objects: &[EObj], ignore: Option<&str>) -> bool {
    for o in objects {
        if ignore == Some(o.id.as_str()) {
            continue;
        }
        let min = radius + kind_r(&o.kind);
        if dist2(pos, o.pos) < min * min {
            return true;
        }
    }
    false
}

/// Spiral out from `anchor` for the first clear (no clash, not in water per `water(x,z)`, clear of `avoid`) snapped
/// spot. Mirrors engine.rs `find_free_spot`; the caller supplies the water predicate (so `water=false` is just `|_,_|false`).
pub fn find_free_spot(anchor: [f64; 3], radius: f64, objects: &[EObj], avoid: Option<([f64; 3], f64)>, ignore: Option<&str>, water: &dyn Fn(f64, f64) -> bool) -> [f64; 3] {
    let free = |p: [f64; 3]| -> bool {
        if clashes(p, radius, objects, ignore) {
            return false;
        }
        if water(p[0], p[2]) {
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

/// Ids of objects whose centre is within `radius` of `c` (x,z) — the would-be blockers under a big footprint.
pub fn blockers_at(objects: &[EObj], c: [f64; 2], radius: f64) -> Vec<String> {
    objects.iter().filter(|o| (o.pos[0] - c[0]).powi(2) + (o.pos[2] - c[1]).powi(2) < radius * radius).map(|o| o.id.clone()).collect()
}

/// Nudge a big-footprint placement toward the least-blocked nearby spot. Mirrors engine.rs `find_clear_area`.
pub fn find_clear_area(objects: &[EObj], prefer: [f64; 2], radius: f64) -> ([f64; 2], Vec<String>) {
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

// ── reference + anchor resolution (struct ports of engine.rs; the fuzzy spatial layer) ───────────────────────────────
fn nearest_idx(objects: &[EObj], filter: impl Fn(&EObj) -> bool, p: [f64; 3]) -> Option<usize> {
    let mut best = None;
    let mut bd = f64::INFINITY;
    for (i, o) in objects.iter().enumerate() {
        if !filter(o) {
            continue;
        }
        let d = dist2(o.pos, p);
        if d < bd {
            bd = d;
            best = Some(i);
        }
    }
    best
}

/// Fuzzy object lookup → index. "last"/"it"/… → newest; exact id; "o"+id; nearest of that KIND; (loose) nearest.
pub fn resolve_ref(reference: &str, objects: &[EObj], p: [f64; 3], loose: bool) -> Option<usize> {
    let r = reference.trim().to_lowercase();
    if r == "last" || r == "it" || r == "that" || r == "previous" {
        return if objects.is_empty() { None } else { Some(objects.len() - 1) };
    }
    if r == "here" || r == "me" || r == "player" || r == "us" || r.is_empty() {
        return None;
    }
    if let Some(i) = objects.iter().position(|o| o.id.to_lowercase() == r) {
        return Some(i);
    }
    let oid = format!("o{r}");
    if let Some(i) = objects.iter().position(|o| o.id.to_lowercase() == oid) {
        return Some(i);
    }
    if let Some(i) = nearest_idx(objects, |o| o.kind.to_lowercase() == r, p) {
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

/// Resolve a symbolic anchor (`pos` | `at` like 'front'/'on:house'/'around:cat'/'between:a,b' | `dist` | `yaw`) → a
/// world point. ALL spatial relations live here; mirror of engine.rs `resolve_anchor`.
pub fn resolve_anchor(pos: Option<[f64; 3]>, at: &str, dist: Option<f64>, objects: &[EObj], p: [f64; 3], yaw: f64) -> [f64; 3] {
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
            let rp = objects[i].pos;
            let off = kind_r(&objects[i].kind)
                + match dist {
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
            let (pa, pb) = (objects[a].pos, objects[b].pos);
            return [(pa[0] + pb[0]) / 2.0, 0.0, (pa[2] + pb[2]) / 2.0];
        }
        if let Some(one) = oa.or(ob) {
            let op = objects[one].pos;
            return [op[0] + kind_r(&objects[one].kind) + 1.5, 0.0, op[2]];
        }
        return p;
    }
    if head == "on" {
        if let Some(i) = resolve_ref(&rest, objects, p, true) {
            let t = &objects[i];
            return [t.pos[0], t.pos[1] + kind_h(&t.kind) * t.scale[1], t.pos[2]];
        }
        return p;
    }
    if matches!(head.as_str(), "near" | "beside" | "nextto" | "by" | "around" | "surround") {
        if let Some(i) = resolve_ref(&rest, objects, p, true) {
            let tp = objects[i].pos;
            return [tp[0] + kind_r(&objects[i].kind) + 1.5, 0.0, tp[2]];
        }
    }
    if let Some(a) = area_vec(&at.to_lowercase()) {
        return a;
    }
    p
}

// ── the op match (struct port of engine::apply_ops) ──────────────────────────────────────────────────────────────────
const MAX_COUNT: usize = 1000; // safety cap so "add 9999 cats" can't lock up the renderer (mirror of engine.rs)

fn is_creature(kind: &str) -> bool {
    matches!(kind, "person" | "cat" | "lion" | "rabbit" | "kangaroo" | "dinosaur")
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
fn nearest_zone(zones: &[EZone], mat: &str, p: [f64; 3]) -> Option<usize> {
    let mut best = None;
    let mut bd = f64::INFINITY;
    for (i, z) in zones.iter().enumerate() {
        if z.material != mat {
            continue;
        }
        let d = dist2([z.pos[0], 0.0, z.pos[2]], p);
        if d < bd {
            bd = d;
            best = Some(i);
        }
    }
    best
}
fn nearest_path(paths: &[EPath], p: [f64; 3]) -> Option<usize> {
    let mut best = None;
    let mut bd = f64::INFINITY;
    for (i, pa) in paths.iter().enumerate() {
        let d = dist2([pa.from[0], 0.0, pa.from[2]], p);
        if d < bd {
            bd = d;
            best = Some(i);
        }
    }
    best
}

fn wzones_of(zones: &[EZone]) -> Vec<WZone> {
    zones.iter().map(|z| WZone { id: z.id.clone(), material: z.material.clone(), pos: z.pos, size: z.size }).collect()
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

fn to_id(prefix: char, n: &mut i64) -> String {
    let id = format!("{prefix}{}", to_radix36(*n));
    *n += 1;
    id
}

/// Next id counter past the highest existing `<prefix><base36>` id (mirrors the JS guard).
fn next_id_counter<'a>(ids: impl Iterator<Item = &'a str>, prefix: char) -> i64 {
    let mut n = 0i64;
    for id in ids {
        let mut ch = id.chars();
        if ch.next() == Some(prefix) {
            if let Ok(v) = i64::from_str_radix(ch.as_str(), 36) {
                if v >= n {
                    n = v + 1;
                }
            }
        }
    }
    n
}

/// Create an object at `pos` (grounded by the terrain), append it. Houses/cabins/towers are `keep`. Mirror of `place`.
fn place(objects: &mut Vec<EObj>, kind: &str, pos: [f64; 3], scale: Option<[f64; 3]>, rot: f64, color: &Option<String>, features: &[EFeat], n: &mut i64) {
    let y = height_at(pos[0], pos[2], features);
    objects.push(EObj {
        id: to_id('o', n),
        kind: kind.to_string(),
        pos: [pos[0], y, pos[2]],
        scale: scale.unwrap_or([1.0, 1.0, 1.0]),
        rot,
        color: color.clone(),
        keep: matches!(kind, "house" | "cabin" | "tower"),
        gene: 0.0,
    });
}

/// A placement conflict (a big footprint landed on existing objects) — surfaced to the player, mirrors the jzon shape.
#[derive(Clone, Debug, PartialEq)]
pub struct Conflict {
    pub label: String,
    pub blockers: Vec<String>,
}

/// The mutable world the binary engine operates on (the typed counterpart of the jzon `world` DOM).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct EWorld {
    pub objects: Vec<EObj>,
    pub zones: Vec<EZone>,
    pub paths: Vec<EPath>,
    pub terrain: Vec<EFeat>,
    pub ground: String,
    pub sky: String,
}

/// A grammar op, already decoded from JS (the binary op-stream decode is a SEPARATE wiring step — this is the engine's
/// internal form). One variant per `engine::apply_ops` branch; fields mirror the jzon keys each branch reads.
#[derive(Clone, Debug, PartialEq)]
pub enum EOp {
    Add { kind: String, at: String, pos: Option<[f64; 3]>, dist: Option<f64>, count: f64, scale: Option<[f64; 3]>, rot: f64, color: Option<String> },
    Scatter { kind: String, area: String, count: f64, color: Option<String> },
    Remove { id: String },
    Move { id: String, pos: Option<[f64; 3]>, at: String, dist: Option<f64> },
    Paint { id: String, color: Option<String> },
    SetGround { value: String },
    SetSky,
    AddZone { material: String, shape: String, at: String, pos: Option<[f64; 3]>, size: f64 },
    AddPath { material: String, from: String, to: String, from_pos: Option<[f64; 3]>, to_pos: Option<[f64; 3]>, width: f64 },
    SetTerrain { preset: String, amplitude: Option<f64> },
    Note,
}

/// Decode the binary op stream JS packs: `num` stride 19, `strs` stride 11. NaN = "field absent" (so optional pos /
/// dist / scale / amplitude round-trip), and defaults (count 1, rot 0, size 10, width 3) are applied HERE so the EOp
/// matches what the jzon engine read via `.unwrap_or(..)`. Empty string = absent (color → None).
///
/// `num`  [0]tag [1]count [2..5]pos [5]dist [6..9]scale [9]rot [10]size [11..14]fromPos [14..17]toPos [17]width [18]amplitude
/// `strs` [0]kind [1]at [2]id [3]color [4]material [5]shape [6]from [7]to [8]value [9]area [10]preset
pub fn decode_ops(num: &[f64], strs: &[String]) -> Vec<EOp> {
    num.chunks_exact(19)
        .enumerate()
        .map(|(i, c)| {
            let s = |j: usize| strs.get(i * 11 + j).cloned().unwrap_or_default();
            let col = |j: usize| {
                let v = s(j);
                if v.is_empty() {
                    None
                } else {
                    Some(v)
                }
            };
            let opt = |v: f64| if v.is_nan() { None } else { Some(v) };
            let pos = |a: usize| if c[a].is_nan() { None } else { Some([c[a], c[a + 1], c[a + 2]]) };
            let or = |v: f64, d: f64| if v.is_nan() { d } else { v };
            match c[0] as i64 {
                0 => EOp::Add { kind: s(0), at: s(1), pos: pos(2), dist: opt(c[5]), count: or(c[1], 1.0), scale: pos(6), rot: or(c[9], 0.0), color: col(3) },
                1 => EOp::Scatter { kind: s(0), area: s(9), count: or(c[1], 1.0), color: col(3) },
                2 => EOp::Remove { id: s(2) },
                3 => EOp::Move { id: s(2), pos: pos(2), at: s(1), dist: opt(c[5]) },
                4 => EOp::Paint { id: s(2), color: col(3) },
                5 => EOp::SetGround { value: s(8) },
                6 => EOp::SetSky,
                7 => EOp::AddZone { material: s(4), shape: s(5), at: s(1), pos: pos(2), size: or(c[10], 10.0) },
                8 => EOp::AddPath { material: s(4), from: s(6), to: s(7), from_pos: pos(11), to_pos: pos(14), width: or(c[17], 3.0) },
                9 => EOp::SetTerrain { preset: s(10), amplitude: opt(c[18]) },
                _ => EOp::Note,
            }
        })
        .collect()
}

/// Apply `ops` to `world` for a player at (px,pz,yaw). Mutates `world`; returns placement conflicts. Struct-for-struct
/// port of `engine::apply_ops` — the SAME deterministic op→geometry layer, no jzon DOM. Parity-pinned by a Rust test.
pub fn apply_ops_bin(world: &mut EWorld, ops: &[EOp], px: f64, pz: f64, yaw: f64) -> Vec<Conflict> {
    let p = [px, 0.0, pz];
    let mut conflicts: Vec<Conflict> = Vec::new();

    // move the arrays out (the borrow checker wants objects/terrain disjoint for place()); written back at the end.
    let mut objects = std::mem::take(&mut world.objects);
    let mut zones = std::mem::take(&mut world.zones);
    let mut paths = std::mem::take(&mut world.paths);
    let mut terrain = std::mem::take(&mut world.terrain);

    let mut oid = next_id_counter(objects.iter().map(|o| o.id.as_str()), 'o');
    let mut zid = next_id_counter(zones.iter().map(|z| z.id.as_str()), 'z');
    let mut pid = next_id_counter(paths.iter().map(|p| p.id.as_str()), 'p');
    let avoid = Some((p, 0.6));
    let fwd = [yaw.sin(), 0.0, -yaw.cos()]; // forward()

    for op in ops {
        match op {
            EOp::Add { kind, at, pos, dist, count, scale, rot, color } => {
                let r = kind_r(kind);
                let at_str = at.trim().to_lowercase();
                let on_top = at_str.starts_with("on:");
                let around = at_str.starts_with("around:") || at_str.starts_with("surround");
                let count = (count.floor() as i64).clamp(1, MAX_COUNT as i64) as usize;
                let wz = wzones_of(&zones);
                if around {
                    let refrest = &at_str[at_str.find(':').map(|i| i + 1).unwrap_or(at_str.len())..];
                    let refi = resolve_ref(refrest, &objects, p, false);
                    let c = match refi {
                        Some(i) => objects[i].pos,
                        None => resolve_anchor(*pos, &at_str, *dist, &objects, p, yaw),
                    };
                    let ring_r = refi.map(|i| kind_r(&objects[i].kind)).unwrap_or(3.0) + r + 1.2;
                    let ring_n = count.max(8);
                    for i in 0..ring_n {
                        let a = (i as f64 / ring_n as f64) * TAU;
                        place(&mut objects, kind, [c[0] + a.cos() * ring_r, 0.0, c[2] + a.sin() * ring_r], *scale, *rot, color, &terrain, &mut oid);
                    }
                    continue;
                }
                let anchor = resolve_anchor(*pos, &at_str, *dist, &objects, p, yaw);
                // BIG CREATURE BATCH: a VISIBLE herd lands right at the anchor so the add is actually SEEN, then the
                // BULK band-spreads WIDE (most → dormant aggregates, perf-safe) so "add 1000" doesn't pin 1000 live near
                // you. Without the near herd the whole batch scattered past the reveal radius → "+100 count, zero visible
                // creatures" (the add-100-dinosaurs bug); the far ones still materialise as you explore toward them.
                if is_creature(kind) && !on_top && count > 8 {
                    let ga = std::f64::consts::PI * (3.0 - 5.0_f64.sqrt());
                    let near = (count / 3).clamp(8, 24).min(count);
                    for i in 0..near {
                        let rr = 20.0 + 28.0 * ((i as f64 + 0.5) / near as f64).sqrt(); // 20–48 m → clearly visible (near-LOD)
                        let a = i as f64 * ga;
                        let water = |x: f64, z: f64| in_water(&wz, x, z);
                        let spot = find_free_spot([anchor[0] + a.cos() * rr, 0.0, anchor[2] + a.sin() * rr], r, &objects, avoid, None, &water);
                        place(&mut objects, kind, spot, *scale, *rot, color, &terrain, &mut oid);
                    }
                    if count > near {
                        let pts = crate::world::band_spread(count - near, anchor[0], anchor[2], r);
                        let mut i = 0;
                        while i + 1 < pts.len() {
                            place(&mut objects, kind, [pts[i], 0.0, pts[i + 1]], *scale, *rot, color, &terrain, &mut oid);
                            i += 2;
                        }
                    }
                    continue;
                }
                for _ in 0..count {
                    if on_top {
                        // on top of a target: keep the resolved Y, no re-grounding, no `keep` (mirror of the jzon inline)
                        objects.push(EObj {
                            id: to_id('o', &mut oid),
                            kind: kind.clone(),
                            pos: anchor,
                            scale: scale.unwrap_or([1.0, 1.0, 1.0]),
                            rot: *rot,
                            color: color.clone(),
                            keep: false,
                            gene: 0.0,
                        });
                    } else {
                        let water = |x: f64, z: f64| in_water(&wz, x, z);
                        let spot = find_free_spot(anchor, r, &objects, avoid, None, &water);
                        place(&mut objects, kind, spot, *scale, *rot, color, &terrain, &mut oid);
                    }
                }
            }
            EOp::Scatter { kind, area, count, color } => {
                let r = kind_r(kind);
                let dir = area_vec(area).unwrap_or([0.0, 0.0, 0.0]);
                let center = [p[0] + dir[0] * 0.6, 0.0, p[2] + dir[2] * 0.6];
                let creature = is_creature(kind);
                let cap = if kind == "dinosaur" { 10 } else if creature { 50 } else { MAX_COUNT };
                let total = (count.floor() as i64).clamp(1, cap as i64) as usize;
                let inner = if creature { 40.0 } else { 0.0 };
                let everywhere = area == "everywhere";
                let spread = if creature {
                    inner + 80.0 * (total as f64 / 5.0).sqrt().max(1.0)
                } else {
                    (if everywhere { 28.0 } else { 15.0 }) * (total as f64 / 12.0).sqrt().max(1.0)
                };
                let ga = std::f64::consts::PI * (3.0 - 5.0_f64.sqrt());
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
                        let water = |x: f64, z: f64| in_water(&wz, x, z);
                        find_free_spot(anchor, r, &objects, avoid, None, &water)
                    };
                    place(&mut objects, kind, spot, None, 0.0, color, &terrain, &mut oid); // scatter passes only color
                }
            }
            EOp::Remove { id } => {
                if let Some(i) = resolve_ref(id, &objects, p, false) {
                    objects.remove(i);
                    continue;
                }
                let rid = id.trim().to_lowercase();
                if let Some(i) = zones.iter().position(|z| z.id == rid) {
                    zones.remove(i);
                    continue;
                }
                if let Some(i) = paths.iter().position(|pa| pa.id == rid) {
                    paths.remove(i);
                    continue;
                }
                if let Some(mat) = zone_word(&rid) {
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
            EOp::Move { id, pos, at, dist } => {
                if let Some(i) = resolve_ref(id, &objects, p, false) {
                    let kind = objects[i].kind.clone();
                    let mid = objects[i].id.clone();
                    let mut np = match pos {
                        Some(pp) => *pp,
                        None => {
                            let wz = wzones_of(&zones);
                            let water = |x: f64, z: f64| in_water(&wz, x, z);
                            let anchor = resolve_anchor(None, &at.trim().to_lowercase(), *dist, &objects, p, yaw);
                            find_free_spot(anchor, kind_r(&kind), &objects, avoid, Some(&mid), &water)
                        }
                    };
                    np[1] = height_at(np[0], np[2], &terrain);
                    objects[i].pos = np;
                }
            }
            EOp::Paint { id, color } => {
                if let Some(i) = resolve_ref(id, &objects, p, false) {
                    if let Some(c) = color {
                        objects[i].color = Some(c.clone());
                    }
                }
            }
            EOp::SetGround { value } => {
                world.ground = value.clone();
            }
            EOp::SetSky => {
                world.sky = "night".to_string(); // night-only game — any sky request resolves to night
            }
            EOp::AddZone { material, shape, at, pos, size } => {
                let bare = pos.is_none() && (at.is_empty() || at == "here"); // raw `at` (mirrors the jzon as_str check)
                let prefer = if bare {
                    [p[0] + fwd[0] * (size + 4.0), 0.0, p[2] + fwd[2] * (size + 4.0)]
                } else {
                    resolve_anchor(*pos, &at.trim().to_lowercase(), None, &objects, p, yaw)
                };
                let (center, blockers) = find_clear_area(&objects, [prefer[0], prefer[2]], *size);
                let c = [center[0], height_at(center[0], center[1], &terrain), center[1]];
                zones.push(EZone { id: to_id('z', &mut zid), material: material.clone(), shape: shape.clone(), pos: c, size: *size });
                if !blockers.is_empty() {
                    let label = if material == "water" { "lake".to_string() } else { material.clone() };
                    conflicts.push(Conflict { label, blockers });
                }
            }
            EOp::AddPath { material, from, to, from_pos, to_pos, width } => {
                let from_p = from_pos.unwrap_or_else(|| resolve_anchor(None, &from.trim().to_lowercase(), None, &objects, p, yaw));
                let mut to_p = to_pos.unwrap_or_else(|| resolve_anchor(None, &to.trim().to_lowercase(), None, &objects, p, yaw));
                if dist2(from_p, to_p) < 4.0 {
                    to_p = [from_p[0] + fwd[0] * 12.0, 0.0, from_p[2] + fwd[2] * 12.0];
                }
                paths.push(EPath { id: to_id('p', &mut pid), material: material.clone(), from: from_p, to: to_p, width: *width });
            }
            EOp::SetTerrain { preset, amplitude } => {
                if preset == "flat" {
                    terrain.clear();
                } else {
                    let (radius, height, rough) = terrain_preset(preset);
                    let prefer = [p[0] + fwd[0] * radius, p[2] + fwd[2] * radius];
                    let (center, _) = find_clear_area(&objects, prefer, radius);
                    let h = match amplitude {
                        Some(a) if *a != 0.0 => *a,
                        _ => height,
                    };
                    terrain.push(EFeat { center: [center[0], center[1]], radius, height: h, rough });
                }
                for o in objects.iter_mut() {
                    let pp = o.pos;
                    o.pos = [pp[0], height_at(pp[0], pp[2], &terrain), pp[2]];
                }
            }
            EOp::Note => {} // note / unknown → no world change
        }
    }

    world.objects = objects;
    world.zones = zones;
    world.paths = paths;
    world.terrain = terrain;
    conflicts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_objs_maps_fields_and_empty_color() {
        let ids = vec!["h0".to_string(), "p0".to_string()];
        let kinds = vec!["house".to_string(), "rabbit".to_string()];
        let colors = vec!["#abc123".to_string(), String::new()]; // "" → None
        #[rustfmt::skip]
        let num = vec![
            1.0, 0.0, 2.0,  1.2, 1.2, 1.2,  45.0, 1.0, 0.0, // house: keep=true, gene 0
            3.0, 0.0, 4.0,  1.0, 1.0, 1.0,   0.0, 0.0, 1.1, // rabbit: keep=false, gene 1.1
        ];
        let objs = decode_objs(&ids, &kinds, &colors, &num);
        assert_eq!(objs.len(), 2);
        assert_eq!(objs[0], EObj { id: "h0".into(), kind: "house".into(), pos: [1.0, 0.0, 2.0], scale: [1.2, 1.2, 1.2], rot: 45.0, color: Some("#abc123".into()), keep: true, gene: 0.0 });
        assert_eq!(objs[1].color, None);
        assert_eq!((objs[1].kind.as_str(), objs[1].gene, objs[1].keep), ("rabbit", 1.1, false));
    }

    #[test]
    fn decode_zones_paths_terrain() {
        let z = decode_zones(&["z0".into()], &["water".into()], &["blob".into()], &[10.0, 0.0, -5.0, 8.0]);
        assert_eq!(z, vec![EZone { id: "z0".into(), material: "water".into(), shape: "blob".into(), pos: [10.0, 0.0, -5.0], size: 8.0 }]);
        let p = decode_paths(&["p0".into()], &["path".into()], &[0.0, 0.0, 0.0, 12.0, 0.0, 3.0, 4.0]);
        assert_eq!(p, vec![EPath { id: "p0".into(), material: "path".into(), from: [0.0, 0.0, 0.0], to: [12.0, 0.0, 3.0], width: 4.0 }]);
        let t = decode_terrain(&[5.0, -5.0, 40.0, 12.0, 0.3]);
        assert_eq!(t, vec![EFeat { center: [5.0, -5.0], radius: 40.0, height: 12.0, rough: 0.3 }]);
    }

    fn obj(id: &str, kind: &str, x: f64, z: f64) -> EObj {
        EObj { id: id.into(), kind: kind.into(), pos: [x, 0.0, z], scale: [1.0; 3], rot: 0.0, color: None, keep: false, gene: 0.0 }
    }

    #[test]
    fn find_free_spot_dodges_clashes_and_water() {
        let no_water = |_: f64, _: f64| false;
        // empty area → the snapped anchor is free, returned as-is
        assert_eq!(find_free_spot([50.0, 0.0, 50.0], 1.0, &[], None, None, &no_water), [50.0, 0.0, 50.0]);
        // anchor on top of a house → it clashes → the spiral finds a CLEAR spot
        let objs = vec![obj("a", "house", 0.0, 0.0)];
        let r = kind_r("house");
        assert!(clashes([0.0, 0.0, 0.0], r, &objs, None), "the anchor clashes (guards a vacuous test)");
        let spot = find_free_spot([0.0, 0.0, 0.0], r, &objs, None, None, &no_water);
        assert!(!clashes(spot, r, &objs, None), "spiral found a clear spot off the house ({spot:?})");
        // a water predicate (x<5 is 'water') pushes the placement out of the band
        let dry = find_free_spot([0.0, 0.0, 50.0], 1.0, &[], None, None, &|x: f64, _z: f64| x < 5.0);
        assert!(dry[0] >= 5.0, "placed clear of the water band ({dry:?})");
    }

    #[test]
    fn resolve_ref_and_anchor() {
        let objs = vec![obj("h0", "house", 10.0, 0.0), obj("c0", "cat", 3.0, 4.0)];
        let p = [0.0, 0.0, 0.0];
        // exact id · kind word (nearest of kind) · "last" (newest) · unknown(strict)=None · unknown(loose)=nearest
        assert_eq!(resolve_ref("h0", &objs, p, false), Some(0));
        assert_eq!(resolve_ref("cat", &objs, p, false), Some(1));
        assert_eq!(resolve_ref("last", &objs, p, false), Some(1));
        assert_eq!(resolve_ref("dragon", &objs, p, false), None);
        assert_eq!(resolve_ref("dragon", &objs, p, true), Some(1)); // cat d²25 < house d²100

        // explicit pos wins · 'front' at yaw 0 → -Z by 5 · 'here' → player · 'on:h0' → atop the house (Y raised)
        assert_eq!(resolve_anchor(Some([7.0, 0.0, 8.0]), "", None, &objs, p, 0.0), [7.0, 0.0, 8.0]);
        assert_eq!(resolve_anchor(None, "front", None, &objs, p, 0.0), [0.0, 0.0, -5.0]);
        assert_eq!(resolve_anchor(None, "here", None, &objs, p, 0.0), p);
        let on = resolve_anchor(None, "on:h0", None, &objs, p, 0.0);
        assert_eq!((on[0], on[2]), (10.0, 0.0));
        assert!(on[1] > 0.0, "on top of the house raises Y ({on:?})");
        // 'between:h0,c0' → midpoint
        assert_eq!(resolve_anchor(None, "between:h0,c0", None, &objs, p, 0.0), [6.5, 0.0, 2.0]);
    }

    fn add(kind: &str, at: &str, count: f64) -> EOp {
        EOp::Add { kind: kind.into(), at: at.into(), pos: None, dist: None, count, scale: None, rot: 0.0, color: None }
    }

    #[test]
    fn apply_ops_bin_runs_each_branch() {
        let mut w = EWorld::default();
        // add a house in front (yaw 0 → -Z), then a cat on top of it, then paint the house
        let c = apply_ops_bin(&mut w, &[add("house", "front", 1.0)], 0.0, 0.0, 0.0);
        assert!(c.is_empty());
        assert_eq!(w.objects.len(), 1);
        assert_eq!((w.objects[0].kind.as_str(), w.objects[0].keep), ("house", true)); // houses are keep
        let hid = w.objects[0].id.clone();
        apply_ops_bin(&mut w, &[add("cat", &format!("on:{hid}"), 1.0)], 0.0, 0.0, 0.0);
        assert_eq!(w.objects.len(), 2);
        assert_eq!(w.objects[1].kind, "cat");
        assert!(!w.objects[1].keep, "on-top placement never sets keep");
        apply_ops_bin(&mut w, &[EOp::Paint { id: hid.clone(), color: Some("#ff0000".into()) }], 0.0, 0.0, 0.0);
        assert_eq!(w.objects.iter().find(|o| o.id == hid).unwrap().color.as_deref(), Some("#ff0000"));

        // ground/sky, a water zone, a path, terrain — then assert each landed
        apply_ops_bin(&mut w, &[EOp::SetGround { value: "sand".into() }, EOp::SetSky], 0.0, 0.0, 0.0);
        assert_eq!((w.ground.as_str(), w.sky.as_str()), ("sand", "night"));
        apply_ops_bin(&mut w, &[EOp::AddZone { material: "water".into(), shape: "blob".into(), at: String::new(), pos: None, size: 12.0 }], 0.0, 0.0, 0.0);
        assert_eq!(w.zones.len(), 1);
        assert_eq!(w.zones[0].id, "z0");
        apply_ops_bin(&mut w, &[EOp::AddPath { material: "path".into(), from: "here".into(), to: "north".into(), from_pos: None, to_pos: None, width: 3.0 }], 0.0, 0.0, 0.0);
        assert_eq!(w.paths.len(), 1);
        apply_ops_bin(&mut w, &[EOp::SetTerrain { preset: "mountains".into(), amplitude: None }], 0.0, 0.0, 0.0);
        assert_eq!(w.terrain.len(), 1);
        assert_eq!(w.terrain[0].height, 16.0); // mountains preset

        // scatter 12 flowers, then remove the cat by kind word, then 'remove water' (zone word)
        let before = w.objects.len();
        apply_ops_bin(&mut w, &[EOp::Scatter { kind: "flower".into(), area: "center".into(), count: 12.0, color: None }], 0.0, 0.0, 0.0);
        assert_eq!(w.objects.len(), before + 12);
        apply_ops_bin(&mut w, &[EOp::Remove { id: "cat".into() }], 0.0, 0.0, 0.0);
        assert!(!w.objects.iter().any(|o| o.kind == "cat"), "the cat was removed by kind word");
        apply_ops_bin(&mut w, &[EOp::Remove { id: "water".into() }], 0.0, 0.0, 0.0);
        assert!(w.zones.is_empty(), "the water zone was removed by zone word");
    }

    #[test]
    fn big_person_add_lands_a_visible_near_herd() {
        // mirrors the live LLM op {op:'add', kind:'person', count:100} with no `at` (→ "front") at the player origin.
        let mut w = EWorld::default();
        let c = apply_ops_bin(&mut w, &[add("person", "", 100.0)], 0.0, 0.0, 0.0);
        assert!(c.is_empty(), "a plain creature add raises no placement conflict");
        let people: Vec<&EObj> = w.objects.iter().filter(|o| o.kind == "person").collect();
        assert_eq!(people.len(), 100, "all 100 people are actually added to the world");
        // a VISIBLE near-herd (~24) lands within ~60 m of the player so the add is SEEN; the rest band-spread far.
        let near = people.iter().filter(|o| o.pos[0].hypot(o.pos[2]) <= 60.0).count();
        assert!(near >= 20, "a near-herd lands close to the player (got {near} within 60 m)");
    }

    #[test]
    fn encode_decode_roundtrip() {
        let objs = vec![
            EObj { id: "o1".into(), kind: "house".into(), pos: [1.0, 2.0, 3.0], scale: [1.1, 1.2, 1.3], rot: 0.5, color: Some("#abc".into()), keep: true, gene: 0.3 },
            EObj { id: "o2".into(), kind: "rabbit".into(), pos: [4.0, 0.0, 5.0], scale: [1.0, 1.0, 1.0], rot: 0.0, color: None, keep: false, gene: 0.0 },
        ];
        let (ids, kinds, colors, num) = encode_objs(&objs);
        assert_eq!(decode_objs(&ids, &kinds, &colors, &num), objs);

        let zones = vec![EZone { id: "z1".into(), material: "water".into(), shape: "blob".into(), pos: [1.0, 0.0, 2.0], size: 8.0 }];
        let (zi, zm, zs, zn) = encode_zones(&zones);
        assert_eq!(decode_zones(&zi, &zm, &zs, &zn), zones);

        let paths = vec![EPath { id: "p1".into(), material: "path".into(), from: [0.0, 0.0, 0.0], to: [3.0, 0.0, 4.0], width: 2.0 }];
        let (pi, pm, pn) = encode_paths(&paths);
        assert_eq!(decode_paths(&pi, &pm, &pn), paths);

        let terrain = vec![EFeat { center: [1.0, 2.0], radius: 10.0, height: 5.0, rough: 0.5 }];
        assert_eq!(decode_terrain(&encode_terrain(&terrain)), terrain);
    }

    #[test]
    fn decode_ops_builds_variants() {
        let n = f64::NAN;
        // [0]tag [1]count [2..5]pos [5]dist [6..9]scale [9]rot [10]size [11..14]fromPos [14..17]toPos [17]width [18]amp
        #[rustfmt::skip]
        let num = vec![
            0.0, 5.0, n,n,n, n, n,n,n, n, n, n,n,n, n,n,n, n, n,             // add rabbit, count 5, color #f00
            8.0, n,   n,n,n, n, n,n,n, n, n, n,n,n, 1.0,0.0,2.0, 4.0, n,     // addPath path, from "here", toPos [1,0,2], width 4
            9.0, n,   n,n,n, n, n,n,n, n, n, n,n,n, n,n,n, n, 7.5,           // setTerrain mountains, amplitude 7.5
        ];
        // [0]kind [1]at [2]id [3]color [4]material [5]shape [6]from [7]to [8]value [9]area [10]preset
        let strs: Vec<String> = [
            ["rabbit", "front", "", "#f00", "", "", "", "", "", "", ""],
            ["", "", "", "", "path", "", "here", "", "", "", ""],
            ["", "", "", "", "", "", "", "", "", "", "mountains"],
        ]
        .iter()
        .flatten()
        .map(|s| s.to_string())
        .collect();
        let ops = decode_ops(&num, &strs);
        assert_eq!(
            ops,
            vec![
                EOp::Add { kind: "rabbit".into(), at: "front".into(), pos: None, dist: None, count: 5.0, scale: None, rot: 0.0, color: Some("#f00".into()) },
                EOp::AddPath { material: "path".into(), from: "here".into(), to: String::new(), from_pos: None, to_pos: Some([1.0, 0.0, 2.0]), width: 4.0 },
                EOp::SetTerrain { preset: "mountains".into(), amplitude: Some(7.5) },
            ]
        );
    }
}
