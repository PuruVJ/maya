//! BINARY away/jump CATCH-UP — the closed-form world advance run on RETURN (reload, tab-return, ⏩ skip-ahead).
//! A faithful Rust port of JS `world.ts::fastForward` (the LIVE slice) + `streaming.ts::fastForwardDormantAway` (the
//! DORMANT far world), so ALL the catch-up SIM compute — population relaxation, settlement founding + spread, the
//! unified town cap — lives in Rust. JS only serialises the world in, applies the returned world, and renders.
//!
//! Reuses the already-Rust math (`ff_targets`/`ff_gene`/`world_area_scale`/`grow_dormant_houses`) and the binary
//! `EObj` boundary (engine_bin). Built ALONGSIDE the JS + invariant-tested, then wired (mirrors the engine_bin →
//! apply_ops rollout). Determinism: JS `Math.random` placement jitter → the crate's seeded `rng::rand` here, so a
//! catch-up is a pure function of (world, away-ms, seed) — testable, and identical in worker / main / server.

use crate::engine::{height_at, in_water_seeded};
use crate::engine_bin::{decode_objs, EFeat, EObj};
use crate::rng;
use crate::structstore::kind_str;
use crate::world::{ff_gene, ff_targets, world_area_scale, FOUND_GAP, GENE_MAX, GENE_MIN};
use std::collections::{HashMap, HashSet};

const CHUNKS: usize = 6; // co-development spiral: chunk the away span so houses→capacity→people→houses compounds
const MAX_CLUSTERS: usize = 48; // UNIFIED global town cap (live clusters + dormant regions) — world CONVERGES past it
const COLONY_R: f64 = 75.0; // a building within this of a centroid belongs to that town
const PEOPLE_PER_HOUSE: f64 = 2.8;
const PER_CLUSTER_HOUSE_CAP: f64 = 12.0;
const GOLDEN: f64 = 2.399963229728653; // golden angle → successive new towns ring out evenly
const REGION_SIZE: f64 = 200.0;
const SETTLEMENT_MIN: usize = 2; // ≥this many buildings = a settlement (vs wild land)
const COLONY_HOUSE_CAP: usize = 12; // a dormant town stops building here + only spreads once this full
const GROW_HOUSES_PER_PULSE: i64 = 6;
const SPREAD_POP_MIN: f64 = 30.0; // a dormant town must be this populous to peel a satellite
const SPREAD_FOUNDERS: f64 = 10.0; // people moved into each new satellite
const NOMAD_CAP: f64 = 3.0; // a wild (houseless) region holds at most this many wandering people
const PERSON: usize = 3; // index of Person in the FF kind order [rabbit, cat, kangaroo, person, lion, dinosaur]

fn creature_idx(kind: &str) -> Option<usize> {
    match kind {
        "rabbit" => Some(0),
        "cat" => Some(1),
        "kangaroo" => Some(2),
        "person" => Some(3),
        "lion" => Some(4),
        "dinosaur" => Some(5),
        _ => None,
    }
}
const KIND_NAME: [&str; 6] = ["rabbit", "cat", "kangaroo", "person", "lion", "dinosaur"];
fn is_building(kind: &str) -> bool {
    matches!(kind, "house" | "cabin" | "tower")
}
fn region_cell(v: f64) -> i64 {
    (v / REGION_SIZE).floor() as i64
}
fn pop_usize(counts: &[f64; 6]) -> [usize; 6] {
    let mut p = [0usize; 6];
    for i in 0..6 {
        p[i] = counts[i].max(0.0).round() as usize;
    }
    p
}

/// A dormant (offloaded) region's aggregate — the typed counterpart of JS `RegionAggregate`, keyed by its region cell
/// `(rx, rz)`. `counts` is FF-kind order; `statics` are its verbatim structures (houses → its development level).
#[derive(Clone, Debug, PartialEq)]
pub struct ERegion {
    pub rx: i64,
    pub rz: i64,
    pub counts: [f64; 6],
    pub gene: f64,
    pub last_tick: f64,
    pub statics: Vec<EObj>,
}
impl ERegion {
    fn build_count(&self) -> usize {
        self.statics.iter().filter(|s| is_building(&s.kind)).count()
    }
}

/// The catch-up result: the advanced live objects + dormant regions (JS replaces world.objects/world.regions), plus
/// the net counts for the welcome-back readout.
pub struct CatchUp {
    pub objects: Vec<EObj>,
    pub regions: Vec<ERegion>,
    pub creatures_added: i64,
    pub houses_added: i64,
}

