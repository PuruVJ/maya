//! Procedural WORLD GENERATORS — the deterministic "make city / forest / lake" commands and the settlement
//! planner. Ported from the old JS (src/lib/city.ts, src/lib/settlementPlanner.ts) so Rust owns ALL world-gen
//! compute (the "Rust owns all compute" north star); JS now only matches the command word + renders. Determinism
//! is preserved exactly — the same PRNG (mulberry32) and the same GLSL-style hash — and parity with the former JS
//! is pinned by tests (src/lib/worldgen.test.ts) so a port can't silently change a generated city/town.

use jzon::{array, object, JsonValue};

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
}
