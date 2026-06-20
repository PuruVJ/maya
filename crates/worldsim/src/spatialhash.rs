//! Flat spatial hash grid — port of `src/lib/spatialhash.ts`. Cell size = the neighbour radius, so any
//! neighbour lands in the query cell or one of the 8 around it (the classic 3×3 sweep). Stores ENTITY
//! INDICES (into the sim's struct-of-arrays buffers) instead of a generic item.
//!
//! The sweep visits cells in a FIXED order (gx outer, gz inner) and items within a cell in insertion order,
//! so neighbour iteration is deterministic + thread-count-invariant (§6.8) — the order-sensitive parts of the
//! sim depend on it. The key `(cx*p1) ^ (cz*p2)` is computed in wrapping `i32`, which reproduces JS's
//! `ToInt32` of the same expression for any realistic cell coordinate (where `cx*p` stays exact in an f64,
//! i.e. `|cx| < 2^26` ≈ a 268-million-metre span at cell 4 — far beyond any world). Hash COLLISIONS are
//! intentional + reproduced bit-for-bit (rare; the caller filters by real distance), e.g. cells (-1,-1) and
//! (1,1) share a bucket — so a query can legitimately return the same index twice.

use std::collections::HashMap;

pub struct SpatialHashGrid {
    cell: f64,
    cells: HashMap<i32, Vec<u32>>,
}

impl SpatialHashGrid {
    pub fn new(cell: f64) -> Self {
        Self { cell, cells: HashMap::new() }
    }

    #[inline]
    fn key(cx: i32, cz: i32) -> i32 {
        cx.wrapping_mul(73_856_093) ^ cz.wrapping_mul(19_349_663)
    }

    #[inline]
    fn cell_of(&self, x: f64, z: f64) -> (i32, i32) {
        ((x / self.cell).floor() as i32, (z / self.cell).floor() as i32)
    }

    pub fn clear(&mut self) {
        self.cells.clear();
    }

    pub fn insert(&mut self, x: f64, z: f64, item: u32) {
        let (cx, cz) = self.cell_of(x, z);
        self.cells.entry(Self::key(cx, cz)).or_default().push(item);
    }

    /// Invoke `cb` for every item in the 3×3 block of cells around (x,z), in fixed sweep order. The caller
    /// filters by real squared distance (a hash collision just adds a few far candidates).
    pub fn for_each_neighbor<F: FnMut(u32)>(&self, x: f64, z: f64, mut cb: F) {
        let (cx, cz) = self.cell_of(x, z);
        for gx in (cx - 1)..=(cx + 1) {
            for gz in (cz - 1)..=(cz + 1) {
                if let Some(bucket) = self.cells.get(&Self::key(gx, gz)) {
                    for &it in bucket {
                        cb(it);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(g: &SpatialHashGrid, x: f64, z: f64) -> Vec<u32> {
        let mut out = Vec::new();
        g.for_each_neighbor(x, z, |it| out.push(it));
        out
    }

    // Reference sequences captured from src/lib/spatialhash.ts (cell 4) — MUST match bit-for-bit, including
    // the (-1,-1)/(1,1) hash-collision duplicates. Determinism here = identical sims across worker/replay.
    #[test]
    fn neighbor_parity() {
        let mut g = SpatialHashGrid::new(4.0);
        let pts = [
            (0.0, 0.0),
            (1.0, 1.0),
            (5.0, 5.0),
            (-3.0, -3.0),
            (2.0, -2.0),
            (0.5, 0.5),
            (-5.0, 5.0),
            (3.9, 3.9),
        ];
        for (i, &(x, z)) in pts.iter().enumerate() {
            g.insert(x, z, i as u32);
        }
        assert_eq!(collect(&g, 0.0, 0.0), vec![2, 3, 4, 0, 1, 5, 7, 2, 3]);
        assert_eq!(collect(&g, 5.0, 5.0), vec![0, 1, 5, 7, 2, 3]);
        assert_eq!(collect(&g, -3.0, -3.0), vec![2, 3, 4, 0, 1, 5, 7]);
        assert_eq!(collect(&g, -5.0, 5.0), vec![6]);
        assert_eq!(collect(&g, 99.0, 99.0), Vec::<u32>::new());
    }

    #[test]
    fn key_parity() {
        assert_eq!(SpatialHashGrid::key(0, 0), 0);
        assert_eq!(SpatialHashGrid::key(-1, -1), 88_192_194);
        assert_eq!(SpatialHashGrid::key(1, 1), 88_192_194); // collides with (-1,-1) — reproduced exactly
        assert_eq!(SpatialHashGrid::key(-2, 1), -166_373_415);
    }
}
