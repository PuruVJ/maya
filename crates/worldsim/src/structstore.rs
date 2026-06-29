//! StructureStore — the Rust-owned BINARY source of truth for STRUCTURE geometry (houses/fences/graves/trees/…).
//!
//! It replaces the `JSON.stringify(world)` → wasm `JSON.parse` → full `world["objects"]` scan → JSON-return round
//! trip that every worldgen op does today (`Scene.svelte` `structWorld()` / `fenceWorld`), whose payload grows
//! unbounded as towns accumulate over days. Here, structures live in a struct-of-arrays arena indexed by a coarse
//! spatial grid, so each op reads only the LOCAL structures (O(local)) and the JS⇄wasm boundary carries small
//! binary deltas, never the whole world. Geometry is `f64` to reproduce the existing byte-pinned worldgen outputs
//! exactly (the render layer downcasts to f32). See `docs/world-data-architecture.md`.

use std::collections::HashMap;

// ── Structure kind codes — the SINGLE source of truth, mirrored in JS (kinds.ts `STRUCT_CODE`) and pinned by a
//    parity test. NON-creature placed kinds only; creatures live in the agent SoA, never here. ──────────────────
pub const SK_HOUSE: u8 = 0;
pub const SK_CABIN: u8 = 1;
pub const SK_MANOR: u8 = 2;
pub const SK_TOWER: u8 = 3;
pub const SK_WELL: u8 = 4;
pub const SK_FENCE: u8 = 5;
pub const SK_GRAVE: u8 = 6;
pub const SK_ROCK: u8 = 7;
pub const SK_TREE: u8 = 8;
pub const SK_PINE: u8 = 9;
pub const SK_BUSH: u8 = 10;
pub const SK_FLOWER: u8 = 11;
pub const SK_LAMP: u8 = 12;
pub const SK_BRIDGE: u8 = 13;
pub const SK_UNKNOWN: u8 = 255;

/// String kind → code, at the JSON-seeding boundary. Unknown/creature kinds map to SK_UNKNOWN (the store ignores them).
pub fn kind_code(s: &str) -> u8 {
    match s {
        "house" => SK_HOUSE,
        "cabin" => SK_CABIN,
        "manor" => SK_MANOR,
        "tower" => SK_TOWER,
        "well" => SK_WELL,
        "fence" => SK_FENCE,
        "grave" => SK_GRAVE,
        "rock" => SK_ROCK,
        "tree" => SK_TREE,
        "pine" => SK_PINE,
        "bush" => SK_BUSH,
        "flower" => SK_FLOWER,
        "lamp" => SK_LAMP,
        "bridge" => SK_BRIDGE,
        _ => SK_UNKNOWN,
    }
}

/// Code → canonical string, when emitting add-ops back to JS.
pub fn kind_str(c: u8) -> &'static str {
    match c {
        SK_HOUSE => "house",
        SK_CABIN => "cabin",
        SK_MANOR => "manor",
        SK_TOWER => "tower",
        SK_WELL => "well",
        SK_FENCE => "fence",
        SK_GRAVE => "grave",
        SK_ROCK => "rock",
        SK_TREE => "tree",
        SK_PINE => "pine",
        SK_BUSH => "bush",
        SK_FLOWER => "flower",
        SK_LAMP => "lamp",
        SK_BRIDGE => "bridge",
        _ => "?",
    }
}

/// `true` for the kinds `build_ops`/`grave_site` treat as BUILDINGS (house/cabin/manor/tower).
pub fn is_building(c: u8) -> bool {
    matches!(c, SK_HOUSE | SK_CABIN | SK_MANOR | SK_TOWER)
}
/// A HOME (house/cabin/manor) — what `settlement_ops` clusters into a town.
pub fn is_home(c: u8) -> bool {
    matches!(c, SK_HOUSE | SK_CABIN | SK_MANOR)
}
/// WALLED kinds (homes + towers + wells) — the points `settlement_ops` fits the perimeter ring around.
pub fn is_walled(c: u8) -> bool {
    is_home(c) || matches!(c, SK_TOWER | SK_WELL)
}

const GRID_CELL: f64 = 64.0; // spatial-grid cell edge (m); a local query visits only the cells its radius overlaps

