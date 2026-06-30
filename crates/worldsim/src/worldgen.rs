//! Procedural WORLD GENERATORS — the deterministic "make city / forest / lake" commands and the settlement
//! planner. Ported from the old JS (src/lib/city.ts, src/lib/settlementPlanner.ts) so Rust owns ALL world-gen
//! compute (the "Rust owns all compute" north star); JS now only matches the command word + renders. Determinism
//! is preserved exactly — the same PRNG (mulberry32) and the same GLSL-style hash — and parity with the former JS
//! is pinned by tests (src/lib/worldgen.test.ts) so a port can't silently change a generated city/town.

use crate::engine::{in_natural_pond, in_water_seeded};
use crate::structstore::{is_building as sk_is_building, is_home as sk_is_home, is_walled as sk_is_walled, kind_code as sk_code, Structure, StructureStore, SK_CABIN, SK_FENCE, SK_GRAVE, SK_HOUSE, SK_LAMP, SK_PINE, SK_ROCK, SK_TOWER, SK_TREE, SK_WELL};

/// Parse a flat `[px, pz, size, seed]×n` water-zone buffer (the binary worldgen ABI form) → the tuples
/// `in_water_seeded` wants. Zones cross as a small typed array instead of JSON, computed once per frame JS-side.
fn zones_seeded(flat: &[f64]) -> Vec<(f64, f64, f64, f64)> {
    flat.chunks_exact(4).map(|c| (c[0], c[1], c[2], c[3])).collect()
}

// Op-stream lane 0 codes for the binary worldgen ABI (Float64Array, stride 9): [op, kind, x, z, rot, sx, sy, sz, color].
const OP_ADD: f64 = 0.0;
#[allow(dead_code)]
const OP_REMOVE: f64 = 1.0; // lane1 = slot (used by ops that demolish, e.g. settlement_ops fence diff) — added with those ports

// ── shared helpers ────────────────────────────────────────────────────────────────────────────────────────────
/// The GLSL-style hash used by the city/forest generators — `fract(sin(i*12.9898+4.13)*43758.5453)`, byte-for-byte
/// the JS `hash1`. f64 sin matches across the boundary to ~1e-12; the worldgen parity tests pin the result.
fn hash1(i: f64) -> f64 {
    let v = (i * 12.9898 + 4.13).sin() * 43758.5453;
    v - v.floor()
}

// ── mulberry32 — a tiny deterministic PRNG, BYTE-IDENTICAL to the JS `prng()` in settlementPlanner.ts ──────────
// JS used signed `|0` adds + `Math.imul` + unsigned `>>>` shifts; u32 wrapping arithmetic reproduces every bit.
struct Mulberry32 {
    a: u32,
}
impl Mulberry32 {
    fn new(seed: u32) -> Self {
        Mulberry32 { a: seed }
    }
    fn next(&mut self) -> f64 {
        self.a = self.a.wrapping_add(0x6d2b_79f5); // (a + 0x6d2b79f5) | 0
        let mut t = (self.a ^ (self.a >> 15)).wrapping_mul(1 | self.a); // Math.imul(a ^ (a>>>15), 1|a)
        t = t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t)) ^ t;
        ((t ^ (t >> 14)) as f64) / 4_294_967_296.0 // ((t ^ (t>>>14)) >>> 0) / 2^32
    }
}

struct Tier {
    blocks: i32,
    houses: i32,
    towers: i32,
    fenced: bool,
}
fn tier(size: &str) -> Tier {
    match size {
        "hamlet" => Tier { blocks: 1, houses: 4, towers: 0, fenced: false },
        "village" => Tier { blocks: 2, houses: 10, towers: 1, fenced: false },
        "town" => Tier { blocks: 3, houses: 20, towers: 1, fenced: true },
        _ => Tier { blocks: 4, houses: 34, towers: 2, fenced: true }, // "city" (and any unknown → biggest)
    }
}

const GAP: f64 = 18.0; // metres between parallel streets (one block)
const SETBACK: f64 = 6.0; // how far houses sit back from the road they face
const HOUSE_SPACING: f64 = 7.5; // spacing of houses along a street

fn is_home(k: &str) -> bool {
    k == "house" || k == "cabin" || k == "manor"
}
fn is_walled(k: &str) -> bool {
    is_home(k) || k == "tower" || k == "well"
}
fn uf_find(parent: &mut [usize], i: usize) -> usize {
    let mut r = i;
    while parent[r] != r {
        r = parent[r];
    }
    let mut c = i;
    while parent[c] != c {
        let nx = parent[c];
        parent[c] = r;
        c = nx;
    }
    r
}

/// Shared perimeter-wall GEOMETRY — used by BOTH `settlement_ops` (JSON) and `settlement_ops_store` (binary), so the
/// ring fit is identical and the JSON path's tests keep guarding it. Clusters `homes` (GATHER=60), supports a per-town
/// jittered ring (anchored to the cluster's stable min-home, not the drifting centroid — docs/spread-redesign.md P2),
/// tiles each edge into abutting panels. Returns the desired panels `[(x, z, rot_deg, scale_x)]` + the indices into
/// `rocks` a panel demolished. `water(x,z)` is the zone-shape test (`in_water` JSON or `in_water_seeded` binary).
// `near` = positions of structures that CHANGED this call ([(x,z)…]). A cluster is re-fitted only if a change is near
// it — every OTHER town's wall is left exactly as the store holds it (no re-fit off a partial/streamed home set → the
// "fence moves randomly while I fly over" bug + its jank). `near` EMPTY = fit every cluster (the one-time load reconcile
// + the JSON parity path). Returns the usual (desired panels, demolished rocks) PLUS the fitted clusters' (cx,cz,keep_r)
// so the caller knows which existing fences belong to a fitted town (and may be diffed) vs an untouched one (kept).
fn fit_walls(homes: &[(f64, f64)], walled: &[(f64, f64, bool)], rocks: &[(f64, f64)], water: &dyn Fn(f64, f64) -> bool, near: &[(f64, f64)]) -> (Vec<(f64, f64, f64, f64)>, Vec<usize>, Vec<bool>) {
    const GATHER: f64 = 60.0;
    const SEG: f64 = 6.5;
    const K: usize = 22;
    const MARGIN: f64 = 8.0;
    let mut desired: Vec<(f64, f64, f64, f64)> = Vec::new();
    let mut demolish: Vec<usize> = Vec::new();
    let n = homes.len();
    let mut fitted: Vec<bool> = vec![false; n]; // homes[i]'s cluster was (re)fitted this call → its OLD panels may be replaced
    if n == 0 {
        return (desired, demolish, fitted);
    }
    let mut parent: Vec<usize> = (0..n).collect();
    for i in 0..n {
        for j in (i + 1)..n {
            if (homes[i].0 - homes[j].0).hypot(homes[i].1 - homes[j].1) < GATHER {
                let ri = uf_find(&mut parent, i);
                let rj = uf_find(&mut parent, j);
                if ri != rj {
                    parent[ri] = rj;
                }
            }
        }
    }
    let roots: Vec<usize> = (0..n).map(|i| uf_find(&mut parent, i)).collect();
    let mut seen: Vec<usize> = Vec::new();
    for &r in &roots {
        if !seen.contains(&r) {
            seen.push(r);
        }
    }
    let centroids: Vec<(f64, f64)> = seen
        .iter()
        .map(|&root| {
            let mem: Vec<usize> = (0..n).filter(|&i| roots[i] == root).collect();
            let m = mem.len() as f64;
            (mem.iter().map(|&i| homes[i].0).sum::<f64>() / m, mem.iter().map(|&i| homes[i].1).sum::<f64>() / m)
        })
        .collect();
    for (ci, &root) in seen.iter().enumerate() {
        let (cx, cz) = centroids[ci];
        // LOCAL fit: skip a cluster with no change near it → its stored wall is left exactly as-is (never re-fit off a
        // partial/streamed home set). `near` empty → fit every cluster (load reconcile + parity path).
        if !near.is_empty() && !near.iter().any(|&(px, pz)| (cx - px).hypot(cz - pz) < GATHER + 30.0) {
            continue;
        }
        for i in 0..n {
            if roots[i] == root {
                fitted[i] = true; // this cluster IS being (re)fitted → its homes own the panels eligible for replacement
            }
        }
        let anchor = (0..n).filter(|&i| roots[i] == root).map(|i| homes[i]).fold((f64::INFINITY, f64::INFINITY), |a, h| if h < a { h } else { a });
        let mut pts: Vec<(f64, f64)> = (0..n).filter(|&i| roots[i] == root).map(|i| (homes[i].0 - cx, homes[i].1 - cz)).collect();
        for &(wx, wz, is_h) in walled {
            if is_h {
                continue;
            }
            let mut nearest = 0usize;
            let mut nd = f64::INFINITY;
            for (kk, &(kx, kz)) in centroids.iter().enumerate() {
                let d = (wx - kx).hypot(wz - kz);
                if d < nd {
                    nd = d;
                    nearest = kk;
                }
            }
            if nearest == ci && nd < 90.0 {
                pts.push((wx - cx, wz - cz));
            }
        }
        let mut ring: Vec<(f64, f64)> = Vec::with_capacity(K);
        for k in 0..K {
            let th = (k as f64 / K as f64) * TAU;
            let (dx, dz) = (th.cos(), th.sin());
            let mut sup = 0.0f64;
            for &(px2, pz2) in &pts {
                sup = sup.max(px2 * dx + pz2 * dz);
            }
            let jit = (hash1(anchor.0 * 53.0 + th * 97.0 + anchor.1 * 31.0) - 0.5) * 7.0;
            let mut r = (sup + MARGIN + jit).clamp(7.0, 400.0);
            if water(cx + dx * r, cz + dz * r) {
                for s in 1..=16 {
                    let nr = (r - s as f64 * 2.0).max(4.0);
                    if !water(cx + dx * nr, cz + dz * nr) {
                        r = nr;
                        break;
                    }
                }
            }
            ring.push((cx + dx * r, cz + dz * r));
        }
        for k in 0..K {
            let (ax, az) = ring[k];
            let (bx, bz) = ring[(k + 1) % K];
            let elen = (bx - ax).hypot(bz - az);
            let np = ((elen / SEG).ceil() as usize).clamp(1, 256);
            let edge_ang = (bz - az).atan2(bx - ax);
            let sx = (elen / np as f64) / 1.4;
            for jp in 0..np {
                let t = (jp as f64 + 0.5) / np as f64;
                let fx = ax + (bx - ax) * t;
                let fz = az + (bz - az) * t;
                if water(fx, fz) {
                    continue;
                }
                for (ri, &(rx, rz)) in rocks.iter().enumerate() {
                    if (rx - fx).hypot(rz - fz) < 3.0 && !demolish.contains(&ri) {
                        demolish.push(ri);
                    }
                }
                let rot = -edge_ang.to_degrees() + (hash1(fx + fz) - 0.5) * 10.0;
                desired.push((fx, fz, rot, sx));
            }
        }
    }
    (desired, demolish, fitted)
}