/// A tiny deterministic jitter source (replaces JS `Math.random` in the placement layer). Each draw advances a key so
/// successive calls differ; seeded per catch-up so the result is reproducible.
struct Jit {
    seed: u32,
    k: i32,
}
impl Jit {
    fn f(&mut self) -> f64 {
        self.k = self.k.wrapping_add(1);
        rng::rand(self.seed, &[self.k])
    }
}

/// Count DISTINCT live settlements — buildings clustered by COLONY_R, ≥2 homes = a town (mirror of JS liveSettlementCount).
fn live_settlement_count(objects: &[EObj]) -> usize {
    let mut cl: Vec<(f64, f64, f64)> = Vec::new(); // (cx, cz, n)
    for o in objects {
        if !is_building(&o.kind) {
            continue;
        }
        if let Some(c) = cl.iter_mut().find(|c| (c.0 - o.pos[0]).powi(2) + (c.1 - o.pos[2]).powi(2) < COLONY_R * COLONY_R) {
            c.0 = (c.0 * c.2 + o.pos[0]) / (c.2 + 1.0);
            c.1 = (c.1 * c.2 + o.pos[2]) / (c.2 + 1.0);
            c.2 += 1.0;
        } else {
            cl.push((o.pos[0], o.pos[2], 1.0));
        }
    }
    cl.iter().filter(|c| c.2 >= 2.0).count()
}

fn dormant_settlement_count(regions: &HashMap<(i64, i64), ERegion>) -> usize {
    regions.values().filter(|r| r.build_count() >= SETTLEMENT_MIN).count()
}

/// THE ENTRY POINT. Advance a saved world by `elapsed_ms` of absence: the live slice + the dormant far world both
/// catch up, sharing the unified town cap. `water` = packed water zones `[px, pz, size, seed]×m` (for the water-safe
/// placement checks AND `grow_dormant_houses`); `terrain` grounds new homes. `id_prefix`/`seed` make the new ids
/// unique + the jitter reproducible. Mirrors the JS call order: full live pass, THEN the dormant pass.
pub fn catch_up(mut objects: Vec<EObj>, regions_in: Vec<ERegion>, water: &[f64], terrain: &[EFeat], elapsed_ms: f64, id_prefix: &str, seed: u32) -> CatchUp {
    let dt = (elapsed_ms / 1000.0).min(86_400.0); // model at most ~1 day (the logistic saturates anyway)
    let mut regions: HashMap<(i64, i64), ERegion> = regions_in.into_iter().map(|r| ((r.rx, r.rz), r)).collect();
    let water_t: Vec<(f64, f64, f64, f64)> = water.chunks_exact(4).map(|c| (c[0], c[1], c[2], c[3])).collect();

    let (mut creatures_added, mut houses_added) = (0i64, 0i64);
    if dt >= 30.0 {
        let dormant = dormant_settlement_count(&regions);
        let (c, h) = live_pass(&mut objects, &water_t, terrain, dormant, dt, id_prefix, &mut Jit { seed, k: 0 });
        creatures_added += c;
        houses_added += h;
        dormant_pass(&mut regions, &objects, water, dt);
    }

    let mut regions: Vec<ERegion> = regions.into_values().collect();
    regions.sort_by_key(|r| (r.rx, r.rz)); // stable order across the boundary (HashMap iteration is unordered)
    CatchUp { objects, regions, creatures_added, houses_added }
}

#[derive(Clone)]
struct Cluster {
    cx: f64,
    cz: f64,
    n: f64,
}

