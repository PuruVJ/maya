//! Procedural WORLD GENERATORS — the deterministic "make city / forest / lake" commands and the settlement
//! planner. Ported from the old JS (src/lib/city.ts, src/lib/settlementPlanner.ts) so Rust owns ALL world-gen
//! compute (the "Rust owns all compute" north star); JS now only matches the command word + renders. Determinism
//! is preserved exactly — the same PRNG (mulberry32) and the same GLSL-style hash — and parity with the former JS
//! is pinned by tests (src/lib/worldgen.test.ts) so a port can't silently change a generated city/town.

use crate::engine::{in_water, wzones_of};
use jzon::{array, object, JsonValue};

// ── shared helpers ────────────────────────────────────────────────────────────────────────────────────────────
fn f(v: &JsonValue) -> f64 {
    v.as_f64().unwrap_or(0.0)
}
/// (x, z) of an object/zone's `pos` (the world is XZ-planar; pos[1] is height).
fn pos_xz(o: &JsonValue) -> (f64, f64) {
    (f(&o["pos"][0]), f(&o["pos"][2]))
}
/// The GLSL-style hash used by the city/forest generators — `fract(sin(i*12.9898+4.13)*43758.5453)`, byte-for-byte
/// the JS `hash1`. f64 sin matches across the boundary to ~1e-12; the worldgen parity tests pin the result.
fn hash1(i: f64) -> f64 {
    let v = (i * 12.9898 + 4.13).sin() * 43758.5453;
    v - v.floor()
}
fn zones_vec(world: &JsonValue) -> Vec<JsonValue> {
    world["zones"].members().cloned().collect()
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

/// Plan one settlement → `{ objects, paths, radius }` (jzon). Faithful port of settlementPlanner.ts `settlementPlan`:
/// a street grid, houses lining + facing the roads, a central well/plaza, watchtower(s), and a perimeter fence on
/// the bigger tiers. Deterministic in (centre, size, seed) so a town is stable and every seed differs.
pub fn settlement_plan(cx: f64, cz: f64, size: &str, seed: u32, id_prefix: &str) -> JsonValue {
    let mut rng = Mulberry32::new(seed);
    let p = tier(size);
    let mut objects: Vec<JsonValue> = Vec::new();
    let mut paths: Vec<JsonValue> = Vec::new();
    let mut n = 0i64;
    // ids: `{prefix}o{n}` / `{prefix}p{n}`, n shared + incrementing in placement order (matches the JS)
    macro_rules! oid {
        () => {{
            let id = format!("{id_prefix}o{n}");
            n += 1;
            id
        }};
    }
    macro_rules! pid {
        () => {{
            let id = format!("{id_prefix}p{n}");
            n += 1;
            id
        }};
    }
    let half = (p.blocks as f64 * GAP) / 2.0;
    let mut lines: Vec<f64> = Vec::new();
    for i in 0..=p.blocks {
        lines.push(-half + i as f64 * GAP);
    }

    // ── STREET GRID — a Path along every grid line, both axes
    for &off in &lines {
        paths.push(object! { "id" => pid!(), "material" => "path", "from" => array![cx - half, 0.0, cz + off], "to" => array![cx + half, 0.0, cz + off], "width" => 3 });
        paths.push(object! { "id" => pid!(), "material" => "path", "from" => array![cx + off, 0.0, cz - half], "to" => array![cx + off, 0.0, cz + half], "width" => 3 });
    }

    // ── HOUSES line the E–W streets, set back on both sides, facing the road
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
                objects.push(object! {
                    "id" => oid!(), "kind" => kind, "pos" => array![hx, 0.0, hz],
                    "rot" => if side_z < 0.0 { 0 } else { 180 }, "scale" => array![s, s, s], "keep" => true,
                });
                placed += 1;
            }
        }
    }

    // ── CENTRAL PLAZA: a well at the crossroads + a lamp beside it
    objects.push(object! { "id" => oid!(), "kind" => "well", "pos" => array![cx, 0.0, cz], "keep" => true });
    objects.push(object! { "id" => oid!(), "kind" => "lamp", "pos" => array![cx + 2.5, 0.0, cz + 2.5], "keep" => true });

    // ── WATCHTOWER(S) at corners
    for t in 0..p.towers {
        let corner = if t == 0 { [-half, -half] } else { [half, half] };
        objects.push(object! { "id" => oid!(), "kind" => "tower", "pos" => array![cx + corner[0], 0.0, cz + corner[1]], "scale" => array![1.0, 1.3, 1.0], "keep" => true });
    }

    // ── LAMPS at the street intersections (skip the centre — the well's there)
    for &ox in &lines {
        for &oz in &lines {
            if ox == 0.0 && oz == 0.0 {
                continue;
            }
            if rng.next() < 0.5 {
                objects.push(object! { "id" => oid!(), "kind" => "lamp", "pos" => array![cx + ox, 0.0, cz + oz], "keep" => true });
            }
        }
    }

    // ── PERIMETER FENCE (town/city) — a ring just outside the grid with a GATE gap on the +X road
    if p.fenced {
        let r = half + 6.0;
        let seg_len = 1.4;
        let per = 2.0 * std::f64::consts::PI * r;
        let segs = (per / seg_len).floor().max(8.0) as i32;
        for i in 0..segs {
            let ang = (i as f64 / segs as f64) * std::f64::consts::PI * 2.0;
            if ang.abs() < 0.18 || (ang - std::f64::consts::PI * 2.0).abs() < 0.18 {
                continue; // gate gap
            }
            let fx = cx + ang.cos() * r;
            let fz = cz + ang.sin() * r;
            objects.push(object! { "id" => oid!(), "kind" => "fence", "pos" => array![fx, 0.0, fz], "rot" => ang.to_degrees() + 90.0, "keep" => true });
        }
    }

    object! { "objects" => objects, "paths" => paths, "radius" => half + if p.fenced { 8.0 } else { 4.0 } }
}

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