/// One placed structure (the value type for add/get).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Structure {
    pub kind: u8,
    pub x: f64,
    pub z: f64,
    pub rot: f64, // stored EXACTLY as the source did (fences/settlement: degrees; graves: radians-read-as-degrees) — never normalised
    pub sx: f64,
    pub sy: f64,
    pub sz: f64,
    pub color: u32, // packed 0xRRGGBB; 0 = none/default
    pub keep: bool,
    pub region: u32, // region-cell key hash → lets persistence chunk by region (the days-away fix)
}

/// Struct-of-arrays arena + a coarse spatial grid. Slots are stable (a free-list recycles tombstones), so JS can
/// reference a structure by `slot` across calls (REMOVE comes back as a slot, never a string id).
pub struct StructureStore {
    kind: Vec<u8>,
    x: Vec<f64>,
    z: Vec<f64>,
    rot: Vec<f64>,
    sx: Vec<f64>,
    sy: Vec<f64>,
    sz: Vec<f64>,
    color: Vec<u32>,
    keep: Vec<bool>,
    region: Vec<u32>,
    alive: Vec<bool>,
    free: Vec<u32>,             // recycled tombstone slots (LIFO)
    grid: HashMap<i64, Vec<u32>>, // cell key → live slots in that cell
    live: usize,                // count of alive slots
}

#[inline]
fn cell_key(x: f64, z: f64) -> i64 {
    let cx = (x / GRID_CELL).floor() as i64;
    let cz = (z / GRID_CELL).floor() as i64;
    (cx << 32) ^ (cz & 0xffff_ffff)
}