/// LIVE SLICE catch-up (port of JS world.ts `fastForward`): the chunked co-development spiral — relax populations
/// toward the breeding plateau (Rust ff_targets), materialise the deltas near each kind, then build homes / found
/// new towns at the expanding frontier (bounded by the unified cap). Returns (creatures, houses) added.
#[allow(clippy::too_many_arguments)]
fn live_pass(objects: &mut Vec<EObj>, water_t: &[(f64, f64, f64, f64)], terrain: &[EFeat], dormant_settlements: usize, dt: f64, id_prefix: &str, jit: &mut Jit) -> (i64, i64) {
    let cdt = dt / CHUNKS as f64;
    let (mut creatures, mut houses) = (0i64, 0i64);
    let (mut nid, mut hid) = (0i64, 0i64);
    let mut founded = 0i64; // persists across chunks → towns keep spiralling outward ever-wider

    for _chunk in 0..CHUNKS {
        // RE-SCAN each chunk — objects grew last chunk, so scale / per-kind anchors / the people target all rise.
        let mut count = [0usize; 6];
        let mut by_kind_pos: [Vec<(f64, f64)>; 6] = Default::default();
        let (mut gene_sum, mut gene_n) = (0.0f64, 0usize);
        let (mut min_x, mut max_x, mut min_z, mut max_z) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
        let mut builds = 0usize;
        for o in objects.iter() {
            let ci = creature_idx(&o.kind);
            if ci.is_none() && !is_building(&o.kind) {
                continue;
            }
            if let Some(i) = ci {
                count[i] += 1;
                by_kind_pos[i].push((o.pos[0], o.pos[2]));
                gene_sum += if o.gene > 0.0 { o.gene } else { 1.0 }; // JS `o.gene ?? 1` (unset → 1)
                gene_n += 1;
            } else {
                builds += 1;
            }
            min_x = min_x.min(o.pos[0]);
            max_x = max_x.max(o.pos[0]);
            min_z = min_z.min(o.pos[2]);
            max_z = max_z.max(o.pos[2]);
        }
        if !min_x.is_finite() {
            break; // an empty world → nothing to advance
        }
        let avg_gene = if gene_n > 0 { gene_sum / gene_n as f64 } else { 1.0 };
        let scale = world_area_scale(builds);
        let adv = ff_targets(&count, scale, cdt); // the whole relaxation is ONE Rust call (single source of truth)
        let mut target = [adv[0] as f64, adv[1] as f64, adv[2] as f64, adv[3] as f64, adv[4] as f64, adv[5] as f64];
        // PEOPLE grow toward the BREEDING PLATEAU (≈100×scale), not the house carrying-capacity, so the surplus
        // founds new towns instead of plateauing as one over-housed blob. Clamp aligned to the MAX_CLUSTERS cap.
        let plateau = 100.0 * scale.min(12.0);
        let p0 = (count[PERSON] as f64).max(0.5);
        target[PERSON] = target[PERSON].max((plateau / (1.0 + (plateau / p0 - 1.0) * (-0.0009 * cdt).exp())).round());

        // materialise deltas — add newcomers (evolved vigour) NEAR their kind, or trim the surplus from the tail.
        for ki in 0..6 {
            let have = count[ki] as i64;
            let want = target[ki].round() as i64;
            if want > have {
                for _ in 0..(want - have) {
                    let (x, z) = if !by_kind_pos[ki].is_empty() {
                        let a = by_kind_pos[ki][(jit.f() * by_kind_pos[ki].len() as f64) as usize % by_kind_pos[ki].len()];
                        (a.0 + (jit.f() - 0.5) * 24.0, a.1 + (jit.f() - 0.5) * 24.0)
                    } else {
                        (min_x + jit.f() * (max_x - min_x), min_z + jit.f() * (max_z - min_z))
                    };
                    let gene = (avg_gene - 0.05 + jit.f() * 0.1).clamp(GENE_MIN, GENE_MAX);
                    objects.push(EObj { id: format!("{id_prefix}c{nid}"), kind: KIND_NAME[ki].to_string(), pos: [x, 0.0, z], scale: [1.0; 3], rot: 0.0, color: None, keep: false, gene });
                    nid += 1;
                    creatures += 1;
                }
            } else if want < have {
                let mut drop = have - want;
                let mut i = objects.len();
                while i > 0 && drop > 0 {
                    i -= 1;
                    if creature_idx(&objects[i].kind) == Some(ki) {
                        objects.remove(i);
                        drop -= 1;
                        creatures -= 1;
                    }
                }
            }
        }

        // CITY GROWTH + SPREAD — raise homes toward people/PEOPLE_PER_HOUSE across towns; a full town's surplus
        // FOUNDS a new one ≥FOUND_GAP out (bounded by the unified cap). Houses lead, people follow next chunk.
        let blds_len = objects.iter().filter(|o| is_building(&o.kind)).count() as f64;
        let people = (count[PERSON] as f64).max(target[PERSON]);
        if blds_len >= 2.0 && people >= 6.0 {
            let mut clusters: Vec<Cluster> = Vec::new();
            for o in objects.iter() {
                if !is_building(&o.kind) {
                    continue;
                }
                if let Some(c) = clusters.iter_mut().find(|c| (c.cx - o.pos[0]).powi(2) + (c.cz - o.pos[2]).powi(2) < COLONY_R * COLONY_R) {
                    c.cx = (c.cx * c.n + o.pos[0]) / (c.n + 1.0);
                    c.cz = (c.cz * c.n + o.pos[2]) / (c.n + 1.0);
                    c.n += 1.0;
                } else {
                    clusters.push(Cluster { cx: o.pos[0], cz: o.pos[2], n: 1.0 });
                }
            }
            let target_homes = (people / PEOPLE_PER_HOUSE).ceil();
            let deficit = (target_homes - blds_len).max(0.0);
            let mut to_add = deficit.min(((cdt / 900.0) * (people / PEOPLE_PER_HOUSE)).round() + 1.0).min(600.0 - blds_len).min(50.0) as i64;
            let mut attempts = 0;
            while to_add > 0 && attempts < 600 {
                attempts += 1;
                // prefer the smallest UNDER-cap town (fill evenly); if all full, FOUND a new one.
                let into = clusters
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| c.n < PER_CLUSTER_HOUSE_CAP)
                    .min_by(|a, b| a.1.n.partial_cmp(&b.1.n).unwrap())
                    .map(|(i, _)| i);
                let into = match into {
                    Some(i) => i,
                    None => match found_cluster(&mut clusters, &mut founded, objects, dormant_settlements, water_t) {
                        Some(i) => i,
                        None => break, // world full / nowhere dry → stop
                    },
                };
                if place_in(into, &mut clusters, objects, water_t, terrain, id_prefix, &mut hid, jit) {
                    to_add -= 1;
                    houses += 1;
                }
            }
        }
    }

    // GRAVES while away — a small, time-proportional cemetery on the edge of town (cosmetic memory of the dead).
    let g_blds: Vec<(f64, f64)> = objects.iter().filter(|o| is_building(&o.kind)).map(|o| (o.pos[0], o.pos[2])).collect();
    let g_people = objects.iter().filter(|o| o.kind == "person").count();
    if g_blds.len() >= 2 && g_people >= 4 {
        let existing = objects.iter().filter(|o| o.kind == "grave").count() as i64;
        let mut to_add = ((dt / 1200.0).round() as i64).min(14 - existing).min(6);
        let cx = g_blds.iter().map(|b| b.0).sum::<f64>() / g_blds.len() as f64;
        let cz = g_blds.iter().map(|b| b.1).sum::<f64>() / g_blds.len() as f64;
        let mut g = 0i64;
        while to_add > 0 {
            let a = jit.f() * std::f64::consts::TAU;
            let r = 8.0 + jit.f() * 22.0;
            let gx = cx + a.cos() * r;
            let gz = cz + a.sin() * r;
            objects.push(EObj { id: format!("{id_prefix}g{g}"), kind: "grave".to_string(), pos: [gx, height_at(gx, gz, terrain), gz], scale: [1.0; 3], rot: jit.f() * std::f64::consts::TAU, color: None, keep: false, gene: 0.0 });
            g += 1;
            to_add -= 1;
        }
    }
    (creatures, houses)
}