/// BINARY settlement_ops against the StructureStore (no JSON). Reads homes/towers/wells/rocks/fences from the store,
/// (re)fits the walls via `fit_walls`, and diffs against the store's EXISTING fence panels — emitting OP_REMOVE by SLOT
/// for stale panels + demolished rocks and OP_ADD for new panels, self-mutating the store. The position-diff state (the
/// existing panels) now LIVES in the store, so the per-build payload is O(local change), not the whole world-wide fence
/// set (today's heaviest stringify). Wall geometry is byte-parity with `settlement_ops` (same `fit_walls`).
pub fn settlement_ops_store(store: &mut StructureStore, zones: &[f64], changed: &[f64]) -> Vec<f64> {
    let zs = zones_seeded(zones);
    let near: Vec<(f64, f64)> = changed.chunks_exact(2).map(|c| (c[0], c[1])).collect(); // structures changed this call → fit only their towns
    let mut walled: Vec<(f64, f64, bool)> = Vec::new();
    let mut rock_slot: Vec<(u32, f64, f64)> = Vec::new();
    let mut existing: Vec<(u32, f64, f64)> = Vec::new(); // non-keep fence: slot, x, z
    for s in store.live_slots() {
        if let Some(st) = store.get(s) {
            if !st.x.is_finite() || !st.z.is_finite() || st.x.abs() > 1.0e6 || st.z.abs() > 1.0e6 {
                continue;
            }
            if sk_is_walled(st.kind) {
                walled.push((st.x, st.z, sk_is_home(st.kind)));
            } else if st.kind == SK_ROCK && !st.keep {
                rock_slot.push((s, st.x, st.z));
            } else if st.kind == SK_FENCE && !st.keep {
                existing.push((s, st.x, st.z));
            }
        }
    }
    let homes: Vec<(f64, f64)> = walled.iter().filter(|w| w.2).map(|w| (w.0, w.1)).collect();
    if homes.is_empty() {
        return Vec::new();
    }
    let rock_pos: Vec<(f64, f64)> = rock_slot.iter().map(|r| (r.1, r.2)).collect();
    let (desired, demolish_idx, fitted) = fit_walls(&homes, &walled, &rock_pos, &|x, z| in_water_seeded(&zs, x, z) || in_natural_pond(x, z), &near);
    // OWNERSHIP by NEAREST HOME (not a radius from the centroid): a fence panel belongs to the town of its closest home.
    // If that town was (re)fitted this call, the panel is a CANDIDATE for replacement (removed below unless a new desired
    // panel covers it) — so a SHRUNK/SHIFTED ring's old far panels are reclaimed, not stranded as a phantom 2nd layer
    // (the "multi-layer fence / fence spur extending out" bug). A radius-from-centroid keep zone missed them because the
    // centroid moves when the town grows/decays asymmetrically. A panel whose nearest home is in an UNTOUCHED town, or a
    // true orphan far (>110 m) from every home, is kept verbatim — preserving the "far town's wall stays put" fix.
    const OWN_MAX: f64 = 110.0;
    let mut keep_e = vec![false; existing.len()];
    for (ei, e) in existing.iter().enumerate() {
        let mut nd = f64::INFINITY;
        let mut owner_fitted = false;
        for (hi, &(hx, hz)) in homes.iter().enumerate() {
            let d = (e.1 - hx).hypot(e.2 - hz);
            if d < nd {
                nd = d;
                owner_fitted = fitted[hi];
            }
        }
        if !owner_fitted || nd > OWN_MAX {
            keep_e[ei] = true;
        }
    }
    let mut covered = vec![false; desired.len()];
    for (di, d) in desired.iter().enumerate() {
        for (ei, e) in existing.iter().enumerate() {
            if (d.0 - e.1).hypot(d.1 - e.2) < 0.5 {
                keep_e[ei] = true;
                covered[di] = true;
            }
        }
    }
    let mut out: Vec<f64> = Vec::new();
    for (ei, e) in existing.iter().enumerate() {
        if !keep_e[ei] {
            store.remove(e.0);
            out.extend_from_slice(&[OP_REMOVE, e.0 as f64, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        }
    }
    for &ri in &demolish_idx {
        let slot = rock_slot[ri].0;
        store.remove(slot);
        out.extend_from_slice(&[OP_REMOVE, slot as f64, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    }
    for (di, d) in desired.iter().enumerate() {
        if !covered[di] {
            store.add(Structure { kind: SK_FENCE, x: d.0, z: d.1, rot: d.2, sx: d.3, sy: 1.0, sz: 1.0, color: 0, keep: false, region: 0 });
            out.extend_from_slice(&[OP_ADD, SK_FENCE as f64, d.0, d.1, d.2, d.3, 1.0, 1.0, 0.0]);
        }
    }
    out
}

/// BINARY grave_site against the StructureStore (no JSON, O(local)). `zones` = `[px,pz,size,seed]×m`. Returns the
/// grave plot as `[x, z]` (empty = no grave: died in the wild / town ringed by water). Byte-parity with `grave_site`.
pub fn grave_site_store(store: &StructureStore, dx: f64, dz: f64, zones: &[f64]) -> Vec<f64> {
    const MEMBER_R2: f64 = 70.0 * 70.0;
    // nearest building within 70 m → did the deceased belong to a settlement?
    let mut bx = 0.0;
    let mut bz = 0.0;
    let mut best = MEMBER_R2;
    for s in store.query_radius(dx, dz, 70.0) {
        if let Some(st) = store.get(s) {
            if sk_is_building(st.kind) {
                let d2 = (st.x - dx).powi(2) + (st.z - dz).powi(2);
                if d2 < best {
                    best = d2;
                    bx = st.x;
                    bz = st.z;
                }
            }
        }
    }
    if best >= MEMBER_R2 {
        return Vec::new(); // died in the wild → no grave
    }
    // settlement cluster around the nearest building (within 90 m) → centroid + extent (plot sits OUTSIDE the wall)
    let cluster: Vec<(f64, f64)> = store
        .query_radius(bx, bz, 90.0)
        .into_iter()
        .filter_map(|s| store.get(s))
        .filter(|st| sk_is_building(st.kind) && (st.x - bx).hypot(st.z - bz) < 90.0)
        .map(|st| (st.x, st.z))
        .collect();
    let m = cluster.len() as f64;
    let cx = cluster.iter().map(|p| p.0).sum::<f64>() / m;
    let cz = cluster.iter().map(|p| p.1).sum::<f64>() / m;
    let mut rad = 0.0f64;
    for &(x, z) in &cluster {
        rad = rad.max((x - cx).hypot(z - cz));
    }
    let base = (((cx / 80.0).round() * 12.9898 + (cz / 80.0).round() * 78.233).sin()).abs() * TAU;
    let zs = zones_seeded(zones);
    let r = rad + 18.0;
    for t in 0..8 {
        let a = base + (t as f64) * (TAU / 8.0);
        let gx = cx + a.cos() * r + (hash1(cx + t as f64) - 0.5) * 7.0;
        let gz = cz + a.sin() * r + (hash1(cz + t as f64 + 5.0) - 0.5) * 7.0;
        if !in_water_seeded(&zs, gx, gz) {
            return vec![gx, gz];
        }
    }
    Vec::new() // every direction wet → no grave this death
}

/// BINARY build_ops against the StructureStore (no JSON, O(local)). `builds` = `[x,z]×n` settler positions; `zones`
/// = `[px,pz,size,seed]×m`. Returns the add-op stream `[OP_ADD, kind, x, z, rot, sx, sy, sz, color]×k` AND self-adds
/// each placed home (so the batch respects itself). Byte-parity with `build_ops` — pinned by `build_ops_store_matches_json`.
/// The colony/FOUND_GAP scans read the store's local neighbourhood + a bounded global house count, never the world.
pub fn build_ops_store(store: &mut StructureStore, builds: &[f64], zones: &[f64]) -> Vec<f64> {
    const COLONY_R2: f64 = 75.0 * 75.0;
    const COLONY_MAX: usize = 10;
    let new_gap2 = (crate::world::FOUND_GAP * 0.83).powi(2);
    const HOUSE_CAP: usize = 200;
    const SENSE: f64 = 200.0; // > sqrt(new_gap2) so the local query covers every building that affects the gap check
    const FOOTPRINT_R2: f64 = 90.0 * 90.0; // gather the local cluster (centroid + spread) so infill clamps INSIDE it
    const GROW: f64 = 8.0; // max a town's footprint may creep per infill build → the wall edges out a ring at a time,
    // never jumps to wrap a house dropped 10–70 m outside it (user: "houses just outside, the fence balloons")
    let zs = zones_seeded(zones);
    let mut out: Vec<f64> = Vec::new();
    let mut house_total = store.live_slots().into_iter().filter(|&s| store.get(s).map_or(false, |st| sk_is_building(st.kind))).count();
    for b in builds.chunks_exact(2) {
        if house_total >= HOUSE_CAP {
            break;
        }
        let mut gx = (b[0] / 8.0).round() * 8.0;
        let mut gz = (b[1] / 8.0).round() * 8.0;
        let mut near_in_colony = 0usize;
        let mut nearest2 = f64::INFINITY;
        let mut local: Vec<(f64, f64)> = Vec::new(); // buildings within FOOTPRINT_R → this town's centroid + spread
        for s in store.query_radius(gx, gz, SENSE) {
            if let Some(st) = store.get(s) {
                if !sk_is_building(st.kind) {
                    continue;
                }
                let d2 = (st.x - gx).powi(2) + (st.z - gz).powi(2);
                if d2 < COLONY_R2 {
                    near_in_colony += 1;
                }
                if d2 < nearest2 {
                    nearest2 = d2;
                }
                if d2 < FOOTPRINT_R2 {
                    local.push((st.x, st.z));
                }
            }
        }
        // EFFECTIVE per-colony cap — a SMALL village (user: "smaller number of houses together + more spread out"). A
        // colony fills to ~this many homes then SPILLS its surplus into NEW towns (the spread), so the world reads as
        // many small hamlets scattered wide, not a few fat blobs. A real stop (not removed); bounded by HOUSE_CAP.
        let colony_cap = (COLONY_MAX + 2).min(HOUSE_CAP); // 12
        if near_in_colony >= colony_cap {
            continue; // this colony is full → no more infill here (its surplus founds NEW towns elsewhere)
        }
        if near_in_colony == 0 && nearest2 < new_gap2 {
            continue; // a brand-new town must be founded clear of any building (≥ the found gap)
        }
        // INFILL near an existing town → pull the build INSIDE its footprint (fills gaps; the ring edges out ≤ GROW per
        // build, never leaps to wrap a house dropped just outside it). A FOUND build keeps its requested open spot.
        if near_in_colony >= 1 && !local.is_empty() {
            let m = local.len() as f64;
            let (cx, cz) = (local.iter().map(|p| p.0).sum::<f64>() / m, local.iter().map(|p| p.1).sum::<f64>() / m);
            let max_r = local.iter().map(|&(x, z)| (x - cx).hypot(z - cz)).fold(0.0f64, f64::max);
            let d = (gx - cx).hypot(gz - cz);
            let limit = max_r + GROW;
            if d > limit && d > 1.0e-6 {
                gx = ((cx + (gx - cx) / d * limit) / 8.0).round() * 8.0;
                gz = ((cz + (gz - cz) / d * limit) / 8.0).round() * 8.0;
            }
        }
        // occupancy is checked at the FINAL (clamped) cell — an occupied interior spot is skipped (the sim retries),
        // never bumped to an awkward outside slot.
        if store.query_radius(gx, gz, 6.0).into_iter().any(|s| store.get(s).map_or(false, |st| sk_is_building(st.kind) && (st.x - gx).powi(2) + (st.z - gz).powi(2) < 36.0)) {
            continue;
        }
        let grave_clash = store.query_radius(gx, gz, 8.0).into_iter().any(|s| store.get(s).map_or(false, |st| st.kind == SK_GRAVE && (st.x - gx).powi(2) + (st.z - gz).powi(2) < 64.0));
        if grave_clash {
            continue;
        }
        // reject building ON or STRADDLING water — test the CENTRE + an 8-point ring at the house footprint, against
        // BOTH zone water AND procedural NATURAL ponds. The old check tested only zone water (+ a 4-point cross), so a
        // home straddled a natural pond's bank while the fence — which already tests both (see fit_walls) — correctly
        // stayed clear → "a house built half inside the pond". The same `wet` test now keeps homes (and the growing
        // town) off the water, so a settlement stops at the bank instead of engulfing the pond.
        let wet = |x: f64, z: f64| in_water_seeded(&zs, x, z) || in_natural_pond(x, z);
        let fr = 6.0; // footprint radius + a clear bank margin (homes sit on the ~8 m grid)
        if wet(gx, gz) || (0..8).any(|k| { let a = k as f64 / 8.0 * std::f64::consts::TAU; wet(gx + a.cos() * fr, gz + a.sin() * fr) }) {
            continue;
        }
        let roll = hash1(gx * 1.7 + gz * 0.31);
        let (kc, s) = if roll < 0.3 {
            (SK_CABIN, 0.85 + hash1(gx + 11.0) * 0.25)
        } else if roll < 0.8 {
            (SK_HOUSE, 0.9 + hash1(gz + 17.0) * 0.35)
        } else {
            (SK_HOUSE, 1.35 + hash1(gx + gz + 23.0) * 0.4)
        };
        store.add(Structure { kind: kc, x: gx, z: gz, rot: 0.0, sx: s, sy: s, sz: s, color: 0, keep: false, region: 0 });
        house_total += 1;
        out.extend_from_slice(&[OP_ADD, kc as f64, gx, gz, 0.0, s, s, s, 0.0]);
    }
    out
}

/// DORMANT SETTLEMENT GROWTH (self-sustaining world). A far settlement the player isn't standing in should still
/// DEVELOP over time — gain new homes, not just relax its population toward a FIXED seeded house count (which made
/// only the live town grow while every other one plateaued). Given the cluster's existing house positions `houses`
/// (`[x,z]×n`), this generates up to `want` build requests around the cluster centroid and runs them through the
/// SAME live placement as a real settler (`build_ops_store`: water-safe, non-overlapping, footprint-clamped, colony-
/// capped at ~30) via a THROWAWAY store seeded with the existing homes. So a dormant town builds out by the identical
/// rules as a live one, just driven by the closed-form fast-forward instead of ticking settlers. `zones` = water
/// zones `[px,pz,size,seed]×m`. Returns the new homes as the standard build op stream `[OP_ADD, kind, x, z, rot,
/// sx, sy, sz, color]×k` (k ≤ want; JS already decodes this format). Deterministic in (cluster, seed). Returns empty
/// once the colony is at its cap — growth STOPS at city size on its own (no runaway).
pub fn grow_dormant_houses(houses: &[f64], want: usize, zones: &[f64], seed: f64) -> Vec<f64> {
    if houses.len() < 2 || want == 0 {
        return Vec::new();
    }
    let n = (houses.len() / 2) as f64;
    let (mut cx, mut cz) = (0.0, 0.0);
    for c in houses.chunks_exact(2) {
        cx += c[0];
        cz += c[1];
    }
    cx /= n;
    cz /= n;
    let mut max_r = 0.0f64;
    for c in houses.chunks_exact(2) {
        max_r = max_r.max((c[0] - cx).hypot(c[1] - cz));
    }
    // seed a throwaway store with the existing homes so build_ops_store sees a COLONY (→ infill) not bare ground (→
    // it would reject each request under the found-new-town gap). build_ops_store self-adds what it places, so the
    // batch also dedups against itself; the returned ops are ONLY the newly placed homes (the seeds aren't re-emitted).
    let mut store = StructureStore::new();
    for c in houses.chunks_exact(2) {
        store.add(Structure { kind: SK_HOUSE, x: c[0], z: c[1], rot: 0.0, sx: 1.0, sy: 1.0, sz: 1.0, color: 0, keep: true, region: 0 });
    }
    // build requests: a golden-angle spiral around the footprint (jittered by `seed` so successive pulses don't
    // restack one spot); build_ops_store clamps each INSIDE footprint+GROW, so the town thickens then edges outward.
    let ga = std::f64::consts::PI * (3.0 - 5.0_f64.sqrt());
    let mut reqs: Vec<f64> = Vec::with_capacity(want * 2);
    for i in 0..want {
        let a = i as f64 * ga + seed * std::f64::consts::TAU;
        let r = (max_r * 0.6 + (i as f64 + 1.0).sqrt() * 6.0).min(60.0);
        reqs.push(cx + a.cos() * r);
        reqs.push(cz + a.sin() * r);
    }
    build_ops_store(&mut store, &reqs, zones)
}

/// BINARY well_ops against the StructureStore (no JSON, O(local)). `reqs` = `[x,z]×n` settler well requests; `zones`
/// = `[px,pz,size,seed]×m`. Returns the add-op stream `[OP_ADD, kind, x, z, rot, sx, sy, sz, color]×k` AND self-adds
/// each placed well to the store (so the batch dedups against itself and future calls see it). Byte-parity with
/// `well_ops` above — pinned by `well_ops_store_matches_json`. Dedup queries the store's 35 m neighbourhood, not the
/// whole world, so cost is independent of total well count over a days-long world.
pub fn well_ops_store(store: &mut StructureStore, reqs: &[f64], zones: &[f64]) -> Vec<f64> {
    let z = zones_seeded(zones);
    let mut out: Vec<f64> = Vec::new();
    for w in reqs.chunks_exact(2) {
        let gx = (w[0] / 4.0).round() * 4.0;
        let gz = (w[1] / 4.0).round() * 4.0;
        if in_water_seeded(&z, gx, gz) {
            continue; // a well in a lake is pointless
        }
        // dedup: an existing well within 35 m already serves this spot (incl. ones placed earlier this batch)
        let dup = store.query_radius(gx, gz, 35.0).into_iter().any(|s| {
            store.get(s).map_or(false, |st| st.kind == SK_WELL && (st.x - gx).powi(2) + (st.z - gz).powi(2) < 35.0 * 35.0)
        });
        if dup {
            continue;
        }
        store.add(Structure { kind: SK_WELL, x: gx, z: gz, rot: 0.0, sx: 1.0, sy: 1.0, sz: 1.0, color: 0, keep: true, region: 0 });
        out.extend_from_slice(&[OP_ADD, SK_WELL as f64, gx, gz, 0.0, 1.0, 1.0, 1.0, 0.0]);
    }
    out
}

/// BINARY vegetation_ops against the StructureStore (no JSON). At most ONE add-tree op `[OP_ADD, SK_TREE, x, z, 0,
/// s, s, s, 0]` (empty = none). `zones` = `[px,pz,size,seed]×m`. NOTE: unlike the other ports this is NOT byte-
/// identical to the JSON path — the JSON path selected the home to plant beside by indexing `world.objects` in its
/// incidental insertion order; the store has no such order, so we sort homes by (x,z) for a STABLE, deterministic
/// selection. Cosmetic only (which home gets a tree); the town still greens deterministically.
pub fn vegetation_ops_store(store: &mut StructureStore, seed: f64, zones: &[f64]) -> Vec<f64> {
    let zs = zones_seeded(zones);
    let mut blds: Vec<(f64, f64)> = Vec::new();
    let mut blockers: Vec<(f64, f64)> = Vec::new(); // buildings + trees/pines (don't plant on top)
    let mut trees: Vec<(f64, f64)> = Vec::new();
    for s in store.live_slots() {
        if let Some(st) = store.get(s) {
            match st.kind {
                SK_HOUSE | SK_CABIN | SK_TOWER => {
                    blds.push((st.x, st.z));
                    blockers.push((st.x, st.z));
                }
                SK_TREE => {
                    blockers.push((st.x, st.z));
                    trees.push((st.x, st.z));
                }
                SK_PINE => blockers.push((st.x, st.z)),
                _ => {}
            }
        }
    }
    blds.sort_by(|a, b| a.partial_cmp(b).unwrap()); // STABLE selection order (the store has no insertion order)
    let house_count = blds.len();
    let colony_trees = trees.iter().filter(|&&(tx, tz)| blds.iter().any(|&(bx, bz)| (bx - tx).hypot(bz - tz) < 18.0)).count();
    if house_count < 3 || colony_trees >= (house_count as f64 * 1.3) as usize || hash1(seed) >= 0.55 {
        return Vec::new();
    }
    let h = blds[(hash1(seed + 1.0) * house_count as f64) as usize % house_count];
    let ta = hash1(seed + 2.0) * TAU;
    let tr = 4.5 + hash1(seed + 3.0) * 6.0;
    let tx = h.0 + ta.cos() * tr;
    let tz = h.1 + ta.sin() * tr;
    if in_water_seeded(&zs, tx, tz) || blockers.iter().any(|&(ox, oz)| (ox - tx).abs() < 2.6 && (oz - tz).abs() < 2.6) {
        return Vec::new();
    }
    let s = 0.7 + hash1(seed + 4.0) * 0.5;
    store.add(Structure { kind: SK_TREE, x: tx, z: tz, rot: 0.0, sx: s, sy: s, sz: s, color: 0, keep: false, region: 0 });
    vec![OP_ADD, SK_TREE as f64, tx, tz, 0.0, s, s, s, 0.0]
}

// ── FOREST — plant/grow a wood ahead of the player (faithful port of city.ts forestOps) ──────────────────────
const FOREST_KINDS: [&str; 3] = ["tree", "tree", "pine"];
fn is_tree(k: &str) -> bool {
    k == "tree" || k == "pine"
}

// ── LAKE — dig/enlarge a pond ahead of the player (faithful port of city.ts lakeOps) ──────────────────────────

// ── Generator BINARY op stream (docs/world-data-architecture.md — the jzon-drop) ──────────────────────────────────
// The generators (forest/lake/city) read a binary world snapshot + emit this flat tagged f64 stream INSTEAD of a jzon
// ops array; JS decodes it back into the SAME engine Op[] (then fed to applyOps), so behaviour is parity-identical with
// no JSON crossing the boundary. No string table needed — every field encodes as a number: kindCode (structpack), the
// packed color u32, the material/shape codes below, and a REMOVE references its target zone/object by SLOT (its index
// in the binary snapshot JS supplied), which JS maps back to the string id. Stride 10; meaning of lanes is per-op-type.
pub const GEN_STRIDE: usize = 10;
pub const GOP_ADD: f64 = 0.0; // [0, kindCode, x, z, sx, sy, sz, rot, colorU32, _]
pub const GOP_ADDZONE: f64 = 1.0; // [1, x, z, size, materialCode, shapeCode, _, _, _, _]
pub const GOP_REMOVE: f64 = 2.0; // [2, slot, _, _, _, _, _, _, _, _]
pub const GOP_ADDPATH: f64 = 3.0; // [3, fromX, fromZ, toX, toZ, width, materialCode, _, _, _]
pub const MAT_WATER: f64 = 0.0;
pub const MAT_PATH: f64 = 1.0;
pub const SHAPE_BLOB: f64 = 0.0;

fn gop_add(out: &mut Vec<f64>, kind: u8, x: f64, z: f64, s: [f64; 3], rot: f64, color: u32) {
    out.extend_from_slice(&[GOP_ADD, kind as f64, x, z, s[0], s[1], s[2], rot, color as f64, 0.0]);
}
fn gop_addzone(out: &mut Vec<f64>, x: f64, z: f64, size: f64, mat: f64, shape: f64) {
    out.extend_from_slice(&[GOP_ADDZONE, x, z, size, mat, shape, 0.0, 0.0, 0.0, 0.0]);
}
fn gop_remove(out: &mut Vec<f64>, slot: usize) {
    out.extend_from_slice(&[GOP_REMOVE, slot as f64, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
}
fn gop_addpath(out: &mut Vec<f64>, from: (f64, f64), to: (f64, f64), width: f64, mat: f64) {
    out.extend_from_slice(&[GOP_ADDPATH, from.0, from.1, to.0, to.1, width, mat, 0.0, 0.0, 0.0]);
}

/// LAKE (binary) — `zones` = WATER zones `[px,pz,size,seed]×n` (JS pre-filters to water; the seed lane is unused here).
/// Same logic as `lake_ops`; a REMOVE references the chosen zone by its SLOT (index in `zones`), which JS maps to its id.
pub fn lake_ops_bin(zones: &[f64], px: f64, pz: f64, yaw: f64) -> Vec<f64> {
    let (fx, fz) = (yaw.sin(), -yaw.cos());
    let (tx, tz) = (px + fx * 18.0, pz + fz * 18.0);
    let mut best: Option<(usize, f64, f64, f64)> = None; // (slot, x, z, size)
    let mut bd = f64::INFINITY;
    for (slot, z) in zones.chunks_exact(4).enumerate() {
        let (zx, zz, size) = (z[0], z[1], z[2]);
        let d = (zx - tx).hypot(zz - tz);
        if d < size + 16.0 && d < bd {
            bd = d;
            best = Some((slot, zx, zz, size));
        }
    }
    let mut out = Vec::new();
    if let Some((slot, zx, zz, size)) = best {
        gop_remove(&mut out, slot);
        gop_addzone(&mut out, zx, zz, size + 6.0, MAT_WATER, SHAPE_BLOB);
    } else {
        gop_addzone(&mut out, tx, tz, 13.0, MAT_WATER, SHAPE_BLOB);
    }
    out
}

/// FOREST (binary) — reads trees/pines from the store, water from `zones` (`[px,pz,size,seed]×n`). Same geometry as
/// `forest_ops`; emits GOP_ADD tree ops at pre-placed spots (applyOps still collision-resolves them after JS decode).
pub fn forest_ops_bin(store: &StructureStore, zones: &[f64], px: f64, pz: f64, yaw: f64) -> Vec<f64> {
    let (fx, fz) = (yaw.sin(), -yaw.cos());
    let (tx, tz) = (px + fx * 14.0, pz + fz * 14.0);
    let near: Vec<(f64, f64)> = store
        .live_slots()
        .iter()
        .filter_map(|&s| store.get(s))
        .filter(|st| st.kind == SK_TREE || st.kind == SK_PINE)
        .map(|st| (st.x, st.z))
        .filter(|&(x, z)| (x - tx).hypot(z - tz) < 40.0)
        .collect();
    let (cx, cz) = if near.is_empty() {
        (tx, tz)
    } else {
        let n = near.len() as f64;
        (near.iter().map(|p| p.0).sum::<f64>() / n, near.iter().map(|p| p.1).sum::<f64>() / n)
    };
    let mut inner_r = 0.0f64;
    for &(x, z) in &near {
        inner_r = inner_r.max((x - cx).hypot(z - cz));
    }
    let outer_r = inner_r + if near.is_empty() { 14.0 } else { 16.0 };
    let area = std::f64::consts::PI * (outer_r * outer_r - inner_r * inner_r);
    let count = (area / 16.0).round().clamp(8.0, 32.0) as i32;
    let ga = std::f64::consts::PI * (3.0 - 5.0f64.sqrt()); // golden angle
    let zs = zones_seeded(zones);
    let mut out = Vec::new();
    for i in 0..count {
        let t = (i as f64 + 0.5) / count as f64;
        let r = (inner_r * inner_r + t * (outer_r * outer_r - inner_r * inner_r)).sqrt();
        let a = i as f64 * ga + hash1(i as f64) * 0.6;
        let jr = 1.0 + (hash1(i as f64 + 99.0) - 0.5) * 4.0;
        let x = cx + a.cos() * (r + jr);
        let z = cz + a.sin() * (r + jr);
        if in_water_seeded(&zs, x, z) {
            continue; // trees don't grow in the lake
        }
        let kind = if FOREST_KINDS[(hash1(i as f64 + 7.0) * FOREST_KINDS.len() as f64) as usize] == "pine" { SK_PINE } else { SK_TREE };
        let s = 0.8 + hash1(i as f64 + 31.0) * 0.7;
        gop_add(&mut out, kind, x, z, [s, s, s], hash1(i as f64 + 51.0) * 360.0, 0);
    }
    out
}

const MAT_PLAZA: f64 = 2.0; // GEN_MATERIAL index (must match structpack.ts GEN_MATERIAL)
const SHAPE_RECT: f64 = 1.0; // GEN_SHAPE index

/// "#rrggbb" → packed u32 (so a block's wall tone crosses the boundary as a number, not a string). 0 on a bad string.
fn hex_to_u32(s: &str) -> u32 {
    u32::from_str_radix(s.strip_prefix('#').unwrap_or(s), 16).unwrap_or(0)
}

/// CITY (binary) — reads buildings from the (seeded) store, water from `zones` (`[px,pz,size,seed]×n`), and the
/// removable old spokes/plaza from `removables` (`[tag(0=path-from,1=plaza), x, z]×n`, JS maps a returned slot → id).
/// Same geometry as `city_ops`; emits GOP_REMOVE/ADDZONE/ADDPATH/ADD (plaza, spoke roads, ring lamps, block buildings).
pub fn city_ops_bin(store: &StructureStore, zones: &[f64], removables: &[f64], px: f64, pz: f64, yaw: f64) -> Vec<f64> {
    let (fx, fz) = (yaw.sin(), -yaw.cos());
    let (tx, tz) = (px + fx * 16.0, pz + fz * 16.0);
    let near: Vec<(f64, f64)> = store
        .live_slots()
        .iter()
        .filter_map(|&s| store.get(s))
        .filter(|st| st.kind == SK_HOUSE || st.kind == SK_CABIN || st.kind == SK_TOWER)
        .map(|st| (st.x, st.z))
        .filter(|&(x, z)| (x - tx).hypot(z - tz) < 45.0)
        .collect();
    let (cx, cz) = if near.is_empty() {
        (js_round(tx / 2.0) * 2.0, js_round(tz / 2.0) * 2.0)
    } else {
        let n = near.len() as f64;
        (near.iter().map(|p| p.0).sum::<f64>() / n, near.iter().map(|p| p.1).sum::<f64>() / n)
    };
    let mut max_r = 0.0f64;
    for &(x, z) in &near {
        max_r = max_r.max((x - cx).hypot(z - cz));
    }
    const RING_GAP: f64 = 16.0;
    let ring_r = if near.is_empty() { 16.0 } else { max_r + RING_GAP };
    let ring = if near.is_empty() { 0.0 } else { js_round(max_r / RING_GAP) };
    let spokes = 6;
    let road_w = 4.0;
    let edge = ring_r + 8.0;
    let zs = zones_seeded(zones);
    let mut out = Vec::new();

    // RE-LAY: remove the old city spokes (paths from near the centre) + the old plaza, then re-add fresh ones below.
    for (slot, r) in removables.chunks_exact(3).enumerate() {
        let (tag, x, z) = (r[0], r[1], r[2]);
        if (tag == 0.0 && (x - cx).hypot(z - cz) < 6.0) || (tag == 1.0 && (x - cx).hypot(z - cz) < 10.0) {
            gop_remove(&mut out, slot);
        }
    }
    if !in_water_seeded(&zs, cx, cz) {
        gop_addzone(&mut out, cx, cz, 15.0f64.min(6.0 + ring * 2.0), MAT_PLAZA, SHAPE_RECT);
    }
    let mut spoke_ang: Vec<f64> = Vec::new();
    for s in 0..spokes {
        let ang = (s as f64 / spokes as f64) * TAU + 0.26;
        spoke_ang.push(ang);
        gop_addpath(&mut out, (cx, cz), (cx + ang.cos() * edge, cz + ang.sin() * edge), road_w, MAT_PATH);
        let off = road_w / 2.0 + 0.6;
        let lx = cx + ang.cos() * ring_r - ang.sin() * off;
        let lz = cz + ang.sin() * ring_r + ang.cos() * off;
        if !in_water_seeded(&zs, lx, lz) {
            gop_add(&mut out, SK_LAMP, lx, lz, [1.0, 1.0, 1.0], 0.0, 0);
        }
    }

    let spacing = 13.0 + ring * 3.0;
    let count = js_round(TAU * ring_r / spacing).clamp(5.0, 30.0) as i32;
    let district = district_for(ring as i32);
    let clear_ang = 0.26f64.min((road_w / 2.0 + 2.0) / ring_r);
    let sector_w = TAU / spokes as f64;
    for i in 0..count {
        let a = (i as f64 / count as f64) * TAU + ring * 0.4 + 0.13;
        let mut on_road = false;
        for &sa in &spoke_ang {
            let da = ((((a - sa) % TAU) + TAU + std::f64::consts::PI) % TAU) - std::f64::consts::PI; // shortest angular dist
            if da.abs() < clear_ang {
                on_road = true;
            }
        }
        if on_road {
            continue; // leave the street clear
        }
        let jr = ring_r + (hash1(ring * 31.0 + i as f64 * 7.0) - 0.5) * RING_GAP * 0.4;
        let x = cx + a.cos() * jr;
        let z = cz + a.sin() * jr;
        if in_water_seeded(&zs, x, z) {
            continue; // never build on a lake
        }
        let sector = ((((a - 0.26) % TAU) + TAU) % TAU / sector_w).floor();
        let b_seed = ring * 23.0 + sector * 7.0;
        let tower_block = hash1(b_seed + 11.0) < district.tower_chance;
        let block_tone = district.tones[(hash1(b_seed + 3.0) * district.tones.len() as f64) as usize];
        let w_base = lerp(district.w, hash1(b_seed + 5.0));
        let h_base = lerp(district.h, hash1(b_seed + 7.0));
        let seed = i as f64 + ring * 17.0;
        let (kind_code, is_tower) = if tower_block {
            (SK_TOWER, true)
        } else if BUILDINGS[(i % 2) as usize] == "house" {
            (SK_HOUSE, false)
        } else {
            (SK_CABIN, false)
        };
        let wide = w_base * (0.92 + hash1(seed) * 0.16);
        let tall = h_base * (0.9 + hash1(seed + 5.0) * 0.2);
        let rot_deg = (cx - x).atan2(cz - z).to_degrees() + (hash1(seed + 9.0) - 0.5) * 16.0;
        let color = if is_tower { 0 } else { hex_to_u32(block_tone) }; // a block shares one wall tone; towers keep stone
        gop_add(&mut out, kind_code, x, z, [wide, tall, wide], rot_deg, color);
    }
    out
}

/// IMMIGRATION (binary) — `counts` = `[n, geneSum]×5` in FLOORS order (rabbit, kangaroo, person, cat, lion). Returns a
/// flat `[floorIdx, x, z, gene]×n` add-creature stream (JS maps floorIdx → the kind string). Same rng draw order as
/// `immigration_ops` so the founders land identically — only the JSON crossing is gone.
pub fn immigration_ops_bin(counts: &[f64], px: f64, pz: f64, global_avg: f64, seed: f64) -> Vec<f64> {
    const FLOOR: [usize; 5] = [6, 4, 4, 4, 2]; // floor counts; index → rabbit, kangaroo, person, cat, lion
    const GENEFLOW_MAX: usize = 8;
    const GENEFLOW_CHANCE: f64 = 0.18;
    let mut rng = Mulberry32::new((seed as i64 as u32).wrapping_add(0x9e37_79b9));
    let mut out = Vec::new();
    for (idx, &floor) in FLOOR.iter().enumerate() {
        let n = counts.get(idx * 2).copied().unwrap_or(0.0) as usize;
        let gene_sum = counts.get(idx * 2 + 1).copied().unwrap_or(0.0);
        let avg = if n > 0 { gene_sum / n as f64 } else { global_avg };
        let bring = if n <= 1 {
            floor.max(3)
        } else if n < floor {
            floor - n
        } else if n < GENEFLOW_MAX && rng.next() < GENEFLOW_CHANCE {
            1
        } else {
            0
        };
        if bring == 0 {
            continue;
        }
        let center = avg.max(global_avg).max(1.12);
        let base_a = rng.next() * TAU;
        let base_r = 140.0 + rng.next() * 130.0; // immigrants walk in from the WILD (140–270 m), clustered to pair up
        let bx = px + base_a.cos() * base_r;
        let bz = pz + base_a.sin() * base_r;
        for _ in 0..bring {
            let x = bx + (rng.next() - 0.5) * 12.0;
            let z = bz + (rng.next() - 0.5) * 12.0;
            let gene = (center - 0.06 + rng.next() * 0.34).clamp(crate::world::GENE_MIN, crate::world::GENE_MAX);
            out.extend_from_slice(&[idx as f64, x, z, gene]);
        }
    }
    out
}

/// SETTLEMENT PLAN (binary) — a deterministic town plan, packed flat: `[radius, numPaths, numObjects, <paths: fromX,
/// fromZ, toX, toZ × P>, <objects: kindCode, x, z, rot, sx, sy, sz × O>]`. PATHS first then OBJECTS, matching the
/// shared id counter in `settlement_plan` (JS rebuilds ids `{prefix}p{n}` / `{prefix}o{n}` in this order). All objects
/// are `keep` structures; y is 0 (grounded by the caller). Same rng draws as `settlement_plan` → identical town.
pub fn settlement_plan_bin(cx: f64, cz: f64, size: &str, seed: u32) -> Vec<f64> {
    let mut rng = Mulberry32::new(seed);
    let p = tier(size);
    let mut objs: Vec<f64> = Vec::new(); // [kindCode, x, z, rot, sx, sy, sz]×O
    let mut paths: Vec<f64> = Vec::new(); // [fromX, fromZ, toX, toZ]×P
    let half = (p.blocks as f64 * GAP) / 2.0;
    let mut lines: Vec<f64> = Vec::new();
    for i in 0..=p.blocks {
        lines.push(-half + i as f64 * GAP);
    }
    // STREET GRID — a path along every grid line, both axes
    for &off in &lines {
        paths.extend_from_slice(&[cx - half, cz + off, cx + half, cz + off]);
        paths.extend_from_slice(&[cx + off, cz - half, cx + off, cz + half]);
    }
    // HOUSES line the E–W streets, set back both sides, facing the road
    let kinds: &[&str] = match size {
        "hamlet" => &["cabin", "cabin", "house"],
        "city" => &["house", "house", "cabin", "manor"],
        _ => &["house", "cabin", "house"],
    };
    let mut placed = 0;
    let cols = ((p.blocks as f64 * GAP) / HOUSE_SPACING).floor().max(1.0) as i32;
    'outer: for &off in &lines {
        for c in 0..=cols {
            for &side_z in &[-SETBACK, SETBACK] {
                if placed >= p.houses {
                    break 'outer;
                }
                if rng.next() < 0.18 {
                    continue; // a few empty plots
                }
                let hx = cx - half + 4.0 + c as f64 * HOUSE_SPACING + (rng.next() - 0.5) * 1.4;
                let hz = cz + off + side_z + (rng.next() - 0.5) * 1.2;
                let kind = kinds[(rng.next() * kinds.len() as f64) as usize];
                let s = 0.9 + rng.next() * 0.5;
                objs.extend_from_slice(&[sk_code(kind) as f64, hx, hz, if side_z < 0.0 { 0.0 } else { 180.0 }, s, s, s]);
                placed += 1;
            }
        }
    }
    // CENTRAL PLAZA: a well at the crossroads + a lamp beside it
    objs.extend_from_slice(&[sk_code("well") as f64, cx, cz, 0.0, 1.0, 1.0, 1.0]);
    objs.extend_from_slice(&[sk_code("lamp") as f64, cx + 2.5, cz + 2.5, 0.0, 1.0, 1.0, 1.0]);
    // WATCHTOWER(S) at corners
    for t in 0..p.towers {
        let corner = if t == 0 { [-half, -half] } else { [half, half] };
        objs.extend_from_slice(&[sk_code("tower") as f64, cx + corner[0], cz + corner[1], 0.0, 1.0, 1.3, 1.0]);
    }
    // LAMPS at street intersections (skip the centre — the well's there)
    for &ox in &lines {
        for &oz in &lines {
            if ox == 0.0 && oz == 0.0 {
                continue;
            }
            if rng.next() < 0.5 {
                objs.extend_from_slice(&[sk_code("lamp") as f64, cx + ox, cz + oz, 0.0, 1.0, 1.0, 1.0]);
            }
        }
    }
    // PERIMETER FENCE (town/city) — a ring just outside the grid with a GATE gap on the +X road
    if p.fenced {
        let r = half + 6.0;
        let per = 2.0 * std::f64::consts::PI * r;
        let segs = (per / 1.4).floor().max(8.0) as i32;
        for i in 0..segs {
            let ang = (i as f64 / segs as f64) * std::f64::consts::PI * 2.0;
            if ang.abs() < 0.18 || (ang - std::f64::consts::PI * 2.0).abs() < 0.18 {
                continue; // gate gap
            }
            objs.extend_from_slice(&[sk_code("fence") as f64, cx + ang.cos() * r, cz + ang.sin() * r, ang.to_degrees() + 90.0, 1.0, 1.0, 1.0]);
        }
    }
    let radius = half + if p.fenced { 8.0 } else { 4.0 };
    let mut out = vec![radius, (paths.len() / 4) as f64, (objs.len() / 7) as f64];
    out.extend_from_slice(&paths);
    out.extend_from_slice(&objs);
    out
}

/// DEMO GALLERY (binary) — Rust owns the whole multi-town LAYOUT (mutual SPACING, column grid, per-site size + seed),
/// not just each town's plan, per "Rust owns all compute". A spaced gallery of every settlement size for seeding /
/// preview. Returns `[numSites, numPaths, numObjects, <sites: cx,cz,sizeCode × S>, <paths×4>, <objects×7>]` (paths
/// then objects, with global ids assigned JS-side). Bump `GAP` here — never in the renderer — to change the spread.
pub fn demo_gallery_bin() -> Vec<f64> {
    const GAP: f64 = 480.0; // mutual distance between seeded towns (doubled from the old JS 240)
    const COLS: i64 = 4;
    const COUNT: i64 = 12;
    const SIZES: [&str; 4] = ["hamlet", "village", "town", "city"];
    let mut sites: Vec<f64> = Vec::new();
    let mut all_paths: Vec<f64> = Vec::new();
    let mut all_objs: Vec<f64> = Vec::new();
    for k in 0..COUNT {
        let sidx = (k as usize) % SIZES.len();
        let cx = 160.0 + (k % COLS) as f64 * GAP;
        let cz = -GAP + (k / COLS) as f64 * GAP;
        sites.extend_from_slice(&[cx, cz, sidx as f64]);
        let plan = settlement_plan_bin(cx, cz, SIZES[sidx], (k * 1000 + 7) as u32);
        let (np, no) = (plan[1] as usize, plan[2] as usize);
        all_paths.extend_from_slice(&plan[3..3 + np * 4]);
        all_objs.extend_from_slice(&plan[3 + np * 4..3 + np * 4 + no * 7]);
    }
    let mut out = vec![COUNT as f64, (all_paths.len() / 4) as f64, (all_objs.len() / 7) as f64];
    out.extend_from_slice(&sites);
    out.extend_from_slice(&all_paths);
    out.extend_from_slice(&all_objs);
    out
}

// ── CITY — build/grow a concentric, district-zoned city ahead of the player (port of city.ts cityOps) ─────────
const TAU: f64 = std::f64::consts::PI * 2.0;
const BUILDINGS: [&str; 3] = ["house", "cabin", "tower"];
const WALL_TONES: [&str; 7] = ["#d2b48c", "#c9a978", "#be9d72", "#cdb389", "#b89a86", "#c2a15f", "#a98c63"];
const STONE_TONES: [&str; 4] = ["#b7b2a8", "#adb0b3", "#c1bcb0", "#a8a59c"];

struct District {
    tower_chance: f64,
    h: [f64; 2],
    w: [f64; 2],
    tones: &'static [&'static str],
}
/// District template per concentric ring: stone-tower CORE (0) → mid-rise belt (1) → low residential (2+).
fn district_for(ring: i32) -> District {
    match ring.min(2) {
        0 => District { tower_chance: 0.3, h: [1.5, 2.2], w: [1.0, 1.25], tones: &STONE_TONES },
        1 => District { tower_chance: 0.1, h: [1.1, 1.6], w: [0.95, 1.2], tones: &WALL_TONES },
        _ => District { tower_chance: 0.03, h: [0.85, 1.15], w: [0.85, 1.05], tones: &WALL_TONES },
    }
}
fn lerp(r: [f64; 2], t: f64) -> f64 {
    r[0] + (r[1] - r[0]) * t
}
fn is_building(k: &str) -> bool {
    matches!(k, "house" | "cabin" | "tower")
}
/// JS `Math.round` semantics — round half toward +∞ (`floor(x+0.5)`). Differs from Rust's `f64::round` (half away
/// from zero) for negative `.5` values like `Math.round(tx/2)` when the player stands at negative coords.
fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

#[cfg(test)]
mod tests {
    use super::*;

    // mulberry32 must reproduce the JS stream bit-for-bit (the parity test in JS checks the other direction; this
    // pins a few known values so a refactor here is caught even without the wasm bridge).
    #[test]
    fn mulberry32_is_deterministic_and_in_range() {
        let mut r = Mulberry32::new(12345);
        let a = r.next();
        let b = r.next();
        assert!((0.0..1.0).contains(&a) && (0.0..1.0).contains(&b));
        assert_ne!(a, b);
        // same seed → same stream
        let mut r2 = Mulberry32::new(12345);
        assert_eq!(r2.next(), a);
        assert_eq!(r2.next(), b);
    }

    #[test]
    fn well_ops_store_dedups_and_avoids_water() {
        // BINARY well placement: seed the store with an existing well + pack the lake zone (carrying its computed
        // seed). Three requests — near the existing well (deduped) · in the lake (rejected) · fresh dry ground (built)
        // — must yield exactly the one fresh dry well.
        let mut store = StructureStore::new();
        store.add(Structure { kind: SK_WELL, x: 0.0, z: 0.0, rot: 0.0, sx: 1.0, sy: 1.0, sz: 1.0, color: 0, keep: true, region: 0 });
        let reqs = [5.0, 0.0, 100.0, 0.0, 300.0, 0.0];
        let zones = [100.0, 0.0, 20.0, crate::engine::water_seed("lake")];
        let bin_adds: Vec<(f64, f64)> = well_ops_store(&mut store, &reqs, &zones).chunks_exact(9).map(|c| (c[2], c[3])).collect();
        assert_eq!(bin_adds.len(), 1, "only the fresh dry well builds (dedup + water rejection)");
        let (wx, wz) = bin_adds[0];
        assert!((wx - 300.0).abs() < 1e-9 && wz.abs() < 1e-9, "the placed well is the fresh dry one at ~300,0, got {bin_adds:?}");
    }

    #[test]
    fn grave_site_store_buries_outside_town_and_skips_wild_deaths() {
        // BINARY grave placement: a death inside a 3-home town gets a dry plot OUTSIDE the home cluster (centroid
        // ~(5,3.3)); a wild death (no building in range) gets none.
        let mut store = StructureStore::new();
        let homes = [(0.0, 0.0), (10.0, 0.0), (5.0, 10.0)];
        for &(x, z) in &homes {
            store.add(Structure { kind: SK_HOUSE, x, z, rot: 0.0, sx: 1.0, sy: 1.0, sz: 1.0, color: 0, keep: false, region: 0 });
        }
        let (dx, dz) = (6.0, 4.0);
        let bin = grave_site_store(&store, dx, dz, &[]);
        assert!(!bin.is_empty(), "a death inside the town gets a plot");
        let (cx, cz) = (5.0, 10.0 / 3.0); // home centroid
        let d = (bin[0] - cx).hypot(bin[1] - cz);
        assert!(d > 10.0, "the grave sits OUTSIDE the homes (d={d} m from centroid), got {bin:?}");
        // a wild death (no town in range) → no grave
        assert!(grave_site_store(&store, 500.0, 500.0, &[]).is_empty(), "a wild death gets no grave");
    }

    #[test]
    fn grow_dormant_houses_develops_a_far_town_and_caps() {
        // a small dormant cluster (3 homes) — far-town development should ADD homes around it, water-safe + clamped.
        let cluster = [0.0, 0.0, 8.0, 0.0, 0.0, 8.0];
        let ops = grow_dormant_houses(&cluster, 4, &[], 0.37);
        let adds: Vec<(f64, f64)> = ops.chunks_exact(9).map(|c| (c[2], c[3])).collect();
        assert!(!adds.is_empty() && adds.len() <= 4, "1..=4 new homes added, got {}", adds.len());
        // each new home is a building kind (house/cabin) and near the cluster (within the footprint, not flung away)
        for c in ops.chunks_exact(9) {
            assert_eq!(c[0], OP_ADD);
            assert!(c[1] == SK_HOUSE as f64 || c[1] == SK_CABIN as f64, "a home kind, got {}", c[1]);
            assert!((c[2]).hypot(c[3]) < 80.0, "new home stays near the cluster ({}, {})", c[2], c[3]);
        }
        // CAP: a cluster already at the colony cap (30 homes within 75 m) grows no further.
        let mut full: Vec<f64> = Vec::new();
        for i in 0..30 {
            let a = i as f64 * 0.7;
            full.push(a.cos() * 30.0);
            full.push(a.sin() * 30.0);
        }
        assert!(grow_dormant_houses(&full, 4, &[], 0.5).is_empty(), "a colony at its cap stops building");
        // want=0 and an empty cluster are no-ops
        assert!(grow_dormant_houses(&cluster, 0, &[], 0.1).is_empty());
        assert!(grow_dormant_houses(&[], 4, &[], 0.1).is_empty());
    }

    #[test]
    fn build_ops_store_clamps_infill_inside() {
        // The settlement-growth rule (user): a build NEAR a town is pulled INSIDE its footprint (the wall doesn't
        // balloon to wrap a house dropped just outside it); a build in the founding dead-zone is rejected; a build far
        // enough founds a NEW town at its requested spot. (This intentionally DIVERGES from the old build_ops — that
        // jzon twin is being retired — so it's no longer a parity test.)
        let mut store = StructureStore::new();
        for &(x, z) in &[(0.0, 0.0), (8.0, 0.0), (0.0, 8.0)] {
            store.add(Structure { kind: SK_HOUSE, x, z, rot: 0.0, sx: 1.0, sy: 1.0, sz: 1.0, color: 0, keep: false, region: 0 });
        }
        // cluster centroid ≈ (2.7, 2.7), spread ≈ 6 m → infill limit ≈ 14 m. Requests: infill dropped 60 m out (in the
        // band) · founding dead-zone @ 120 m (rejected) · open ground @ 400 m (founds a town).
        let breqs = [60.0, 0.0, 120.0, 0.0, 400.0, 0.0];
        let adds: Vec<(f64, f64)> = build_ops_store(&mut store, &breqs, &[]).chunks_exact(9).map(|c| (c[2], c[3])).collect();

        assert_eq!(adds.len(), 2, "the clamped infill + the far founding land; the dead-zone build is rejected");
        let infill = adds.iter().find(|&&(x, _)| x < 100.0).expect("an infill build");
        let d = (infill.0 - 2.7_f64).hypot(infill.1 - 2.7);
        assert!(d < 24.0, "infill pulled INSIDE the footprint (d={d} m from centroid), not left at the requested 60 m");
        assert!(adds.iter().any(|&(x, _)| x > 390.0), "the far build founds a NEW town at its requested ~400 m");
    }

    #[test]
    fn settlement_ops_store_walls_a_cluster_and_is_idempotent() {
        // BINARY wall fit: 3 spread homes are ringed by a real closed wall (>12 panels), and the position-diff state
        // lives in the store so re-fitting with no change emits zero ops (STATEFUL idempotency).
        let mut store = StructureStore::new();
        for &(x, z) in &[(0.0, 0.0), (14.0, 6.0), (6.0, 16.0)] {
            store.add(Structure { kind: SK_HOUSE, x, z, rot: 0.0, sx: 1.0, sy: 1.0, sz: 1.0, color: 0, keep: false, region: 0 });
        }
        let bin = settlement_ops_store(&mut store, &[], &[]);
        let panels = bin.chunks_exact(9).filter(|c| c[0] == OP_ADD).count();
        assert!(panels > 12, "a real closed wall, got {panels} panels");
        // STATEFUL idempotency: the store now holds the fences; re-fitting with no change emits nothing (the diff lives in the store).
        assert!(settlement_ops_store(&mut store, &[], &[]).is_empty(), "idempotent re-fit emits zero ops");
    }

    #[test]
    fn settlement_ops_store_local_fit_leaves_other_towns() {
        // LOCAL fit: when a structure changes in town A, ONLY town A's wall re-fits — town B's stored wall is left
        // exactly as-is (the "fence moves randomly while I fly over another town" fix). A whole-world re-fit off the
        // partial live home set was the bug; now `changed` scopes it.
        let mut store = StructureStore::new();
        let home = |s: &mut StructureStore, x: f64, z: f64| s.add(Structure { kind: SK_HOUSE, x, z, rot: 0.0, sx: 1.0, sy: 1.0, sz: 1.0, color: 0, keep: false, region: 0 });
        for &(x, z) in &[(0.0, 0.0), (12.0, 0.0), (0.0, 12.0)] {
            home(&mut store, x, z); // town A at origin
        }
        for &(x, z) in &[(300.0, 0.0), (312.0, 0.0), (300.0, 12.0)] {
            home(&mut store, x, z); // town B, 300 m east
        }
        let _ = settlement_ops_store(&mut store, &[], &[]); // fit EVERY town once (load reconcile)
        let b_wall = |s: &StructureStore| s.live_slots().iter().filter_map(|&sl| s.get(sl)).filter(|st| st.kind == SK_FENCE && (st.x - 304.0).hypot(st.z - 4.0) < 60.0).map(|st| ((st.x * 100.0) as i64, (st.z * 100.0) as i64)).collect::<std::collections::BTreeSet<_>>();
        let b_before = b_wall(&store);

        home(&mut store, 8.0, 8.0); // a settler raises a NEW home in town A
        let ops = settlement_ops_store(&mut store, &[], &[8.0, 8.0]); // re-fit ONLY town A (the change is at 8,8)
        let b_after = b_wall(&store);

        assert_eq!(b_before, b_after, "town B's wall must be byte-identical when only town A changes");
        assert!(!b_before.is_empty(), "town B actually has a wall (guards a vacuous test)");
        assert!(ops.chunks_exact(9).any(|c| c[0] == OP_ADD), "town A re-fit + grew its wall for the new home");
    }

    #[test]
    fn settlement_ops_store_refit_after_shrink_leaves_no_stranded_ring() {
        // A WIDE town is walled, then its WEST homes decay away → its footprint shrinks + its centroid shifts east. The
        // re-fit must RECLAIM the old wide ring's far-west panels (owned by nearest home), not strand them as a phantom
        // outer layer / spur (the user's "multi-layer fence, fence extending out"). The old radius-from-centroid keep
        // zone missed them because the centroid moved.
        let mut store = StructureStore::new();
        let home = |s: &mut StructureStore, x: f64, z: f64| s.add(Structure { kind: SK_HOUSE, x, z, rot: 0.0, sx: 1.0, sy: 1.0, sz: 1.0, color: 0, keep: false, region: 0 });
        let mut west: Vec<u32> = Vec::new();
        for &(x, z) in &[(-44.0, -6.0), (-44.0, 6.0), (-30.0, 0.0), (-16.0, 8.0)] {
            west.push(home(&mut store, x, z)); // the side that will decay
        }
        for &(x, z) in &[(0.0, 0.0), (14.0, -6.0), (28.0, 6.0), (40.0, 0.0)] {
            home(&mut store, x, z); // the survivors
        }
        let _ = settlement_ops_store(&mut store, &[], &[]); // fit the WIDE wall
        let far_west = |s: &StructureStore| s.live_slots().iter().filter_map(|&sl| s.get(sl)).filter(|st| st.kind == SK_FENCE && st.x < -40.0).count();
        assert!(far_west(&store) > 0, "the wide town is walled out west (guards a vacuous test)");

        for sl in west {
            store.remove(sl); // the west homes decay away → town now spans only x∈[-16,40]
        }
        let _ = settlement_ops_store(&mut store, &[], &[10.0, 0.0]); // re-fit (a change inside the town)

        assert_eq!(far_west(&store), 0, "the old far-west ring is reclaimed, NOT left as a phantom outer layer");
        assert!(store.live_slots().iter().filter_map(|&sl| store.get(sl)).any(|st| st.kind == SK_FENCE), "the shrunk town is still walled");
    }

    #[test]
    fn vegetation_ops_store_plants_deterministically() {
        let homes = [(0.0f64, 0.0f64), (10.0, 0.0), (5.0, 10.0), (15.0, 8.0)];
        let mk = |hs: &[(f64, f64)]| {
            let mut st = StructureStore::new();
            for &(x, z) in hs {
                st.add(Structure { kind: SK_HOUSE, x, z, rot: 0.0, sx: 1.0, sy: 1.0, sz: 1.0, color: 0, keep: false, region: 0 });
            }
            st
        };
        let mut hit = None;
        for k in 0..40 {
            let seed = k as f64 * 3.0 + 1.0;
            let op = vegetation_ops_store(&mut mk(&homes), seed, &[]);
            if !op.is_empty() {
                hit = Some((seed, op));
                break;
            }
        }
        let (seed, op) = hit.expect("some seed should roll 'plant'");
        assert_eq!(op[1], SK_TREE as f64);
        let (tx, tz) = (op[2], op[3]);
        assert!(homes.iter().any(|&(bx, bz)| (bx - tx).hypot(bz - tz) <= 10.6), "tree planted near a home");
        assert_eq!(vegetation_ops_store(&mut mk(&homes), seed, &[])[2..4], op[2..4], "deterministic for a fixed seed");
    }

    #[test]
    fn lake_ops_bin_grows_and_digs() {
        // BINARY lake: grow an existing pond → the REMOVE references the zone by SLOT (its index in the water-zones
        // array JS supplies) and the new blob is centred the same, 6 m bigger.
        let (px, pz, yaw) = (0.0, 0.0, 0.0);
        let zones = [10.0, 0.0, 12.0, crate::engine::water_seed("lake")];
        let bin = lake_ops_bin(&zones, px, pz, yaw);
        let recs: Vec<&[f64]> = bin.chunks_exact(GEN_STRIDE).collect();
        let b_remove_slot = recs.iter().find(|r| r[0] == GOP_REMOVE).map(|r| r[1] as usize);
        let b_zone = recs.iter().find(|r| r[0] == GOP_ADDZONE).map(|r| (r[1], r[2], r[3])).unwrap();
        assert_eq!(b_remove_slot, Some(0), "removes the only water zone, slot 0");
        assert_eq!(b_zone, (10.0, 0.0, 18.0), "the grown pond is centred the same, 6 m bigger");

        // fresh pond (no water nearby) → exactly one 13 m blob at the target, no remove
        let bin2 = lake_ops_bin(&[], px, pz, yaw);
        assert_eq!(bin2.chunks_exact(GEN_STRIDE).count(), 1, "one fresh op");
        assert_eq!((bin2[0], bin2[3]), (GOP_ADDZONE, 13.0), "one fresh 13 m pond");
    }

    #[test]
    fn forest_ops_bin_plants_a_wood() {
        // BINARY forest: an empty area plants a fresh wood (golden-spiral spread) of valid tree/pine kinds.
        let (px, pz, yaw) = (0.0, 0.0, 0.0);
        let store = StructureStore::new(); // no existing trees
        let b: Vec<(f64, f64, u8)> = forest_ops_bin(&store, &[], px, pz, yaw).chunks_exact(GEN_STRIDE).filter(|r| r[0] == GOP_ADD).map(|r| (r[2], r[3], r[1] as u8)).collect();
        assert!(!b.is_empty(), "an empty area plants a fresh wood");
        for (_, _, kind) in &b {
            assert!(*kind == SK_TREE || *kind == SK_PINE, "every planted tree is a tree or pine, got kind {kind}");
        }
    }

    #[test]
    fn city_ops_bin_grows_a_ring() {
        // BINARY city: grow a ring around an existing 3-building cluster → adds buildings + lamps (valid kinds), one
        // plaza zone, and the spoke roads.
        let objs = [(SK_HOUSE, 0.0, 0.0), (SK_HOUSE, 14.0, 6.0), (SK_TOWER, 6.0, 16.0)];
        let (px, pz, yaw) = (0.0, 0.0, 0.0);
        let mut store = StructureStore::new();
        for &(kind, x, z) in &objs {
            store.add(Structure { kind, x, z, rot: 0.0, sx: 1.0, sy: 1.0, sz: 1.0, color: 0, keep: false, region: 0 });
        }
        let bin = city_ops_bin(&store, &[], &[], px, pz, yaw);
        let adds = bin.chunks_exact(GEN_STRIDE).filter(|r| r[0] == GOP_ADD).count();
        let bcount = |t: f64| bin.chunks_exact(GEN_STRIDE).filter(|r| r[0] == t).count();
        assert!(adds > 0, "the ring adds buildings + lamps, got {adds}");
        assert_eq!(bcount(GOP_ADDZONE), 1, "one central plaza");
        assert_eq!(bcount(GOP_ADDPATH), 6, "six spoke roads");
        for r in bin.chunks_exact(GEN_STRIDE).filter(|r| r[0] == GOP_ADD) {
            let k = r[1] as u8;
            assert!(k == SK_HOUSE || k == SK_CABIN || k == SK_TOWER || k == SK_LAMP, "a city add is a building or lamp, got kind {k}");
        }
    }

    #[test]
    fn immigration_ops_bin_rescues_deficient_species() {
        // BINARY immigration: rabbit extinct (n=0) → a founding group walks in; cat healthy (n=10) → none. `counts`
        // = `[n, geneSum]×5` in FLOORS order (rabbit, kangaroo, person, cat, lion).
        let (px, pz, ga, seed) = (5.0, -3.0, 1.2, 7.0);
        let kinds = ["rabbit", "kangaroo", "person", "cat", "lion"];
        let cbin = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 10.0, 10.0, 0.0, 0.0]; // only cat is stocked (n=10)
        let b: Vec<(&str, f64)> = immigration_ops_bin(&cbin, px, pz, ga, seed).chunks_exact(4).map(|c| (kinds[c[0] as usize], c[3])).collect();
        let rabbits = b.iter().filter(|(k, _)| *k == "rabbit").count();
        assert!(rabbits >= 3, "an extinct species walks in a founding group, got {rabbits}");
        assert!(b.iter().all(|(k, _)| *k != "cat"), "the healthy cat does not immigrate");
        for (_, gene) in &b {
            assert!((crate::world::GENE_MIN..=crate::world::GENE_MAX).contains(gene) && *gene >= 1.0, "migrant gene rescued + in band, got {gene}");
        }
    }

    #[test]
    fn settlement_plan_bin_scales_with_tier() {
        // BINARY town plan: a city plans more houses than a hamlet, and a city is fenced while a hamlet is not. The
        // plan packs `[radius, numPaths, numObjects, <paths×4>, <objects×7>]`.
        let (cx, cz, seed) = (100.0, -50.0, 1234u32);
        let homes_and_fenced = |size: &str| {
            let bin = settlement_plan_bin(cx, cz, size, seed);
            let (np, no) = (bin[1] as usize, bin[2] as usize);
            let ostart = 3 + np * 4;
            let mut homes = 0usize;
            let mut fenced = false;
            for i in 0..no {
                let k = bin[ostart + i * 7] as u8;
                if k == SK_HOUSE || k == SK_CABIN {
                    homes += 1;
                } else if k == SK_FENCE {
                    fenced = true;
                }
            }
            (homes, fenced)
        };
        let (hamlet_homes, hamlet_fenced) = homes_and_fenced("hamlet");
        let (city_homes, city_fenced) = homes_and_fenced("city");
        assert!(city_homes > hamlet_homes, "a city plans more houses than a hamlet ({city_homes} vs {hamlet_homes})");
        assert!(city_fenced && !hamlet_fenced, "a city is fenced; a hamlet is not");
    }
}
