// World persistence — 100% LOCAL (single-player; the game is "local & free"). The whole world lives in the player's
// own browser via IndexedDB — no server, no shared world. IndexedDB (vs localStorage) is async and handles the large
// World (incl. its terrain heightfield) without stalling the frame.
//
// load(): read the local cache → null (caller shows a fresh demo / seeded world). save(): write the local cache.
// Callers pass a PLAIN snapshot ($state.snapshot(world)) — this module is rune-free (plain .ts, unit-testable).
import type { World } from './world';

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

/** Load the world from the local IndexedDB cache (null → the caller shows a fresh demo world). */
export async function loadWorld(): Promise<World | null> {
	return idbGet();
}

/** Persist the world to the local IndexedDB cache. Never blocks the frame. */
export async function saveWorld(w: World): Promise<void> {
	await idbPut(w);
}