/// FOUND a new town at the expanding frontier (golden-angle bearing at settled-radius + FOUND_GAP), bounded by the
/// unified cap. Moves a few founders from the densest town so the new one is a living colony. Returns its cluster index.
fn found_cluster(clusters: &mut Vec<Cluster>, founded: &mut i64, objects: &mut [EObj], dormant_settlements: usize, water_t: &[(f64, f64, f64, f64)]) -> Option<usize> {
    if clusters.len() + dormant_settlements >= MAX_CLUSTERS {
        return None; // world is FULL of towns — grow the ones we have
    }
    let n = clusters.len() as f64;
    let gcx = clusters.iter().map(|c| c.cx).sum::<f64>() / n;
    let gcz = clusters.iter().map(|c| c.cz).sum::<f64>() / n;
    let settled_r = clusters.iter().map(|c| (c.cx - gcx).hypot(c.cz - gcz)).fold(0.0, f64::max);
    let src = clusters.iter().fold(clusters[0].clone(), |best, c| if c.n > best.n { c.clone() } else { best });
    for attempt in 0..16 {
        let a = *founded as f64 * GOLDEN;
        let r = settled_r + FOUND_GAP * (1.0 + (attempt % 4) as f64 * 0.4);
        *founded += 1;
        let ax = ((gcx + a.cos() * r) / 8.0).round() * 8.0;
        let az = ((gcz + a.sin() * r) / 8.0).round() * 8.0;
        if in_water_seeded(water_t, ax, az) {
            continue;
        }
        if clusters.iter().any(|c| (c.cx - ax).powi(2) + (c.cz - az).powi(2) < FOUND_GAP * FOUND_GAP) {
            continue; // too close to an existing town
        }
        clusters.push(Cluster { cx: ax, cz: az, n: 0.0 });
        let mut moved = 0;
        for o in objects.iter_mut() {
            if moved >= 6 {
                break;
            }
            if o.kind != "person" || (o.pos[0] - src.cx).powi(2) + (o.pos[2] - src.cz).powi(2) > 80.0 * 80.0 {
                continue;
            }
            let fa = moved as f64 * GOLDEN;
            o.pos[0] = ax + fa.cos() * 4.0;
            o.pos[2] = az + fa.sin() * 4.0;
            moved += 1;
        }
        return Some(clusters.len() - 1);
    }
    None
}

