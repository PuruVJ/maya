// World persistence — 100% LOCAL (single-player; the game is "local & free"). The whole world lives in the player's
// own browser via IndexedDB — no server, no shared world. IndexedDB (vs localStorage) is async and handles the large
// World without stalling the frame.
//
// MULTI-KEY SCHEMA (docs/world-data-architecture.md "B"): the world is split across INDEPENDENTLY-WRITTEN keys instead
// of one ever-growing blob rewritten every second. The live slice (`meta`: bounded objects + zones + paths + terrain +
// header) is small and written each save; the UNBOUNDED part — `world.regions`, whose dormant aggregates accumulate as
// you explore over days — is sharded one key per region (`region:<id>`) and each is rewritten ONLY when its content
// changes (cheap signature diff). So leaving the world running for days no longer means re-serialising the entire
// explored history every second — a save touches the bounded live slice plus only the handful of regions that moved.
//
// load(): read the local cache → null (caller shows a fresh demo / seeded world). save(): write the local cache.
// Callers pass a PLAIN snapshot ($state.snapshot(world)) — this module is rune-free (plain .ts; the split/merge/sig
// helpers are pure + unit-tested in node, the thin IDB plumbing is exercised by the in-browser smoke).
import type { World, RegionAggregate, WorldObject } from './world';
import { packPersist, unpackPersist } from './structpack';

const DB_NAME = 'worldgen';
const STORE = 'worlds';
const META_KEY = 'meta'; // the live world MINUS regions, + the index of region keys present
const LEGACY_KEY = 'current'; // pre-split whole-world blob → migrated to the split schema on the first save after load
const REGION_PREFIX = 'region:';

/** A stored region: the aggregate with its statics BINARY-encoded (flat `soa` + parallel `ids` + verbatim `rest` for
 *  any non-structure). `statics` is the legacy (B-step-1) structured-clone field — read-tolerated, never written. */
export type StoredRegion = {
	counts: Record<string, number>;
	gene: number;
	lastTick: number;
	soa: Float64Array;
	ids: string[];
	rest: WorldObject[];
	statics?: WorldObject[];
};

/** What's stored under `meta`: the World header minus regions + objects, with `objects` BINARY-encoded (soa/ids/rest)
 *  and the index of region keys present. `objects?` is the legacy structured-clone field — read-tolerated. */
export type StoredMeta = Omit<World, 'regions' | 'objects'> & {
	regionKeys: string[];
	soa: Float64Array;
	ids: string[];
	rest: WorldObject[];
	objects?: WorldObject[];
};

// ── PURE split / merge / signature (no IDB — unit-tested) ──────────────────────────────────────────────────────────

function encodeRegion(a: RegionAggregate): StoredRegion {
	const { soa, ids, rest } = packPersist(a.statics);
	// `a` may be a live $state proxy (caller passes regions BY REFERENCE to avoid a per-second deep clone). soa/ids are
	// fresh detached arrays and statics are CODE_KIND structures (so `rest` is empty), but `counts` is still the proxy's
	// object — shallow-copy it to a plain object, or IndexedDB's structured-clone throws DataCloneError on the proxy and
	// the whole save silently aborts. (rest stays empty for statics; a stray non-structure would just skip that save.)
	return { counts: { ...a.counts }, gene: a.gene, lastTick: a.lastTick, soa, ids, rest };
}
/** Decode a stored region back to an aggregate. Tolerates the legacy `_statics` (B-step-1) structured-clone form. */
function decodeRegion(s: StoredRegion): RegionAggregate {
	const statics = s.soa ? [...unpackPersist(s.soa, s.ids), ...(s.rest ?? [])] : (s.statics ?? []);
	return { counts: s.counts, gene: s.gene, lastTick: s.lastTick, statics };
}

/** Split a world into the bounded, binary-encoded `meta` slice + its per-region encoded aggregates (the unbounded,
 *  separately-keyed part). The structure-heavy arrays (live objects, each region's statics) become flat typed arrays. */