impl StructureStore {
    pub fn new() -> Self {
        StructureStore {
            kind: Vec::new(),
            x: Vec::new(),
            z: Vec::new(),
            rot: Vec::new(),
            sx: Vec::new(),
            sy: Vec::new(),
            sz: Vec::new(),
            color: Vec::new(),
            keep: Vec::new(),
            region: Vec::new(),
            alive: Vec::new(),
            free: Vec::new(),
            grid: HashMap::new(),
            live: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.live
    }
    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    /// Drop every structure (keeps capacity). Used when JS re-seeds the whole store (load / reset).
    pub fn clear(&mut self) {
        self.kind.clear();
        self.x.clear();
        self.z.clear();
        self.rot.clear();
        self.sx.clear();
        self.sy.clear();
        self.sz.clear();
        self.color.clear();
        self.keep.clear();
        self.region.clear();
        self.alive.clear();
        self.free.clear();
        self.grid.clear();
        self.live = 0;
    }

    /// Insert a structure; returns its stable slot. Reuses a tombstoned slot when available (bounded growth).
    pub fn add(&mut self, s: Structure) -> u32 {
        let slot = if let Some(f) = self.free.pop() {
            let i = f as usize;
            self.kind[i] = s.kind;
            self.x[i] = s.x;
            self.z[i] = s.z;
            self.rot[i] = s.rot;
            self.sx[i] = s.sx;
            self.sy[i] = s.sy;
            self.sz[i] = s.sz;
            self.color[i] = s.color;
            self.keep[i] = s.keep;
            self.region[i] = s.region;
            self.alive[i] = true;
            f
        } else {
            let f = self.kind.len() as u32;
            self.kind.push(s.kind);
            self.x.push(s.x);
            self.z.push(s.z);
            self.rot.push(s.rot);
            self.sx.push(s.sx);
            self.sy.push(s.sy);
            self.sz.push(s.sz);
            self.color.push(s.color);
            self.keep.push(s.keep);
            self.region.push(s.region);
            self.alive.push(true);
            f
        };
        self.grid.entry(cell_key(s.x, s.z)).or_default().push(slot);
        self.live += 1;
        slot
    }

    /// Tombstone a slot (frees it for reuse). Idempotent — removing a dead/out-of-range slot is a no-op.
    pub fn remove(&mut self, slot: u32) {
        let i = slot as usize;
        if i >= self.alive.len() || !self.alive[i] {
            return;
        }
        self.alive[i] = false;
        self.live -= 1;
        if let Some(cell) = self.grid.get_mut(&cell_key(self.x[i], self.z[i])) {
            if let Some(p) = cell.iter().position(|&s| s == slot) {
                cell.swap_remove(p);
            }
        }
        self.free.push(slot);
    }

    /// Read a slot, or None if dead/out-of-range.
    pub fn get(&self, slot: u32) -> Option<Structure> {
        let i = slot as usize;
        if i >= self.alive.len() || !self.alive[i] {
            return None;
        }
        Some(Structure {
            kind: self.kind[i],
            x: self.x[i],
            z: self.z[i],
            rot: self.rot[i],
            sx: self.sx[i],
            sy: self.sy[i],
            sz: self.sz[i],
            color: self.color[i],
            keep: self.keep[i],
            region: self.region[i],
        })
    }

    /// Slots whose centre is within `r` of (x,z). Visits only the grid cells the radius overlaps → O(local), not
    /// O(world). Order is deterministic for a fixed cell set (sorted) so worldgen RNG/diffs reproduce exactly.
    pub fn query_radius(&self, x: f64, z: f64, r: f64) -> Vec<u32> {
        let r2 = r * r;
        let (cx0, cx1) = (((x - r) / GRID_CELL).floor() as i64, ((x + r) / GRID_CELL).floor() as i64);
        let (cz0, cz1) = (((z - r) / GRID_CELL).floor() as i64, ((z + r) / GRID_CELL).floor() as i64);
        let mut out: Vec<u32> = Vec::new();
        for cx in cx0..=cx1 {
            for cz in cz0..=cz1 {
                let key = (cx << 32) ^ (cz & 0xffff_ffff);
                if let Some(cell) = self.grid.get(&key) {
                    for &slot in cell {
                        let i = slot as usize;
                        let dx = self.x[i] - x;
                        let dz = self.z[i] - z;
                        if dx * dx + dz * dz <= r2 {
                            out.push(slot);
                        }
                    }
                }
            }
        }
        out.sort_unstable(); // deterministic visit order regardless of grid/HashMap iteration
        out
    }

    /// Every live slot (sorted). For ops that genuinely need a global view (e.g. the `new town ≥ FOUND_GAP from ANY
    /// building` check) — bounded by the live cap, so still cheap.
    pub fn live_slots(&self) -> Vec<u32> {
        let mut out: Vec<u32> = (0..self.alive.len() as u32).filter(|&s| self.alive[s as usize]).collect();
        out.sort_unstable();
        out
    }

    // ── Binary persistence: one flat little-endian blob of the LIVE structures (compaction drops tombstones), so
    //    IndexedDB stores bytes (no JSON). Layout: u32 count, then per-structure [kind u8][pad×3][x,z,rot,sx,sy,sz
    //    f64×6][color u32][keep u8][pad×3][region u32] = 64 bytes/struct. Round-trips losslessly. ────────────────
    const REC: usize = 64;

    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(4 + self.live * Self::REC);
        buf.extend_from_slice(&(self.live as u32).to_le_bytes());
        for i in 0..self.alive.len() {
            if !self.alive[i] {
                continue;
            }
            buf.push(self.kind[i]);
            buf.extend_from_slice(&[0u8; 3]);
            for v in [self.x[i], self.z[i], self.rot[i], self.sx[i], self.sy[i], self.sz[i]] {
                buf.extend_from_slice(&v.to_le_bytes());
            }
            buf.extend_from_slice(&self.color[i].to_le_bytes());
            buf.push(self.keep[i] as u8);
            buf.extend_from_slice(&[0u8; 3]);
            buf.extend_from_slice(&self.region[i].to_le_bytes());
        }
        buf
    }

    /// Replace the store's contents from a `serialize()` blob (used on load).
    pub fn deserialize(&mut self, buf: &[u8]) {
        self.clear();
        if buf.len() < 4 {
            return;
        }
        let n = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        let mut o = 4usize;
        let rd_f64 = |b: &[u8], o: usize| f64::from_le_bytes(b[o..o + 8].try_into().unwrap());
        let rd_u32 = |b: &[u8], o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        for _ in 0..n {
            if o + Self::REC > buf.len() {
                break;
            }
            let s = Structure {
                kind: buf[o],
                x: rd_f64(buf, o + 4),
                z: rd_f64(buf, o + 12),
                rot: rd_f64(buf, o + 20),
                sx: rd_f64(buf, o + 28),
                sy: rd_f64(buf, o + 36),
                sz: rd_f64(buf, o + 44),
                color: rd_u32(buf, o + 52),
                keep: buf[o + 56] != 0,
                region: rd_u32(buf, o + 60),
            };
            self.add(s);
            o += Self::REC;
        }
    }
}