/// Place ONE house beside cluster `ci`'s centroid (ring jitter), water-safe + not on a taken plot. Mirrors JS placeIn.
#[allow(clippy::too_many_arguments)]
fn place_in(ci: usize, clusters: &mut [Cluster], objects: &mut Vec<EObj>, water_t: &[(f64, f64, f64, f64)], terrain: &[EFeat], id_prefix: &str, hid: &mut i64, jit: &mut Jit) -> bool {
    let (cx, cz, n) = (clusters[ci].cx, clusters[ci].cz, clusters[ci].n);
    let ring = 10.0 + n.sqrt() * 8.0;
    let a = jit.f() * std::f64::consts::TAU;
    let gx = ((cx + a.cos() * ring * (0.5 + jit.f() * 0.5)) / 8.0).round() * 8.0;
    let gz = ((cz + a.sin() * ring * (0.5 + jit.f() * 0.5)) / 8.0).round() * 8.0;
    if objects.iter().any(|o| is_building(&o.kind) && (o.pos[0] - gx).abs() < 6.0 && (o.pos[2] - gz).abs() < 6.0) {
        return false; // plot taken
    }
    if in_water_seeded(water_t, gx, gz) {
        return false; // don't grow a home into a lake
    }
    objects.push(EObj { id: format!("{id_prefix}h{hid}"), kind: "house".to_string(), pos: [gx, height_at(gx, gz, terrain), gz], scale: [1.0; 3], rot: 0.0, color: None, keep: false, gene: 0.0 });
    *hid += 1;
    clusters[ci].cx = (cx * n + gx) / (n + 1.0);
    clusters[ci].cz = (cz * n + gz) / (n + 1.0);
    clusters[ci].n += 1.0;
    true
}

/// DORMANT far-world catch-up (port of streaming.ts `fastForwardDormantAway`): chunk the away span, advance every
/// dormant region (relax + develop), and spread FULL towns into new satellites — bounded by the shared cap.
fn dormant_pass(regions: &mut HashMap<(i64, i64), ERegion>, objects: &[EObj], water: &[f64], dt: f64) {
    if dt < 30.0 {
        return;
    }
    let cdt = dt / CHUNKS as f64;
    for chunk in 0..CHUNKS {
        let built = built_cells(objects, regions); // for the nomad clamp (settlementNear) — stable within a chunk
        let keys: Vec<(i64, i64)> = regions.keys().copied().collect();
        for k in &keys {
            if let Some(reg) = regions.get_mut(k) {
                advance_dormant(reg, &built, cdt, water);
            }
        }
        spread_dormant(regions, objects, chunk);
    }
}

/// Cells (live + dormant) that hold a building — the "is there a home near?" set guarding the nomad clamp.
fn built_cells(objects: &[EObj], regions: &HashMap<(i64, i64), ERegion>) -> HashSet<(i64, i64)> {
    let mut s = HashSet::new();
    for o in objects {
        if is_building(&o.kind) {
            s.insert((region_cell(o.pos[0]), region_cell(o.pos[2])));
        }
    }
    for (k, reg) in regions {
        if reg.build_count() > 0 {
            s.insert(*k);
        }
    }
    s
}
fn settlement_near(built: &HashSet<(i64, i64)>, cx: i64, cz: i64) -> bool {
    for dx in -1..=1 {
        for dz in -1..=1 {
            if built.contains(&(cx + dx, cz + dz)) {
                return true;
            }
        }
    }
    false
}

