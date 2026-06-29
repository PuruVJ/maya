// Binary structure packing for the worldgen StructureStore bridge (docs/world-data-architecture.md). Replaces the
// `JSON.stringify(world)` round-trip into the worldgen ops with flat typed arrays — no serialize, no parse, O(local).
//
// `kindCode` is the SINGLE JS↔Rust source of truth for structure kinds — it MUST match crates/worldsim/src/
// structstore.rs `SK_*` exactly (pinned by structpack.test.ts + the Rust kind_code/kind_str round-trip test).
import type { WorldObject, Zone } from './world';
import type { Op } from './engine';

/** code → kind, by index (the Rust SK_* order). NON-creature placed kinds only. */
export const CODE_KIND = ['house', 'cabin', 'manor', 'tower', 'well', 'fence', 'grave', 'rock', 'tree', 'pine', 'bush', 'flower', 'lamp', 'bridge'] as const;
const KIND_CODE: Record<string, number> = Object.fromEntries(CODE_KIND.map((k, i) => [k, i]));

/** SoA stride for `seed`: [kind, x, z, rot, sx, sy, sz, color, keep]. */
export const SOA_STRIDE = 9;
/** Op-stream stride: [op(0=add,1=remove), kind|slot, x, z, rot, sx, sy, sz, color]. */
export const OP_STRIDE = 9;
export const OP_ADD = 0;
export const OP_REMOVE = 1;

export function kindCode(k: string): number {
	const c = KIND_CODE[k];
	return c === undefined ? 255 : c;
}
export function kindStr(c: number): string {
	return CODE_KIND[c] ?? '?';
}
/** Is this kind a STRUCTURE (lives in the store) vs a creature / non-structure? */
export function isStructureKind(k: string): boolean {
	return k in KIND_CODE;
}

/** Pack the world's STRUCTURE objects (non-creatures) into the seed SoA, recording `idBySlot[slot] = obj.id` (slot =
 *  the index here, so a returned REMOVE slot maps back to the object). Bounded by the live structure budget (≤250). */
export function packStructures(objects: WorldObject[], idBySlot: string[]): Float64Array {
	idBySlot.length = 0;
	const lanes: number[] = [];
	for (const o of objects) {
		const code = KIND_CODE[o.kind];
		if (code === undefined) continue; // creatures + non-structures aren't in the store
		idBySlot.push(o.id);
		const s = o.scale ?? [1, 1, 1];
		const col = o.color ? parseInt(o.color.slice(1), 16) || 0 : 0;
		lanes.push(code, o.pos[0], o.pos[2], o.rot ?? 0, s[0], s[1], s[2], col, o.keep ? 1 : 0);
	}
	return Float64Array.from(lanes);
}

/** Pack water zones into `[px, pz, size, seed]×n` for the binary ops (`seed = waterSeed(id)`, supplied by the caller
 *  so the Rust `in_water_seeded` reproduces the organic shoreline without an id string crossing the boundary). */
export function packWaterZones(zones: Zone[] | undefined, waterSeed: (id: string) => number): Float64Array {
	const lanes: number[] = [];
	for (const z of zones ?? []) {
		if (z.material !== 'water') continue;
		lanes.push(z.pos[0], z.pos[2], z.size, waterSeed(z.id));
	}
	return Float64Array.from(lanes);
}

// ── PERSISTENCE codec (docs/world-data-architecture.md "B") — verbatim binary round-trip for STORED structures ──
// Distinct from the op-stream SoA above: this preserves pos[1] (Y) so a restored region static lands at its exact saved
// height (region statics aren't re-grounded on a streaming wake), and carries the object id (stored parallel) so edits/
// region-restore keep stable handles. Known structure kinds → flat typed array; creatures + any unknown kind → a
// verbatim `rest` sidecar (structured-clone), so the pack is LOSSLESS for an arbitrary mixed object list.
/** Persist record: [kindCode, x, y, z, rot, sx, sy, sz, color, keep]. */
export const PERSIST_STRIDE = 10;

/** Split a mixed object list for storage: known structures → `{soa, ids}` flat arrays; everything else (creatures,
 *  unknown kinds) → `rest` verbatim. `unpackPersist(soa, ids).concat(rest)` reconstitutes the list (structs first). */
