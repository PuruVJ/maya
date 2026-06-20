// Flat spatial hash grid for cheap neighbour queries. Cell size MUST equal the neighbour radius so
// any neighbour lands in the query cell or one of the 8 around it (the classic 3×3 sweep). Rebuilt
// once per frame; the query is callback-form (no per-agent array allocation → no GC churn).
// Generic over the stored item so it doesn't depend on the agent type (avoids an import cycle).
export class SpatialHashGrid<T> {
	readonly cell: number;
	#cells = new Map<number, T[]>();

	constructor(cell: number) {
		this.cell = cell;
	}

	// classic spatial-hash primes; rare key collisions just add a few far candidates the caller
	// filters out by real distance — correctness is preserved.
	#key(cx: number, cz: number): number {
		return (cx * 73856093) ^ (cz * 19349663);
	}

	clear(): void {
		this.#cells.clear();
	}

	insert(x: number, z: number, item: T): void {
		const k = this.#key(Math.floor(x / this.cell), Math.floor(z / this.cell));
		const bucket = this.#cells.get(k);
		if (bucket) bucket.push(item);
		else this.#cells.set(k, [item]);
	}

	// invoke `cb` for every item in the 3×3 block of cells around (x,z). Caller checks real distance.
	forEachNeighbor(x: number, z: number, cb: (item: T) => void): void {
		const cx = Math.floor(x / this.cell);
		const cz = Math.floor(z / this.cell);
		for (let gx = cx - 1; gx <= cx + 1; gx++) {
			for (let gz = cz - 1; gz <= cz + 1; gz++) {
				const bucket = this.#cells.get(this.#key(gx, gz));
				if (bucket) for (const it of bucket) cb(it);
			}
		}
	}
}