impl Default for StructureStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(kind: u8, x: f64, z: f64) -> Structure {
        Structure { kind, x, z, rot: 0.0, sx: 1.0, sy: 1.0, sz: 1.0, color: 0, keep: false, region: 0 }
    }

    #[test]
    fn add_get_remove_recycles_slots() {
        let mut s = StructureStore::new();
        let a = s.add(mk(SK_HOUSE, 0.0, 0.0));
        let b = s.add(mk(SK_FENCE, 5.0, 5.0));
        assert_eq!(s.len(), 2);
        assert_eq!(s.get(a).unwrap().kind, SK_HOUSE);
        s.remove(a);
        assert_eq!(s.len(), 1);
        assert!(s.get(a).is_none());
        let c = s.add(mk(SK_WELL, 1.0, 1.0)); // should reuse a's slot
        assert_eq!(c, a, "free-list recycles the tombstoned slot");
        assert_eq!(s.len(), 2);
        s.remove(99); // out-of-range → no-op
        s.remove(b);
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn query_radius_is_local_and_deterministic() {
        let mut s = StructureStore::new();
        s.add(mk(SK_HOUSE, 0.0, 0.0));
        s.add(mk(SK_HOUSE, 10.0, 0.0));
        s.add(mk(SK_HOUSE, 300.0, 300.0)); // far — in another grid cell
        let near = s.query_radius(0.0, 0.0, 20.0);
        assert_eq!(near.len(), 2, "only the two nearby houses");
        // far house excluded
        assert!(s.query_radius(0.0, 0.0, 20.0).iter().all(|&slot| s.get(slot).unwrap().x < 100.0));
        // deterministic order (sorted)
        assert_eq!(near, { let mut v = near.clone(); v.sort_unstable(); v });
        // spanning the grid boundary still finds both
        let span = s.query_radius(70.0, 0.0, 80.0); // reaches back to x=0 across the 64m cell line
        assert!(span.len() >= 2);
    }

    #[test]
    fn serialize_round_trips_losslessly() {
        let mut s = StructureStore::new();
        s.add(Structure { kind: SK_FENCE, x: 12.5, z: -8.25, rot: 137.0, sx: 4.6, sy: 1.0, sz: 1.0, color: 0x7c5230, keep: true, region: 42 });
        s.add(mk(SK_GRAVE, 1.0, 2.0));
        let r = s.add(mk(SK_ROCK, 9.0, 9.0));
        s.remove(r); // tombstone dropped by serialize (compaction)
        let blob = s.serialize();
        let mut t = StructureStore::new();
        t.deserialize(&blob);
        assert_eq!(t.len(), 2, "compacted to live count");
        // the fence round-trips exactly (incl. the non-normalised rot + packed color)
        let f = t.query_radius(12.5, -8.25, 1.0);
        let fs = t.get(f[0]).unwrap();
        assert_eq!(fs.kind, SK_FENCE);
        assert_eq!(fs.rot, 137.0);
        assert_eq!(fs.color, 0x7c5230);
        assert!(fs.keep);
        assert_eq!(fs.region, 42);
        assert!((fs.x - 12.5).abs() < 1e-12 && (fs.z + 8.25).abs() < 1e-12);
    }

    #[test]
    fn kind_code_round_trips() {
        for k in ["house", "cabin", "manor", "tower", "well", "fence", "grave", "rock", "tree", "pine", "bush", "flower", "lamp", "bridge"] {
            assert_eq!(kind_str(kind_code(k)), k, "kind {k} round-trips");
        }
        assert_eq!(kind_code("person"), SK_UNKNOWN, "creatures are not structures");
        assert!(is_building(kind_code("house")) && !is_building(kind_code("fence")));
    }
}