/// Advance ONE dormant region by `dt`: relax its populations toward the capacity ITS OWN homes support, evolve vigour,
/// and develop its settlement (build homes → raise capacity next call). Port of streaming.ts `advanceDormant`.
fn advance_dormant(reg: &mut ERegion, built: &HashSet<(i64, i64)>, dt: f64, water: &[f64]) {
    if dt <= 0.0 {
        return;
    }
    let builds = reg.build_count();
    let scale = world_area_scale(builds);
    let pop = pop_usize(&reg.counts);
    let adv = ff_targets(&pop, scale, dt);
    let mut next = [adv[0] as f64, adv[1] as f64, adv[2] as f64, adv[3] as f64, adv[4] as f64, adv[5] as f64];
    // PEOPLE need HOUSES — clamp to nomads only on TRULY wild land (no homes near); a settlement whose people were
    // offloaded here while its houses stayed live must be CONSERVED (settlementNear sees those homes → skips the clamp).
    if builds < SETTLEMENT_MIN && !settlement_near(built, reg.rx, reg.rz) {
        next[PERSON] = reg.counts[PERSON].min(NOMAD_CAP);
    }
    reg.gene = ff_gene(reg.gene, &pop, dt).clamp(GENE_MIN, GENE_MAX);
    reg.counts = next;
    if builds >= SETTLEMENT_MIN {
        grow_dormant(reg, next[PERSON], water);
    }
}

/// Develop a dormant settlement — build homes toward what its (just-FF'd) population supports, via the SAME Rust
/// placement as a live settler (`grow_dormant_houses`). Port of streaming.ts `growDormantSettlement`.
fn grow_dormant(reg: &mut ERegion, people: f64, water: &[f64]) {
    let builds = reg.build_count() as i64;
    let target = (people / PEOPLE_PER_HOUSE).floor().min(COLONY_HOUSE_CAP as f64) as i64;
    let want = GROW_HOUSES_PER_PULSE.min(target - builds);
    if want <= 0 {
        return;
    }
    let mut houses: Vec<f64> = Vec::new();
    for s in &reg.statics {
        if is_building(&s.kind) {
            houses.push(s.pos[0]);
            houses.push(s.pos[2]);
        }
    }
    if houses.len() < 2 {
        return;
    }
    let seed = ((reg.rx as f64 * 12.9898 + reg.rz as f64 * 78.233).sin()).abs(); // deterministic per region
    let ops = crate::worldgen::grow_dormant_houses(&houses, want as usize, water, seed);
    let mut i = 0;
    while i + 8 < ops.len() {
        // [OP_ADD, kind, x, z, rot, sx, sy, sz, color]. Y=0 — the renderer regrounds on wake; the glow uses heightAt.
        let slot = reg.statics.len();
        reg.statics.push(EObj {
            id: format!("dh{}_{}_{}", reg.rx, reg.rz, slot),
            kind: kind_str(ops[i + 1] as u8).to_string(),
            pos: [ops[i + 2], 0.0, ops[i + 3]],
            scale: [ops[i + 5], ops[i + 6], ops[i + 7]],
            rot: ops[i + 4],
            color: None,
            keep: false,
            gene: 0.0,
        });
        i += 9;
    }
}

/// DORMANT SPREAD — a FULL, populous town peels SPREAD_FOUNDERS into a NEW satellite FOUND_GAP away (+ 2 starter
/// homes), bounded by the shared cap. Port of streaming.ts `spreadDormantSettlements`.
fn spread_dormant(regions: &mut HashMap<(i64, i64), ERegion>, objects: &[EObj], chunk: usize) {
    let settled = dormant_settlement_count(regions);
    if settled + live_settlement_count(objects) >= MAX_CLUSTERS {
        return; // the live + dormant towns share ONE global cap
    }
    let keys: Vec<(i64, i64)> = regions.keys().copied().collect();
    for k in keys {
        let (people, cx, cz, gene, last_tick) = {
            let reg = &regions[&k];
            if reg.build_count() < COLONY_HOUSE_CAP || reg.counts[PERSON] < SPREAD_POP_MIN {
                continue;
            }
            let (mut sx, mut sz, mut nh) = (0.0, 0.0, 0.0);
            for s in &reg.statics {
                if is_building(&s.kind) {
                    sx += s.pos[0];
                    sz += s.pos[2];
                    nh += 1.0;
                }
            }
            if nh == 0.0 {
                continue;
            }
            (reg.counts[PERSON], sx / nh, sz / nh, reg.gene, reg.last_tick)
        };
        // satellite site: a golden-angle ring ≥FOUND_GAP out, seeded by (region cell, chunk) → deterministic, rings out
        let kh = (k.0.wrapping_mul(73_856_093) ^ k.1.wrapping_mul(19_349_663)).unsigned_abs() % 1000;
        let ang = kh as f64 / 1000.0 * std::f64::consts::TAU + chunk as f64 * GOLDEN;
        let satx = cx + ang.cos() * FOUND_GAP * 1.3;
        let satz = cz + ang.sin() * FOUND_GAP * 1.3;
        let skey = (region_cell(satx), region_cell(satz));
        if skey == k {
            continue; // landed in the parent's own region
        }
        if regions.get(&skey).is_some_and(|r| r.build_count() >= SETTLEMENT_MIN) {
            continue; // already a town there
        }
        // PEEL founders from the parent into the satellite (conserves population) + 2 starter homes → it's a settlement
        regions.get_mut(&k).unwrap().counts[PERSON] = people - SPREAD_FOUNDERS;
        let sat = regions.entry(skey).or_insert_with(|| ERegion { rx: skey.0, rz: skey.1, counts: [0.0; 6], gene, last_tick, statics: Vec::new() });
        sat.counts[PERSON] += SPREAD_FOUNDERS;
        if sat.build_count() < SETTLEMENT_MIN {
            for h in 0..2 {
                sat.statics.push(EObj { id: format!("ds{}_{}_{}", skey.0, skey.1, h), kind: "house".to_string(), pos: [satx + h as f64 * 8.0, 0.0, satz], scale: [1.0; 3], rot: 0.0, color: None, keep: true, gene: 0.0 });
            }
        }
    }
}

