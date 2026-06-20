// World persistence — replaces the old #w= URL-hash (which gzipped the whole world into the address bar on a
// 1 Hz timer, a periodic main-thread stall). No URL involved anymore.
//
// Two tiers, behind one interface so the call sites don't care:
//  1. SHARED world on Cloudflare KV (the agreed big-world.md target) via the /api/world route — the source of
//     truth once deployed with a WORLD_KV binding. This is the v0 "one shared world" (single blob, last-write-
//     wins); the per-region Durable-Object architecture (big-world.md §3.5) layers on top of this seam later.
//  2. LOCAL IndexedDB cache — async + handles the large World (incl. its terrain heightfield) without blocking
//     the frame (unlike synchronous localStorage). Used as an offline cache and the dev/no-KV fallback, so the
//     app persists and runs even before the backend is deployed.
//
// load(): shared (KV) first → local cache → null (caller shows the demo). save(): write the local cache instantly
// + best-effort sync to the shared world. Callers pass a PLAIN snapshot ($state.snapshot(world)) — this module is
// rune-free (plain .ts, unit-testable).
import type { World } from './world';

const API = '/api/world';
const DB_NAME = 'worldgen';
const STORE = 'worlds';
const KEY = 'current';

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

async function idbGet(): Promise<World | null> {
	try {
		const db = await openDb();
		const w = await new Promise<World | null>((resolve, reject) => {
			const req = db.transaction(STORE, 'readonly').objectStore(STORE).get(KEY);
			req.onsuccess = () => resolve((req.result as World) ?? null);
			req.onerror = () => reject(req.error);
		});
		db.close();
		return w;
	} catch {
		return null; // private mode / unsupported → no local cache
	}
}

async function idbPut(w: World): Promise<void> {
	try {
		const db = await openDb();
		await new Promise<void>((resolve, reject) => {
			const tx = db.transaction(STORE, 'readwrite');
			tx.objectStore(STORE).put(w, KEY);
			tx.oncomplete = () => resolve();
			tx.onerror = () => reject(tx.error);
		});
		db.close();
	} catch {
		/* best-effort cache */
	}
}

/** Load the world: shared (Cloudflare KV) first, else the local IndexedDB cache, else null (→ demo). */
export async function loadWorld(): Promise<World | null> {
	try {
		const res = await fetch(API, { method: 'GET' });
		if (res.ok) {
			const w = (await res.json()) as World | null;
			if (w && Array.isArray(w.objects)) {
				idbPut(w); // refresh the offline cache (fire-and-forget)
				return w;
			}
		}
	} catch {
		/* offline / dev / no KV bound → fall through to the local cache */
	}
	return idbGet();
}

/** Persist the world: instant local cache + best-effort sync to the shared KV world. Never blocks the frame. */
export async function saveWorld(w: World): Promise<void> {
	idbPut(w); // local, async, fire-and-forget
	try {
		await fetch(API, { method: 'PUT', headers: { 'content-type': 'application/json' }, body: JSON.stringify(w) });
	} catch {
		/* best-effort; the local cache holds it until the backend is reachable */
	}
}