/// INCREMENTAL settlement walls (user: "the fence GROWS with the town"). Clusters the world's homes into settlements
/// and emits ops to keep each ringed by a HAPHAZARD perimeter that hugs the home spread — the convex-hull SUPPORT
/// function + per-vertex jitter, NOT a circle — seamlessly tiled (ceil → panels exactly span each edge → no holes),
/// routed AROUND water, demolishing rocks on its line. Player-built `keep` fences are exempt. A POSITION-DIFF against
/// the existing auto-fence makes it IDEMPOTENT: an unchanged town emits nothing; only growth / a freshly-loaded town
/// moves panels. Returns an ops JSON array (`add` fence · `remove` stale fence / demolished rock).
pub fn settlement_ops(world: &JsonValue) -> JsonValue {
    let mut ops = JsonValue::new_array();
    let zones = wzones_of(&zones_vec(world));
    let mut walled: Vec<(f64, f64, bool)> = Vec::new(); // (x, z, is_home)
    let mut rocks: Vec<(String, f64, f64)> = Vec::new();
    let mut existing: Vec<(String, f64, f64)> = Vec::new(); // non-`keep` fence: id, x, z
    for o in world["objects"].members() {
        let k = o["kind"].as_str().unwrap_or("");
        let keep = o["keep"].as_bool().unwrap_or(false);
        let (x, z) = pos_xz(o);
        if !x.is_finite() || !z.is_finite() || x.abs() > 1.0e6 || z.abs() > 1.0e6 {
            continue; // GUARD: a stray NaN/∞/absurd coord would blow the support-ring radius → an astronomical panel
            // count → OOM/freeze (the user's "crash near civilisations"). Skip it rather than wall the void.
        }
        if is_walled(k) {
            walled.push((x, z, is_home(k)));
        } else if k == "rock" && !keep {
            rocks.push((o["id"].as_str().unwrap_or("").to_string(), x, z));
        } else if k == "fence" && !keep {
            existing.push((o["id"].as_str().unwrap_or("").to_string(), x, z));
        }
    }
    let homes: Vec<(f64, f64)> = walled.iter().filter(|w| w.2).map(|w| (w.0, w.1)).collect();
    let n = homes.len();
    if n == 0 {
        return ops;
    }
    // union-find: homes within GATHER chain into one settlement
    let mut parent: Vec<usize> = (0..n).collect();
    const GATHER: f64 = 60.0;
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
    let mut desired: Vec<(f64, f64, f64, f64)> = Vec::new(); // x, z, rot_deg, scale_x
    let mut demolish: Vec<String> = Vec::new();
    const SEG: f64 = 6.5;
    const K: usize = 22;
    const MARGIN: f64 = 8.0;
    // cluster centroids (from home members), precomputed so towers/wells attach to their NEAREST town only
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
        // STABLE per-town jitter anchor = this cluster's MIN-coordinate home (lexicographic) — a fixed point that does
        // NOT drift as the town grows. The per-vertex wobble below is keyed on THIS, not the centroid: adding a house
        // no longer changes the hash input for all 22 vertices, so the ring stops tearing down + rebuilding on every
        // build (the "fence jumps / churns" bug + a jitter source — docs/spread-redesign.md P2). Town-specific so rings
        // still vary; stable as long as the anchor home stands (a rare decay of it nudges the wobble once, not per-build).
        let anchor = (0..n).filter(|&i| roots[i] == root).map(|i| homes[i]).fold((f64::INFINITY, f64::INFINITY), |a, h| if h < a { h } else { a });
        // pts = THIS town's homes + towers/wells whose NEAREST centroid is this one. (Gathering everything within 90 m
        // of the centroid let a neighbouring town's homes leak in → two walls enclosing each other = the overlap.)
        let mut pts: Vec<(f64, f64)> = (0..n).filter(|&i| roots[i] == root).map(|i| (homes[i].0 - cx, homes[i].1 - cz)).collect();
        for &(wx, wz, is_h) in &walled {
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
        // support-function ring (hugs the spread) + per-vertex jitter (haphazard). If a vertex lands in WATER, pull it
        // INWARD until dry → the polygon INDENTS around the lake (the wall stays closed; it doesn't gap at the shore).
        let mut ring: Vec<(f64, f64)> = Vec::with_capacity(K);
        for k in 0..K {
            let th = (k as f64 / K as f64) * TAU;
            let (dx, dz) = (th.cos(), th.sin());
            let mut sup = 0.0f64;
            for &(px2, pz2) in &pts {
                sup = sup.max(px2 * dx + pz2 * dz);
            }
            let jit = (hash1(anchor.0 * 53.0 + th * 97.0 + anchor.1 * 31.0) - 0.5) * 7.0; // keyed on the STABLE anchor, not the drifting centroid
            let mut r = (sup + MARGIN + jit).clamp(7.0, 400.0); // CAP the radius: a town's wall never needs >400 m — and a
            // capped radius caps the edge length below, so `np` (panels/edge) can never go astronomical (OOM/freeze guard).
            if in_water(&zones, cx + dx * r, cz + dz * r) {
                for s in 1..=16 {
                    let nr = (r - s as f64 * 2.0).max(4.0);
                    if !in_water(&zones, cx + dx * nr, cz + dz * nr) {
                        r = nr;
                        break;
                    }
                }
            }
            ring.push((cx + dx * r, cz + dz * r));
        }
        // tile each EDGE seamlessly into a COMPLETE, CLOSED ring — ceil(elen/SEG) panels EXACTLY span each edge (no
        // holes), each panel as wide as its slot so neighbours abut. NO gate gap (user: "fence must be fully complete";
        // animals no longer collide with fences — the settlement-avoidance keeps them out — so an opening isn't needed).
        // The ONLY breaks are where the ring genuinely crosses water (rare; the ring already indents inward to dodge it).
        for k in 0..K {
            let (ax, az) = ring[k];
            let (bx, bz) = ring[(k + 1) % K];
            let elen = (bx - ax).hypot(bz - az);
            let np = ((elen / SEG).ceil() as usize).clamp(1, 256); // hard cap (defence in depth): even a degenerate edge
            // yields at most 256 panels, never a usize-overflow loop that would push billions of panels (OOM/freeze).
            let edge_ang = (bz - az).atan2(bx - ax);
            let sx = (elen / np as f64) / 1.4;
            for jp in 0..np {
                let t = (jp as f64 + 0.5) / np as f64;
                let fx = ax + (bx - ax) * t;
                let fz = az + (bz - az) * t;
                if in_water(&zones, fx, fz) {
                    continue;
                }
                for rk in &rocks {
                    if (rk.1 - fx).hypot(rk.2 - fz) < 3.0 && !demolish.contains(&rk.0) {
                        demolish.push(rk.0.clone());
                    }
                }
                let rot = -edge_ang.to_degrees() + (hash1(fx + fz) - 0.5) * 10.0;
                desired.push((fx, fz, rot, sx));
            }
        }
    }
    // POSITION-DIFF → idempotent: keep existing auto-fence matching a desired panel; remove the rest; add uncovered
    // desired panels; remove demolished rocks.
    let mut keep_e = vec![false; existing.len()];
    let mut covered = vec![false; desired.len()];
    for (di, d) in desired.iter().enumerate() {
        for (ei, e) in existing.iter().enumerate() {
            if (d.0 - e.1).hypot(d.1 - e.2) < 0.5 {
                keep_e[ei] = true;
                covered[di] = true;
            }
        }
    }
    for (ei, e) in existing.iter().enumerate() {
        if !keep_e[ei] {
            let _ = ops.push(object! { "op" => "remove", "id" => e.0.clone() });
        }
    }
    for id in &demolish {
        let _ = ops.push(object! { "op" => "remove", "id" => id.clone() });
    }
    for (di, d) in desired.iter().enumerate() {
        if !covered[di] {
            let _ = ops.push(object! { "op" => "add", "kind" => "fence", "pos" => array![d.0, 0.0, d.1], "rot" => d.2, "scale" => array![d.3, 1.0, 1.0] });
        }
    }
    ops
}