// ── binary boundary for regions (the live objects reuse engine_bin encode/decode_objs) ───────────────────────────────
fn parse_key(key: &str) -> (i64, i64) {
    let mut it = key.split(',');
    let rx = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let rz = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (rx, rz)
}

/// Decode dormant regions: `keys` ("rx,rz") + `num` stride 8 `[counts×6, gene, lastTick]` + `static_n` (statics per
/// region) + the statics as ONE concatenated obj SoA (engine_bin layout), split per region by `static_n`.
pub fn decode_regions(keys: &[String], num: &[f64], static_n: &[f64], s_ids: &[String], s_kinds: &[String], s_colors: &[String], s_num: &[f64]) -> Vec<ERegion> {
    let stats = decode_objs(s_ids, s_kinds, s_colors, s_num);
    let mut out = Vec::with_capacity(keys.len());
    let mut off = 0usize;
    for (i, key) in keys.iter().enumerate() {
        let c = &num[i * 8..i * 8 + 8];
        let (rx, rz) = parse_key(key);
        let sn = static_n.get(i).copied().unwrap_or(0.0) as usize;
        let statics = stats[off..(off + sn).min(stats.len())].to_vec();
        off += sn;
        out.push(ERegion { rx, rz, counts: [c[0], c[1], c[2], c[3], c[4], c[5]], gene: c[6], last_tick: c[7], statics });
    }
    out
}