export function splitWorld(w: World): { meta: StoredMeta; regions: Record<string, StoredRegion> } {
	const regionsIn = w.regions ?? {};
	const regions: Record<string, StoredRegion> = {};
	for (const [id, a] of Object.entries(regionsIn)) regions[id] = encodeRegion(a);
	const { soa, ids, rest } = packPersist(w.objects);
	const header = { ...w } as Partial<World>;
	delete header.objects;
	delete header.regions;
	const meta: StoredMeta = { ...(header as Omit<World, 'regions' | 'objects'>), regionKeys: Object.keys(regionsIn), soa, ids, rest };
	return { meta, regions };
}

/** Reassemble a World from its `meta` slice + the region aggregates fetched by key. Tolerates the legacy structured-
 *  clone shapes (B-step-1 `meta.objects` / `region._statics`) so worlds saved before the binary codec still load. */
export function mergeWorld(meta: StoredMeta, regions: Record<string, StoredRegion>): World {
	const { regionKeys: _drop, soa, ids, rest, objects: legacyObjects, ...header } = meta;
	const w = header as World;
	w.objects = soa ? [...unpackPersist(soa, ids), ...(rest ?? [])] : (legacyObjects ?? []);
	const decoded: Record<string, RegionAggregate> = {};
	for (const [id, s] of Object.entries(regions)) decoded[id] = decodeRegion(s);
	if (Object.keys(decoded).length) w.regions = decoded;
	return w;
}

/** Cheap change-signature for a STORED region. `lastTick` bumps every time a region re-collapses (the only moment its
 *  statics/counts change — see streaming.ts), so it + the count/length/gene fingerprint detects any real change without
 *  re-encoding the whole aggregate. A rare collision just leaves one region's blob stale until it next moves. */
export function regionSig(s: StoredRegion): string {
	let c = '';
	for (const k of Object.keys(s.counts).sort()) c += k + ':' + s.counts[k] + ';';
	const len = s.soa ? s.soa.length : (s.statics?.length ?? 0);
	return `${s.lastTick}|${len}|${(s.rest ?? []).length}|${Math.round(s.gene * 1e3)}|${c}`;
}

// ── IDB plumbing ───────────────────────────────────────────────────────────────────────────────────────────────────

// Per-region signatures last written to disk → skip rewriting unchanged regions on every 1 Hz save. Primed on load so
// the first save after a reload doesn't pointlessly rewrite every region. Module-singleton (worldStore is a singleton).
let lastSigs = new Map<string, string>();
// Serialise saves so the 1 Hz interval + debounced edit saves never build overlapping transactions / race `lastSigs`.
let saveChain: Promise<void> = Promise.resolve();

function openDb(): Promise<IDBDatabase> {
	return new Promise((resolve, reject) => {
		const req = indexedDB.open(DB_NAME, 1);
		req.onupgradeneeded = () => {
			const db = req.result;
			if (!db.objectStoreNames.contains(STORE)) db.createObjectStore(STORE);
		};
		req.onsuccess = () => resolve(req.result);
		req.onerror = () => reject(req.error);
	});
}

async function idbLoad(): Promise<World | null> {
	try {
		const db = await openDb();
		// the encoded regions as they sit on disk → used to prime the signature cache (so the first post-load save
		// only rewrites regions that actually moved). Empty for a legacy whole-world load → that save migrates all.
		const stored: Record<string, StoredRegion> = {};
		const world = await new Promise<World | null>((resolve, reject) => {
			const os = db.transaction(STORE, 'readonly').objectStore(STORE);
			const metaReq = os.get(META_KEY);
			metaReq.onsuccess = () => {
				const meta = metaReq.result as StoredMeta | undefined;
				if (!meta) {
					// no split schema yet → fall back to a legacy whole-world blob (migrated on the next save).
					const legacy = os.get(LEGACY_KEY);
					legacy.onsuccess = () => resolve((legacy.result as World) ?? null);
					legacy.onerror = () => reject(legacy.error);
					return;
				}
				const keys = meta.regionKeys ?? [];
				let pending = keys.length;
				if (!pending) return resolve(mergeWorld(meta, stored));
				for (const id of keys) {
					const r = os.get(REGION_PREFIX + id);
					r.onsuccess = () => {
						if (r.result) stored[id] = r.result as StoredRegion;
						if (--pending === 0) resolve(mergeWorld(meta, stored));
					};
					r.onerror = () => {
						if (--pending === 0) resolve(mergeWorld(meta, stored));
					};
				}
			};
			metaReq.onerror = () => reject(metaReq.error);
		});
		db.close();
		// prime the signature cache from the ON-DISK encoded regions → the first save after load only rewrites moves.
		lastSigs = new Map(Object.entries(stored).map(([id, s]) => [id, regionSig(s)]));
		return world;
	} catch {
		return null; // private mode / unsupported → no local cache
	}
}