/// GRAVE SITE — where to bury someone who died at (dx,dz). Rust owns it (same engine that knows water → NO graves in
/// lakes). Returns `{x,z}` for a cemetery plot just OUTSIDE the deceased's settlement, on DRY ground; or `null` if they
/// died in the WILD (no building within ~70 m). Ported from Scene.svelte `graveyardSpot`.
pub fn grave_site(world: &JsonValue, dx: f64, dz: f64) -> JsonValue {
    fn is_bldg(k: &str) -> bool {
        matches!(k, "house" | "cabin" | "manor" | "tower")
    }
    const MEMBER_R2: f64 = 70.0 * 70.0;
    // nearest building → did the deceased belong to a settlement?
    let mut bx = 0.0;
    let mut bz = 0.0;
    let mut best = MEMBER_R2;
    for o in world["objects"].members() {
        if !is_bldg(o["kind"].as_str().unwrap_or("")) {
            continue;
        }
        let (x, z) = pos_xz(o);
        let d2 = (x - dx).powi(2) + (z - dz).powi(2);
        if d2 < best {
            best = d2;
            bx = x;
            bz = z;
        }
    }
    if best >= MEMBER_R2 {
        return JsonValue::Null; // died in the wild → no grave
    }
    // settlement cluster around the nearest building → centroid + extent (so the plot sits OUTSIDE the homes + wall)
    let cluster: Vec<(f64, f64)> = world["objects"]
        .members()
        .filter(|o| is_bldg(o["kind"].as_str().unwrap_or("")) && {
            let (x, z) = pos_xz(o);
            (x - bx).hypot(z - bz) < 90.0
        })
        .map(pos_xz)
        .collect();
    let m = cluster.len() as f64;
    let cx = cluster.iter().map(|p| p.0).sum::<f64>() / m;
    let cz = cluster.iter().map(|p| p.1).sum::<f64>() / m;
    let mut rad = 0.0f64;
    for &(x, z) in &cluster {
        rad = rad.max((x - cx).hypot(z - cz));
    }
    // a stable plot direction (coarse-centroid hash) tried first, then ROTATE around the town for DRY ground
    let base = (((cx / 80.0).round() * 12.9898 + (cz / 80.0).round() * 78.233).sin()).abs() * TAU;
    let zones = wzones_of(&zones_vec(world));
    let r = rad + 18.0; // just outside the wall (~rad+8)
    for t in 0..8 {
        let a = base + (t as f64) * (TAU / 8.0);
        let gx = cx + a.cos() * r + (hash1(cx + t as f64) - 0.5) * 7.0;
        let gz = cz + a.sin() * r + (hash1(cz + t as f64 + 5.0) - 0.5) * 7.0;
        if !in_water(&zones, gx, gz) {
            return object! { "x" => gx, "z" => gz };
        }
    }
    JsonValue::Null // every direction wet (town ringed by water) → no grave this death
}

/// HOUSE PLACEMENT — Rust owns it (the engine knows water → homes clear the lake by a MARGIN, not dipping in). Takes
/// the world DOM + this frame's build REQUESTS (settler positions from the sim) and applies the colony rules: ≤10 homes
/// per town, a NEW town ≥350 m from any building, no plot/grave clash, dry ground (footprint margin). Returns add-house
/// ops for the valid plots. Ported from the Scene.svelte `drainBuilds` handler.
pub fn build_ops(world: &JsonValue, builds: &JsonValue) -> JsonValue {
    let mut ops = JsonValue::new_array();
    const COLONY_R2: f64 = 75.0 * 75.0;
    const COLONY_MAX: usize = 10;
    // A NEW town (a build with no close neighbours) must be ≥ this from any building so it reads as a SEPARATE
    // settlement. Derived from the sim's FOUND_GAP (the distance the land-pressure pioneer drive shoves surplus folk
    // out to, see docs/spread-redesign.md) and set SLIGHTLY under it (×0.83) so a pioneer reliably clears the bar and
    // founds where it lands → ONE source of truth, no drift. (Was a fixed 350 m DEAD ZONE no emergent disperser could
    // ever reach, so towns only infilled + sprawled into each other — the merge bug. Now pioneers found distinct towns.)
    let new_gap2 = (crate::world::FOUND_GAP * 0.83).powi(2);
    const HOUSE_CAP: usize = 200; // world-wide house ceiling (raised: the world is many SMALL spread towns now, not 3 big ones)
    let zones = wzones_of(&zones_vec(world));
    let mut bld: Vec<(f64, f64)> = Vec::new();
    let mut graves: Vec<(f64, f64)> = Vec::new();
    for o in world["objects"].members() {
        let k = o["kind"].as_str().unwrap_or("");
        if matches!(k, "house" | "cabin" | "tower" | "manor") {
            bld.push(pos_xz(o));
        } else if k == "grave" {
            graves.push(pos_xz(o));
        }
    }
    let mut placed: Vec<(f64, f64)> = Vec::new(); // homes added this batch (so requests respect each other)
    for b in builds.members() {
        if bld.len() + placed.len() >= HOUSE_CAP {
            break;
        }
        let gx = (f(&b["x"]) / 8.0).round() * 8.0; // snap to an 8 m grid → aligned blocks
        let gz = (f(&b["z"]) / 8.0).round() * 8.0;
        let mut near_in_colony = 0usize;
        let mut nearest2 = f64::INFINITY;
        let mut taken = false;
        for &(ox, oz) in bld.iter().chain(placed.iter()) {
            let d2 = (ox - gx).powi(2) + (oz - gz).powi(2);
            if d2 < 36.0 {
                taken = true;
                break;
            }
            if d2 < COLONY_R2 {
                near_in_colony += 1;
            }
            if d2 < nearest2 {
                nearest2 = d2;
            }
        }
        if taken || near_in_colony >= COLONY_MAX {
            continue;
        }
        if near_in_colony == 0 && nearest2 < new_gap2 {
            continue; // a brand-new town must be founded clear of any other building (≥ FOUND_GAP-ish) → distinct, no merge
        }
        if graves.iter().any(|&(gvx, gvz)| (gvx - gx).powi(2) + (gvz - gz).powi(2) < 64.0) {
            continue; // never build ON the graveyard
        }
        // WATER MARGIN: the house FOOTPRINT must clear the lake — centre AND four points ~4 m out — so a home never
        // dips into the shore (user: "houses building on lake's edge, a little bit in").
        if in_water(&zones, gx, gz) || in_water(&zones, gx + 4.0, gz) || in_water(&zones, gx - 4.0, gz) || in_water(&zones, gx, gz + 4.0) || in_water(&zones, gx, gz - 4.0) {
            continue;
        }
        // VARIED home (deterministic roll from the plot): cosy cabin / modest house / the occasional big "manor" (house)
        let roll = hash1(gx * 1.7 + gz * 0.31);
        let (kind, s) = if roll < 0.3 {
            ("cabin", 0.85 + hash1(gx + 11.0) * 0.25)
        } else if roll < 0.8 {
            ("house", 0.9 + hash1(gz + 17.0) * 0.35)
        } else {
            ("house", 1.35 + hash1(gx + gz + 23.0) * 0.4)
        };
        placed.push((gx, gz));
        let _ = ops.push(object! { "op" => "add", "kind" => kind, "pos" => array![gx, 0.0, gz], "scale" => array![s, s, s] });
    }
    ops
}