export function packPersist(objects: WorldObject[]): { soa: Float64Array; ids: string[]; rest: WorldObject[] } {
	const ids: string[] = [];
	const rest: WorldObject[] = [];
	const lanes: number[] = [];
	for (const o of objects) {
		const code = kindCode(o.kind);
		if (code === 255) {
			rest.push(o); // creature / unknown kind → kept verbatim (all its fields survive)
			continue;
		}
		ids.push(o.id);
		const s = o.scale ?? [1, 1, 1];
		const col = o.color ? parseInt(o.color.slice(1), 16) || 0 : 0;
		lanes.push(code, o.pos[0], o.pos[1], o.pos[2], o.rot ?? 0, s[0], s[1], s[2], col, o.keep ? 1 : 0);
	}
	return { soa: Float64Array.from(lanes), ids, rest };
}

// ── Generator op stream (worldgen.rs GEN_* — the jzon-drop) — decode the binary forest/lake/city ops into engine Op[] ──
// Mirror of the Rust encoder: stride 10, tagged by op type, no strings (kindCode / material+shape codes / color u32 /
// a REMOVE-by-slot ref). `decodeGenOps` rebuilds the exact Op[] the jzon generators used to return, fed to applyOps.
export const GEN_STRIDE = 10;
const GOP_ADD = 0;
const GOP_ADDZONE = 1;
const GOP_REMOVE = 2;
const GOP_ADDPATH = 3;
const GEN_MATERIAL = ['water', 'path', 'plaza']; // material code → string (must match worldgen.rs MAT_*)
const GEN_SHAPE = ['blob', 'rect', 'ring']; // shape code → string

/** Decode the generator binary op stream into engine Op[]. `zoneIds[slot]` maps a REMOVE's zone slot back to its
 *  string id (slot = the zone's index in the water-zones array that was passed to the generator). */
export function decodeGenOps(stream: Float64Array | null, zoneIds: string[]): Op[] {
	if (!stream) return [];
	const ops: Op[] = [];
	for (let i = 0; i + GEN_STRIDE <= stream.length; i += GEN_STRIDE) {
		const t = stream[i];
		if (t === GOP_ADD) {
			const op: Op = { op: 'add', kind: kindStr(stream[i + 1]), pos: [stream[i + 2], 0, stream[i + 3]], scale: [stream[i + 4], stream[i + 5], stream[i + 6]], rot: stream[i + 7] };
			if (stream[i + 8]) op.color = '#' + Math.round(stream[i + 8]).toString(16).padStart(6, '0');
			ops.push(op);
		} else if (t === GOP_ADDZONE) {
			ops.push({ op: 'addZone', material: GEN_MATERIAL[stream[i + 4]] ?? 'water', shape: GEN_SHAPE[stream[i + 5]] ?? 'blob', pos: [stream[i + 1], 0, stream[i + 2]], size: stream[i + 3] });
		} else if (t === GOP_REMOVE) {
			const id = zoneIds[stream[i + 1]];
			if (id !== undefined) ops.push({ op: 'remove', id });
		} else if (t === GOP_ADDPATH) {
			ops.push({ op: 'addPath', material: GEN_MATERIAL[stream[i + 6]] ?? 'path', fromPos: [stream[i + 1], 0, stream[i + 2]], toPos: [stream[i + 3], 0, stream[i + 4]], width: stream[i + 5] });
		}
	}
	return ops;
}

/** Inverse of `packPersist`'s structure half: rebuild the WorldObjects from the flat SoA + parallel id list. */
export function unpackPersist(soa: Float64Array, ids: string[]): WorldObject[] {
	const out: WorldObject[] = [];
	for (let i = 0, slot = 0; i + PERSIST_STRIDE <= soa.length; i += PERSIST_STRIDE, slot++) {
		const o: WorldObject = {
			id: ids[slot],
			kind: kindStr(soa[i]),
			pos: [soa[i + 1], soa[i + 2], soa[i + 3]],
			rot: soa[i + 4],
			scale: [soa[i + 5], soa[i + 6], soa[i + 7]]
		};
		if (soa[i + 8]) o.color = '#' + Math.round(soa[i + 8]).toString(16).padStart(6, '0');
		if (soa[i + 9]) o.keep = true;
		out.push(o);
	}
	return out;
}