/// Encode dormant regions → `(keys, num, static_n, s_ids, s_kinds, s_colors, s_num)` (inverse of `decode_regions`).
pub fn encode_regions(regions: &[ERegion]) -> (Vec<String>, Vec<f64>, Vec<f64>, Vec<String>, Vec<String>, Vec<String>, Vec<f64>) {
    let mut keys = Vec::with_capacity(regions.len());
    let mut num = Vec::with_capacity(regions.len() * 8);
    let mut static_n = Vec::with_capacity(regions.len());
    let mut all_stats: Vec<EObj> = Vec::new();
    for r in regions {
        keys.push(format!("{},{}", r.rx, r.rz));
        num.extend_from_slice(&[r.counts[0], r.counts[1], r.counts[2], r.counts[3], r.counts[4], r.counts[5], r.gene, r.last_tick]);
        static_n.push(r.statics.len() as f64);
        all_stats.extend(r.statics.iter().cloned());
    }
    let (s_ids, s_kinds, s_colors, s_num) = crate::engine_bin::encode_objs(&all_stats);
    (keys, num, static_n, s_ids, s_kinds, s_colors, s_num)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(id: &str, kind: &str, x: f64, z: f64) -> EObj {
        EObj { id: id.into(), kind: kind.into(), pos: [x, 0.0, z], scale: [1.0; 3], rot: 0.0, color: None, keep: false, gene: 1.0 }
    }
    fn person_count(objs: &[EObj]) -> usize {
        objs.iter().filter(|o| o.kind == "person").count()
    }
    fn building_count(objs: &[EObj]) -> usize {
        objs.iter().filter(|o| is_building(&o.kind)).count()
    }

    /// A small developed colony: a handful of homes + people + a wild rabbit herd, like the demo shape.
    fn colony() -> (Vec<EObj>, Vec<ERegion>) {
        let mut objs = Vec::new();
        for i in 0..6 {
            objs.push(obj(&format!("h{i}"), "house", (i % 3) as f64 * 8.0, (i / 3) as f64 * 8.0));
        }
        for i in 0..40 {
            objs.push(obj(&format!("p{i}"), "person", (i % 8) as f64 * 2.0, 20.0 + (i / 8) as f64 * 2.0));
        }
        for i in 0..30 {
            objs.push(obj(&format!("r{i}"), "rabbit", i as f64, -30.0));
        }
        (objs, Vec::new())
    }

    #[test]
    fn a_blink_away_is_a_noop() {
        let (objs, regs) = colony();
        let r = catch_up(objs.clone(), regs, &[], &[], 10_000.0, "t", 1);
        assert_eq!(r.creatures_added, 0);
        assert_eq!(r.houses_added, 0);
        assert_eq!(r.objects.len(), objs.len());
    }

    #[test]
    fn a_day_away_grows_the_colony_and_founds_distant_towns() {
        let (objs, regs) = colony();
        let p0 = person_count(&objs);
        let r = catch_up(objs, regs, &[], &[], 24.0 * 3600.0 * 1000.0, "sp", 7);
        let p1 = person_count(&r.objects);
        let far_homes = r.objects.iter().filter(|o| is_building(&o.kind) && o.pos[0].hypot(o.pos[2]) > 240.0).count();
        let far_people = r.objects.iter().filter(|o| o.kind == "person" && o.pos[0].hypot(o.pos[2]) > 240.0).count();
        assert!(p1 > p0 * 3 / 2, "people grew past the carrying cap toward the plateau ({p0} → {p1})");
        assert!(far_homes > 0, "founded DISTANT towns ({far_homes} homes >240 m out)");
        assert!(far_people > 0, "…that have RESIDENTS ({far_people} people >240 m out)");
        assert!(r.houses_added > 0);
    }

    #[test]
    fn repeated_jumps_converge_at_the_town_cap() {
        // THE bug this whole port hardens: jumping repeatedly must CONVERGE (≤MAX_CLUSTERS towns), not keep founding a
        // fresh batch each jump (user: "3×+1d → 84 settlements / 8.5k people, exponential").
        let (mut objs, mut regs) = colony();
        let mut last = 0usize;
        for jump in 0..5 {
            let r = catch_up(objs, regs, &[], &[], 24.0 * 3600.0 * 1000.0, &format!("j{jump}"), 100 + jump);
            objs = r.objects;
            regs = r.regions;
            last = live_settlement_count(&objs) + dormant_settlement_count(&regs.iter().cloned().map(|x| ((x.rx, x.rz), x)).collect());
        }
        assert!(last <= MAX_CLUSTERS, "the world CONVERGED at the town cap, got {last} settlements after 5 jumps");
        assert!(last >= 8, "…but it DID spread into many towns ({last})");
    }

    #[test]
    fn region_encode_decode_roundtrip() {
        let regions = vec![
            ERegion { rx: 1, rz: -2, counts: [10.0, 1.0, 3.0, 20.0, 0.0, 0.0], gene: 1.1, last_tick: 500.0, statics: vec![obj("h0", "house", 210.0, -390.0), obj("h1", "house", 218.0, -390.0)] },
            ERegion { rx: 0, rz: 0, counts: [5.0; 6], gene: 1.0, last_tick: 0.0, statics: vec![] },
        ];
        let (keys, num, sn, si, sk, sc, snum) = encode_regions(&regions);
        assert_eq!(decode_regions(&keys, &num, &sn, &si, &sk, &sc, &snum), regions);
    }

    #[test]
    fn dormant_far_town_develops_and_spreads() {
        // a single FULL dormant town (12 homes, populous) must DEVELOP (already capped) and SPREAD a satellite.
        let mut statics = Vec::new();
        for i in 0..12 {
            statics.push(obj(&format!("h{i}"), "house", 1000.0 + (i % 4) as f64 * 8.0, 1000.0 + (i / 4) as f64 * 8.0));
        }
        let regs = vec![ERegion { rx: region_cell(1000.0), rz: region_cell(1000.0), counts: [0.0, 0.0, 0.0, 56.0, 0.0, 0.0], gene: 1.0, last_tick: 0.0, statics }];
        let r = catch_up(Vec::new(), regs, &[], &[], 24.0 * 3600.0 * 1000.0, "d", 3);
        assert!(r.regions.len() >= 2, "the full town spread into ≥1 satellite ({} regions)", r.regions.len());
        let total_people: f64 = r.regions.iter().map(|x| x.counts[PERSON]).sum();
        assert!(total_people > 0.0, "people persisted across the spread");
    }
}