/// WELL PLACEMENT — Rust owns it. A settler with no water in reach dug one; place it (grid-snapped, never in a lake,
/// deduped so a cluster of diggers makes ONE well not a stack). Returns add-well ops. Ported from Scene `drainWells`.
pub fn well_ops(world: &JsonValue, wells: &JsonValue) -> JsonValue {
    let mut ops = JsonValue::new_array();
    let zones = wzones_of(&zones_vec(world));
    let mut existing: Vec<(f64, f64)> = world["objects"].members().filter(|o| o["kind"] == "well").map(pos_xz).collect();
    for w in wells.members() {
        let gx = (f(&w["x"]) / 4.0).round() * 4.0;
        let gz = (f(&w["z"]) / 4.0).round() * 4.0;
        if in_water(&zones, gx, gz) {
            continue; // a well in a lake is pointless
        }
        if existing.iter().any(|&(ex, ez)| (ex - gx).powi(2) + (ez - gz).powi(2) < 35.0 * 35.0) {
            continue; // a well already serves this spot (dedup, including others placed this batch)
        }
        existing.push((gx, gz));
        let _ = ops.push(object! { "op" => "add", "kind" => "well", "pos" => array![gx, 0.0, gz], "keep" => true });
    }
    ops
}

/// COLONY VEGETATION — Rust owns it. Occasionally a broadleaf tree takes root near a home so a town greens over time
/// (bounded ~1.3/building; never on a plot, another tree, or a lake). `seed` varies per call (the sim tick) for the
/// gradual roll. Returns at most ONE add-tree op. Ported from Scene's colony-vegetation block.
pub fn vegetation_ops(world: &JsonValue, seed: f64) -> JsonValue {
    let mut ops = JsonValue::new_array();
    let zones = wzones_of(&zones_vec(world));
    let mut blds: Vec<(f64, f64)> = Vec::new();
    let mut blockers: Vec<(f64, f64)> = Vec::new(); // buildings + trees/pines (don't plant on top)
    let mut trees: Vec<(f64, f64)> = Vec::new();
    for o in world["objects"].members() {
        let k = o["kind"].as_str().unwrap_or("");
        let p = pos_xz(o);
        if matches!(k, "house" | "cabin" | "tower") {
            blds.push(p);
            blockers.push(p);
        } else if k == "tree" || k == "pine" {
            blockers.push(p);
            if k == "tree" {
                trees.push(p);
            }
        }
    }
    let house_count = blds.len();
    // colony trees ≈ trees within 18 m of a building (proxy for the JS treePrefix count → bounds the canopy)
    let colony_trees = trees.iter().filter(|&&(tx, tz)| blds.iter().any(|&(bx, bz)| (bx - tx).hypot(bz - tz) < 18.0)).count();
    if house_count < 3 || colony_trees >= (house_count as f64 * 1.3) as usize || hash1(seed) >= 0.55 {
        return ops; // too small, leafy enough already, or the roll said "not this check"
    }
    let h = blds[(hash1(seed + 1.0) * house_count as f64) as usize % house_count];
    let ta = hash1(seed + 2.0) * TAU;
    let tr = 4.5 + hash1(seed + 3.0) * 6.0;
    let tx = h.0 + ta.cos() * tr;
    let tz = h.1 + ta.sin() * tr;
    if in_water(&zones, tx, tz) || blockers.iter().any(|&(ox, oz)| (ox - tx).abs() < 2.6 && (oz - tz).abs() < 2.6) {
        return ops; // on a plot / another tree / in a lake → skip
    }
    let s = 0.7 + hash1(seed + 4.0) * 0.5;
    let _ = ops.push(object! { "op" => "add", "kind" => "tree", "pos" => array![tx, 0.0, tz], "scale" => array![s, s, s] });
    ops
}

