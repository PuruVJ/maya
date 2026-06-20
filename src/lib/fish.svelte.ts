// Live registry of lake fish — kept OUT of the reactive graph + the agent manager (fish aren't world
// objects; they're ambient, like grass/birds, deterministic per water zone so a shared link shows the same
// shoal). Each LakeFish component registers its own `school` array and mutates the positions in place every
// frame; cats query `nearest()` to get lured toward a fish (the water obstacle then stops them at the bank,
// so they "fish" without wading in). Non-reactive on purpose — this is read in the agent hot path.
export interface FishPos {
	x: number;
	z: number;
}

class FishRegistry {
	#schools = new Set<FishPos[]>();

	/** A LakeFish registers its live position array; returns an unregister fn for cleanup. */
	register(school: FishPos[]): () => void {
		this.#schools.add(school);
		return () => this.#schools.delete(school);
	}

	/** Total live fish across all schools — for sizing the Rust sim's lure buffer. */
	get count(): number {
		let n = 0;
		for (const s of this.#schools) n += s.length;
		return n;
	}

	/** Visit every live fish position — used to feed the Rust sim's lure points (it owns no fish of its own). */
	forEach(cb: (f: FishPos) => void): void {
		for (const s of this.#schools) for (const f of s) cb(f);
	}

	/** Nearest fish to (x, z) within `maxD` metres, or null — used to lure cats to the water's edge. */
	nearest(x: number, z: number, maxD: number): FishPos | null {
		let best: FishPos | null = null;
		let bd = maxD * maxD;
		for (const school of this.#schools) {
			for (const f of school) {
				const d2 = (f.x - x) * (f.x - x) + (f.z - z) * (f.z - z);
				if (d2 < bd) ((bd = d2), (best = f));
			}
		}
		return best;
	}
}

export const fishRegistry = new FishRegistry();