async function idbWrite(meta: StoredMeta, regions: Record<string, StoredRegion>): Promise<void> {
	try {
		const db = await openDb();
		await new Promise<void>((resolve, reject) => {
			const tx = db.transaction(STORE, 'readwrite');
			const os = tx.objectStore(STORE);
			os.put(meta, META_KEY);
			const next = new Map<string, string>();
			for (const [id, agg] of Object.entries(regions)) {
				const sig = regionSig(agg);
				next.set(id, sig);
				if (lastSigs.get(id) !== sig) os.put(agg, REGION_PREFIX + id); // changed (or new) → write
			}
			for (const id of lastSigs.keys()) if (!next.has(id)) os.delete(REGION_PREFIX + id); // gone → drop its key
			os.delete(LEGACY_KEY); // migrated to the split schema → retire the old whole-world blob (no-op once gone)
			lastSigs = next;
			tx.oncomplete = () => resolve();
			tx.onerror = () => reject(tx.error);
		});
		db.close();
	} catch {
		/* best-effort cache */
	}
}

/** Load the world from the local IndexedDB cache (null → the caller shows a fresh demo world). */
export async function loadWorld(): Promise<World | null> {
	return idbLoad();
}

/** Persist the world to the local IndexedDB cache. Never blocks the frame; saves are serialised so concurrent
 *  callers (the 1 Hz interval + debounced edits) never race. Caller passes a detached $state.snapshot. */
export async function saveWorld(w: World): Promise<void> {
	// ENCODE SYNCHRONOUSLY at call time: splitWorld reads `w` (incl the LIVE regions the caller may pass BY REFERENCE)
	// with no await in between, so it can't race a streaming sleep/wake — the result is detached binary, and the chained
	// async write is safe. This lets the caller skip deep-cloning `world.regions` every second (the old whole-world
	// $state.snapshot scaled with explored history); now only the bounded live slice is cloned.
	const { meta, regions } = splitWorld(w);
	saveChain = saveChain.then(() => idbWrite(meta, regions)).catch(() => {});
	return saveChain;
}

async function idbClear(): Promise<void> {
	lastSigs = new Map();
	try {
		const db = await openDb();
		await new Promise<void>((resolve, reject) => {
			const tx = db.transaction(STORE, 'readwrite');
			tx.objectStore(STORE).clear(); // ALL keys, not just 'current'
			tx.oncomplete = () => resolve();
			tx.onerror = () => reject(tx.error);
		});
		db.close();
	} catch {
		/* private mode / unsupported → nothing to clear */
	}
}

/** Wipe the ENTIRE local world cache — every key (meta + all region:* + any legacy blob). The multi-key schema means a
 *  reset can't just drop one key. CHAINED through the save queue so any in-flight autosave finishes FIRST — otherwise a
 *  pending write lands after the clear and the old world survives the reset (the bug this fixes). The caller sets the
 *  `resetting` guard before calling, so no NEW save queues behind us. For a TRUE reset (↺). 100 % local. */
export async function clearWorld(): Promise<void> {
	saveChain = saveChain.then(idbClear, idbClear); // drain pending saves, then clear (even if a prior save rejected)
	return saveChain;
}