/// IMMIGRATION DECISION — Rust owns the rescue logic (extinction-proofing + genetic rescue + anti-inbreeding gene
/// flow). JS gathers the LIVE per-kind counts + vigour (from agentManager — Rust can't see the live agent set) and
/// hands them in as `counts` = `{kind: {n, geneSum}, …}`; this decides HOW MANY of each deficient kind walk in, with
/// rescued vigour, clustered near the player so founders can pair up. Returns add-creature ops (kind/pos/gene) that JS
/// pushes into world.objects (→ revealed as agents). `seed` varies per check (the sim tick). Ported from Scene.
pub fn immigration_ops(counts: &JsonValue, px: f64, pz: f64, global_avg: f64, seed: f64) -> JsonValue {
    let mut ops = JsonValue::new_array();
    const FLOORS: [(&str, usize); 5] = [("rabbit", 6), ("kangaroo", 4), ("person", 4), ("cat", 4), ("lion", 2)];
    const GENEFLOW_MAX: usize = 8;
    const GENEFLOW_CHANCE: f64 = 0.18;
    let mut rng = Mulberry32::new((seed as i64 as u32).wrapping_add(0x9e37_79b9));
    for (kind, floor) in FLOORS {
        let entry = &counts[kind];
        let n = f(&entry["n"]) as usize;
        let avg = if n > 0 { f(&entry["geneSum"]) / n as f64 } else { global_avg };
        // HOW MANY arrive: near-extinct (≤1) → a viable founding group (≥3); below floor → top up; small-but-stable →
        // occasional fresh blood (gene flow).
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
        // GENETIC RESCUE: robust dispersers (≥~1.12, biased above the struggling locals, with spread) → LIFTS a degraded
        // pool + injects diversity. A wave lands as ONE cluster (founders within mate-finding range), not scattered.
        let center = avg.max(global_avg).max(1.12);
        let base_a = rng.next() * TAU;
        let base_r = 140.0 + rng.next() * 130.0; // 140–270 m out (was 55–85): immigrants walk in from the WILD, not at the
        // player's feet — so a rescue never pops a cluster INSIDE/at a settlement the player is standing in (user). Still
        // within the live span (~300 m) so they materialise, and clustered so founders pair up.
        let bx = px + base_a.cos() * base_r;
        let bz = pz + base_a.sin() * base_r;
        for _ in 0..bring {
            let x = bx + (rng.next() - 0.5) * 12.0;
            let z = bz + (rng.next() - 0.5) * 12.0;
            let gene = (center - 0.06 + rng.next() * 0.34).clamp(crate::world::GENE_MIN, crate::world::GENE_MAX);
            let _ = ops.push(object! { "op" => "add", "kind" => kind, "pos" => array![x, 0.0, z], "gene" => gene });
        }
    }
    ops
}

// ── FOREST — plant/grow a wood ahead of the player (faithful port of city.ts forestOps) ──────────────────────
const FOREST_KINDS: [&str; 3] = ["tree", "tree", "pine"];
fn is_tree(k: &str) -> bool {
    k == "tree" || k == "pine"
}

/// Ops that plant (or grow) a forest. Centre = nearby trees' centroid (grows the same wood), else ahead of the
/// player; fills the next annulus outward with golden-spiral-spread, jittered trees. Returns an ops JSON array.
pub fn forest_ops(world: &JsonValue, px: f64, pz: f64, yaw: f64) -> JsonValue {
    let mut ops = JsonValue::new_array();
    let (fx, fz) = (yaw.sin(), -yaw.cos());
    let (tx, tz) = (px + fx * 14.0, pz + fz * 14.0);
    let near: Vec<(f64, f64)> = world["objects"]
        .members()
        .filter(|o| is_tree(o["kind"].as_str().unwrap_or("")))
        .map(pos_xz)
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
    let zones = wzones_of(&zones_vec(world));
    for i in 0..count {
        let t = (i as f64 + 0.5) / count as f64;
        let r = (inner_r * inner_r + t * (outer_r * outer_r - inner_r * inner_r)).sqrt();
        let a = i as f64 * ga + hash1(i as f64) * 0.6;
        let jr = 1.0 + (hash1(i as f64 + 99.0) - 0.5) * 4.0;
        let x = cx + a.cos() * (r + jr);
        let z = cz + a.sin() * (r + jr);
        if in_water(&zones, x, z) {
            continue; // trees don't grow in the lake
        }
        let kind = FOREST_KINDS[(hash1(i as f64 + 7.0) * FOREST_KINDS.len() as f64) as usize];
        let s = 0.8 + hash1(i as f64 + 31.0) * 0.7;
        let _ = ops.push(object! { "op" => "add", "kind" => kind, "pos" => array![x, 0.0, z], "scale" => array![s, s, s], "rot" => hash1(i as f64 + 51.0) * 360.0 });
    }
    ops
}

// ── LAKE — dig/enlarge a pond ahead of the player (faithful port of city.ts lakeOps) ──────────────────────────
/// Ops to dig (or grow) a lake: a fresh organic pond ahead of you, or — if you're at an existing one — remove it
/// and re-add a bigger zone centred the same. Returns an ops JSON array.
pub fn lake_ops(world: &JsonValue, px: f64, pz: f64, yaw: f64) -> JsonValue {
    let mut ops = JsonValue::new_array();
    let (fx, fz) = (yaw.sin(), -yaw.cos());
    let (tx, tz) = (px + fx * 18.0, pz + fz * 18.0);
    let mut best: Option<(String, f64, f64, f64)> = None; // (id, x, z, size)
    let mut bd = f64::INFINITY;
    for z in world["zones"].members() {
        if z["material"].as_str() != Some("water") {
            continue;
        }
        let (zx, zz) = pos_xz(z);
        let size = f(&z["size"]);
        let d = (zx - tx).hypot(zz - tz);
        if d < size + 16.0 && d < bd {
            bd = d;
            best = Some((z["id"].as_str().unwrap_or("").to_string(), zx, zz, size));
        }
    }
    if let Some((id, zx, zz, size)) = best {
        let _ = ops.push(object! { "op" => "remove", "id" => id });
        let _ = ops.push(object! { "op" => "addZone", "material" => "water", "shape" => "blob", "pos" => array![zx, 0.0, zz], "size" => size + 6.0 });
    } else {
        let _ = ops.push(object! { "op" => "addZone", "material" => "water", "shape" => "blob", "pos" => array![tx, 0.0, tz], "size" => 13.0 });
    }
    ops
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

/// Ops that build (or grow) a city — the next concentric ring outward of district-zoned, plaza-facing buildings +
/// radial spoke roads + lamps + a growing central plaza. Faithful port of city.ts cityOps. Returns an ops array.
pub fn city_ops(world: &JsonValue, px: f64, pz: f64, yaw: f64) -> JsonValue {
    let mut ops = JsonValue::new_array();
    let (fx, fz) = (yaw.sin(), -yaw.cos());
    let (tx, tz) = (px + fx * 16.0, pz + fz * 16.0);
    let near: Vec<(f64, f64)> = world["objects"]
        .members()
        .filter(|o| is_building(o["kind"].as_str().unwrap_or("")))
        .map(pos_xz)
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

    // RE-LAY the road network + plaza so they always reach the rim as the city grows: remove the old city spokes
    // (paths starting at the centre) and the old plaza; fresh ones are added below. User-drawn roads are untouched.
    for p in world["paths"].members() {
        if (f(&p["from"][0]) - cx).hypot(f(&p["from"][2]) - cz) < 6.0 {
            let _ = ops.push(object! { "op" => "remove", "id" => p["id"].as_str().unwrap_or("") });
        }
    }
    for z in world["zones"].members() {
        if z["material"].as_str() == Some("plaza") {
            let (zx, zz) = pos_xz(z);
            if (zx - cx).hypot(zz - cz) < 10.0 {
                let _ = ops.push(object! { "op" => "remove", "id" => z["id"].as_str().unwrap_or("") });
            }
        }
    }
    let zones = wzones_of(&zones_vec(world));
    if !in_water(&zones, cx, cz) {
        let _ = ops.push(object! { "op" => "addZone", "material" => "plaza", "shape" => "rect", "pos" => array![cx, 0.0, cz], "size" => 15.0f64.min(6.0 + ring * 2.0) });
    }
    let mut spoke_ang: Vec<f64> = Vec::new();
    for s in 0..spokes {
        let ang = (s as f64 / spokes as f64) * TAU + 0.26;
        spoke_ang.push(ang);
        let _ = ops.push(object! { "op" => "addPath", "material" => "path", "fromPos" => array![cx, 0.0, cz], "toPos" => array![cx + ang.cos() * edge, 0.0, cz + ang.sin() * edge], "width" => road_w });
        let off = road_w / 2.0 + 0.6;
        let lx = cx + ang.cos() * ring_r - ang.sin() * off;
        let lz = cz + ang.sin() * ring_r + ang.cos() * off;
        if !in_water(&zones, lx, lz) {
            let _ = ops.push(object! { "op" => "add", "kind" => "lamp", "pos" => array![lx, 0.0, lz] });
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
        if in_water(&zones, x, z) {
            continue; // never build on a lake
        }
        // BLOCK coherence: everything in the same wedge of the same ring shares a style baseline
        let sector = ((((a - 0.26) % TAU) + TAU) % TAU / sector_w).floor();
        let b_seed = ring * 23.0 + sector * 7.0;
        let tower_block = hash1(b_seed + 11.0) < district.tower_chance;
        let block_tone = district.tones[(hash1(b_seed + 3.0) * district.tones.len() as f64) as usize];
        let w_base = lerp(district.w, hash1(b_seed + 5.0));
        let h_base = lerp(district.h, hash1(b_seed + 7.0));
        let seed = i as f64 + ring * 17.0;
        let kind = if tower_block { "tower" } else { BUILDINGS[(i % 2) as usize] };
        let wide = w_base * (0.92 + hash1(seed) * 0.16);
        let tall = h_base * (0.9 + hash1(seed + 5.0) * 0.2);
        let rot_deg = (cx - x).atan2(cz - z).to_degrees() + (hash1(seed + 9.0) - 0.5) * 16.0;
        let mut o = object! { "op" => "add", "kind" => kind, "pos" => array![x, 0.0, z], "rot" => rot_deg, "scale" => array![wide, tall, wide] };
        if kind != "tower" {
            o["color"] = block_tone.into(); // towers keep stone; a block shares one wall tone
        }
        let _ = ops.push(o);
    }
    ops
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
    fn settlement_scales_with_tier() {
        let hamlet = settlement_plan(0.0, 0.0, "hamlet", 7, "t_");
        let city = settlement_plan(0.0, 0.0, "city", 7, "t_");
        let houses = |v: &JsonValue| v["objects"].members().filter(|o| matches!(o["kind"].as_str(), Some("house" | "cabin" | "manor"))).count();
        assert!(houses(&city) > houses(&hamlet), "a city should plan more houses than a hamlet");
        // a city is fenced; a hamlet is not
        let fenced = |v: &JsonValue| v["objects"].members().any(|o| o["kind"] == "fence");
        assert!(fenced(&city) && !fenced(&hamlet));
    }

    #[test]
    fn grave_site_buries_outside_town_and_skips_wild_deaths() {
        let world = object! {
            "objects" => array![
                object! { "id" => "h0", "kind" => "house", "pos" => array![0.0, 0.0, 0.0] },
                object! { "id" => "h1", "kind" => "house", "pos" => array![10.0, 0.0, 0.0] },
            ],
            "zones" => array![],
        };
        // a death near the town → a grave OUTSIDE the home cluster (centroid ~(5,0))
        let r = grave_site(&world, 5.0, 4.0);
        assert!(!r.is_null(), "a death near a settlement should get a grave");
        let d = (f(&r["x"]) - 5.0).hypot(f(&r["z"]));
        assert!(d > 15.0, "grave should sit OUTSIDE the homes, got dist {d}");
        // a WILD death (no building within ~70 m) → no grave
        assert!(grave_site(&world, 600.0, 600.0).is_null(), "a wild death gets no grave");
    }

    #[test]
    fn grave_site_never_in_water() {
        // the town sits under one big lake → every plot direction is wet → no grave (engine knows water)
        let world = object! {
            "objects" => array![object! { "id" => "h0", "kind" => "house", "pos" => array![0.0, 0.0, 0.0] }],
            "zones" => array![object! { "material" => "water", "shape" => "blob", "pos" => array![0.0, 0.0, 0.0], "size" => 200.0 }],
        };
        assert!(grave_site(&world, 2.0, 2.0).is_null(), "no dry ground anywhere → no grave in the lake");
    }

    #[test]
    fn settlement_ops_walls_a_cluster_and_is_idempotent() {
        let world = object! {
            "objects" => array![
                object! { "id" => "h0", "kind" => "house", "pos" => array![0.0, 0.0, 0.0] },
                object! { "id" => "h1", "kind" => "house", "pos" => array![12.0, 0.0, 0.0] },
            ],
            "zones" => array![],
        };
        let ops = settlement_ops(&world);
        let adds: Vec<JsonValue> = ops.members().filter(|o| o["op"] == "add" && o["kind"] == "fence").cloned().collect();
        assert!(adds.len() > 8, "two homes should be walled by several panels, got {}", adds.len());
        // apply the panels, then re-run → IDEMPOTENT (the position-diff finds them all → no new ops)
        let mut w2 = world.clone();
        for (i, a) in adds.iter().enumerate() {
            let _ = w2["objects"].push(object! { "id" => format!("f{i}"), "kind" => "fence", "pos" => a["pos"].clone() });
        }
        let ops2 = settlement_ops(&w2);
        assert_eq!(ops2.members().count(), 0, "an already-walled town should emit nothing (idempotent), got {}", ops2.members().count());
    }

    // a fence panel's ground line: centre, unit direction (local +X under the Y-rotation), length
    fn panel_dir_len(o: &JsonValue) -> ((f64, f64), (f64, f64), f64) {
        let (x, z) = pos_xz(o);
        let rot = f(&o["rot"]) * std::f64::consts::PI / 180.0;
        let len = f(&o["scale"][0]) * 1.4;
        ((x, z), (rot.cos(), -rot.sin()), len)
    }
    // two panels OVERLAP if they're near-parallel (<~10°), on the same line (perp < 0.5 m), and their extents overlap
    // by > 1 m — i.e. stacked / duplicated. (Corner panels meet at an ANGLE, so they're excluded; a convex ring can't
    // cross itself.)
    fn panels_overlap(a: &JsonValue, b: &JsonValue) -> bool {
        let (ca, da, la) = panel_dir_len(a);
        let (cb, db, lb) = panel_dir_len(b);
        if (da.0 * db.0 + da.1 * db.1).abs() < 0.985 {
            return false;
        }
        let rel = (cb.0 - ca.0, cb.1 - ca.1);
        let along = rel.0 * da.0 + rel.1 * da.1;
        let perp = (rel.0 - da.0 * along, rel.1 - da.1 * along);
        if perp.0.hypot(perp.1) > 0.5 {
            return false;
        }
        let lo = (-la / 2.0).max((along - lb / 2.0).min(along + lb / 2.0));
        let hi = (la / 2.0).min((along - lb / 2.0).max(along + lb / 2.0));
        hi - lo > 1.0
    }

    #[test]
    fn settlement_wall_is_a_complete_closed_ring() {
        // the wall must be COMPLETE — no gate, no missing section (user: "fence not fully complete"). Build a town's
        // wall, take each panel's bearing from the home centroid, sort, and assert NO angular gap between consecutive
        // panels is large enough to be a hole (panels abut all the way round).
        let world = object! {
            "objects" => array![
                object! { "id" => "h0", "kind" => "house", "pos" => array![0.0, 0.0, 0.0] },
                object! { "id" => "h1", "kind" => "house", "pos" => array![14.0, 0.0, 6.0] },
                object! { "id" => "h2", "kind" => "house", "pos" => array![6.0, 0.0, 16.0] },
            ],
            "zones" => array![],
        };
        let (cx, cz) = (20.0 / 3.0, 22.0 / 3.0); // home centroid (matches settlement_ops)
        let mut angs: Vec<f64> = settlement_ops(&world)
            .members()
            .filter(|o| o["op"] == "add" && o["kind"] == "fence")
            .map(|o| (f(&o["pos"][2]) - cz).atan2(f(&o["pos"][0]) - cx))
            .collect();
        assert!(angs.len() > 12, "expected a full wall, got {} panels", angs.len());
        angs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mut max_gap = 0.0f64;
        for i in 0..angs.len() {
            let next = if i + 1 < angs.len() { angs[i + 1] } else { angs[0] + TAU };
            max_gap = max_gap.max(next - angs[i]);
        }
        assert!(max_gap < 0.6, "the ring has a HOLE: widest gap between panels is {max_gap:.2} rad (should be a tight, complete ring)");
    }

    // P2 (docs/spread-redesign.md): adding ONE house must NOT tear down + rebuild the whole ring. With the per-vertex
    // wobble anchored to a STABLE corner home (not the drifting centroid), a build keeps almost every existing panel
    // (the position-diff still matches) instead of mass remove+re-add (the "fence jumps/churns on every build" bug).
    #[test]
    fn settlement_wall_is_stable_when_a_house_is_added() {
        let homes = |extra: bool| {
            let mut o = array![
                object! { "id" => "h0", "kind" => "house", "pos" => array![0.0, 0.0, 0.0] },
                object! { "id" => "h1", "kind" => "house", "pos" => array![10.0, 0.0, 2.0] },
                object! { "id" => "h2", "kind" => "house", "pos" => array![3.0, 0.0, 11.0] },
                object! { "id" => "h3", "kind" => "house", "pos" => array![12.0, 0.0, 12.0] },
            ];
            if extra {
                let _ = o.push(object! { "id" => "h4", "kind" => "house", "pos" => array![6.0, 0.0, 5.0] }); // an INFILL house inside the hull
            }
            o
        };
        // 1) fit the base town's wall (no existing fences → every panel is an 'add')
        let base = settlement_ops(&object! { "objects" => homes(false), "zones" => array![] });
        let panels: Vec<JsonValue> = base.members().filter(|o| o["op"] == "add" && o["kind"] == "fence").cloned().collect();
        let total = panels.len();
        // 2) world = town + a NEW infill house + those panels already materialised as existing fences
        let mut objs = homes(true);
        for (i, p) in panels.iter().enumerate() {
            let _ = objs.push(object! { "id" => format!("fc{i}"), "kind" => "fence", "pos" => p["pos"].clone(), "rot" => p["rot"].clone(), "scale" => p["scale"].clone() });
        }
        // 3) re-fit with the extra house present → how much of the ring gets torn out?
        let removes = settlement_ops(&object! { "objects" => objs, "zones" => array![] }).members().filter(|o| o["op"] == "remove").count();
        eprintln!("[wall-stable] base {total} panels; after +1 house: {removes} removed");
        assert!(total > 10, "sanity: a real wall");
        assert!(removes * 3 < total, "adding a house must keep MOST of the ring (stable), not rebuild it (removed {removes}/{total})");
    }

    #[test]
    fn settlement_walls_do_not_overlap() {
        // settlement A (3 spread homes near origin) + a SECOND town B ~80 m away. Their walls must not stack on each
        // other, and within a wall no panel may overlap another.
        let world = object! {
            "objects" => array![
                object! { "id" => "a0", "kind" => "house", "pos" => array![0.0, 0.0, 0.0] },
                object! { "id" => "a1", "kind" => "house", "pos" => array![14.0, 0.0, 6.0] },
                object! { "id" => "a2", "kind" => "house", "pos" => array![6.0, 0.0, 16.0] },
                object! { "id" => "b0", "kind" => "house", "pos" => array![85.0, 0.0, 0.0] },
                object! { "id" => "b1", "kind" => "house", "pos" => array![99.0, 0.0, 8.0] },
            ],
            "zones" => array![],
        };
        let panels: Vec<JsonValue> = settlement_ops(&world).members().filter(|o| o["op"] == "add" && o["kind"] == "fence").cloned().collect();
        assert!(panels.len() > 12, "expected full walls, got {} panels", panels.len());
        let mut overlaps = 0;
        for i in 0..panels.len() {
            for j in (i + 1)..panels.len() {
                if panels_overlap(&panels[i], &panels[j]) {
                    overlaps += 1;
                }
            }
        }
        assert_eq!(overlaps, 0, "found {overlaps} overlapping fence-panel pairs (stacked/duplicated walls)");
    }

    #[test]
    fn build_ops_never_places_a_house_in_water() {
        let world = object! {
            "objects" => array![],
            "zones" => array![object! { "material" => "water", "shape" => "blob", "pos" => array![0.0, 0.0, 0.0], "size" => 25.0 }],
        };
        let mut builds = JsonValue::new_array(); // a spread of requests straight across the lake
        for i in -4..=8 {
            let _ = builds.push(object! { "x" => (i as f64) * 8.0, "z" => 0.0 });
        }
        let adds: Vec<JsonValue> = build_ops(&world, &builds).members().filter(|o| o["op"] == "add").cloned().collect();
        assert!(!adds.is_empty(), "the dry requests should build");
        let zones = wzones_of(&zones_vec(&world));
        for a in &adds {
            let (x, z) = (f(&a["pos"][0]), f(&a["pos"][2]));
            assert!(
                !in_water(&zones, x, z) && !in_water(&zones, x - 4.0, z) && !in_water(&zones, x + 4.0, z) && !in_water(&zones, x, z - 4.0) && !in_water(&zones, x, z + 4.0),
                "house at ({x},{z}) must clear water + a 4 m footprint margin"
            );
        }
    }

    #[test]
    fn build_ops_caps_a_dense_colony() {
        // a tight cluster of requests (all within one colony radius) caps at COLONY_MAX (10) homes
        let world = object! { "objects" => array![], "zones" => array![] };
        let mut builds = JsonValue::new_array();
        for i in 0..30 {
            let _ = builds.push(object! { "x" => (i % 5) as f64 * 8.0, "z" => (i / 5) as f64 * 8.0 });
        }
        let adds = build_ops(&world, &builds).members().filter(|o| o["op"] == "add").count();
        assert!(adds <= 10, "a dense colony caps at 10 homes, got {adds}");
        assert!(adds >= 5, "but several should build, got {adds}");
    }

    #[test]
    fn build_ops_founds_a_new_town_past_the_gap_but_not_inside_it() {
        // One existing town at the origin. A pioneer the sim shoves out to ~FOUND_GAP (240 m) FOUNDS a new DISTINCT
        // town where it lands; a build still CLOSE IN (well inside the gap) is rejected, so a town can't creep/found
        // into its neighbour (no merge). The gap is DERIVED from the sim's FOUND_GAP → one source of truth, and the
        // old fixed 350 m DEAD ZONE (which no emergent disperser could reach → towns only ever merged) is gone.
        let world = object! { "objects" => array![object! { "id" => "h0", "kind" => "house", "pos" => array![0.0, 0.0, 0.0] }], "zones" => array![] };
        let founds = |x: f64| build_ops(&world, &array![object! { "x" => x, "z" => 0.0 }]).members().filter(|o| o["op"] == "add").count();
        assert_eq!(founds(240.0), 1, "a pioneer out at ~FOUND_GAP should FOUND a new distinct town where it lands");
        assert_eq!(founds(400.0), 1, "and certainly one well past the gap");
        assert_eq!(founds(120.0), 0, "a build still close in (inside the gap) is rejected → no creep/merge into a neighbour");
    }

    #[test]
    fn well_ops_dedups_and_avoids_water() {
        let world = object! {
            "objects" => array![object! { "id" => "w0", "kind" => "well", "pos" => array![0.0, 0.0, 0.0] }],
            "zones" => array![object! { "material" => "water", "shape" => "blob", "pos" => array![100.0, 0.0, 0.0], "size" => 20.0 }],
        };
        // near the existing well (deduped) · in the lake (rejected) · fresh dry ground (built)
        let reqs = array![object! { "x" => 5.0, "z" => 0.0 }, object! { "x" => 100.0, "z" => 0.0 }, object! { "x" => 300.0, "z" => 0.0 }];
        let adds = well_ops(&world, &reqs).members().filter(|o| o["op"] == "add").count();
        assert_eq!(adds, 1, "only the fresh dry well should build, got {adds}");
    }

    #[test]
    fn vegetation_ops_bounded_and_needs_a_town() {
        let small = object! { "objects" => array![object! { "id" => "h0", "kind" => "house", "pos" => array![0.0, 0.0, 0.0] }], "zones" => array![] };
        assert_eq!(vegetation_ops(&small, 1.0).members().count(), 0, "a lone home gets no orchard");
        let town = object! {
            "objects" => array![
                object! { "id" => "h0", "kind" => "house", "pos" => array![0.0, 0.0, 0.0] },
                object! { "id" => "h1", "kind" => "house", "pos" => array![10.0, 0.0, 0.0] },
                object! { "id" => "h2", "kind" => "house", "pos" => array![5.0, 0.0, 10.0] },
            ],
            "zones" => array![],
        };
        for s in 0..20 {
            assert!(vegetation_ops(&town, s as f64).members().count() <= 1, "at most one tree per call");
        }
    }

    #[test]
    fn immigration_rescues_deficient_species() {
        // rabbit fully extinct (n=0) → a founding group; cat healthy + above gene-flow window (n=10) → none
        let counts = object! {
            "rabbit" => object! { "n" => 0, "geneSum" => 0.0 },
            "cat" => object! { "n" => 10, "geneSum" => 10.0 },
        };
        let ops = immigration_ops(&counts, 0.0, 0.0, 1.2, 5.0);
        let rabbits = ops.members().filter(|o| o["op"] == "add" && o["kind"] == "rabbit").count();
        let cats = ops.members().filter(|o| o["op"] == "add" && o["kind"] == "cat").count();
        assert!(rabbits >= 3, "an extinct species walks in a founding group, got {rabbits}");
        assert_eq!(cats, 0, "a healthy species gets none, got {cats}");
        for o in ops.members() {
            let g = f(&o["gene"]);
            assert!((0.6..=1.6).contains(&g) && g >= 1.0, "migrant gene rescued + in band, got {g}");
        }
    }
}
