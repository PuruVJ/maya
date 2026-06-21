// Shared-world persistence backend (Cloudflare D1). The client (src/lib/worldStore.ts) GETs the world on load
// and PUTs it (debounced) on edit; this owns the database. v0 is ONE shared world (single row, last-write-wins) —
// the seam the per-region Durable-Object architecture (docs/big-world.md §3.5) layers on top of later.
//
// The world is stored as the already-JSON-encoded blob the client sends — D1 is dumb storage here, it never
// parses the schema. If no DB is bound (e.g. `vite dev` without the platform proxy), every handler is a 204
// no-op and the client transparently falls back to its local IndexedDB cache, so the app still works offline.
import type { RequestHandler } from './$types';

const ID = 'current'; // single shared world for now; D1 will key per-region/per-user when big-world lands

async function ensure(db: D1Database): Promise<void> {
	await db.prepare('CREATE TABLE IF NOT EXISTS worlds (id TEXT PRIMARY KEY, data TEXT NOT NULL, updated INTEGER NOT NULL)').run();
}

export const GET: RequestHandler = async ({ platform }) => {
	const db = platform?.env?.DB;
	if (!db) return new Response(null, { status: 204 }); // no D1 bound → client uses its local cache
	await ensure(db);
	const row = await db.prepare('SELECT data FROM worlds WHERE id = ?').bind(ID).first<{ data: string }>();
	if (!row?.data) return new Response(null, { status: 204 });
	return new Response(row.data, { headers: { 'content-type': 'application/json' } });
};

export const PUT: RequestHandler = async ({ platform, request }) => {
	const db = platform?.env?.DB;
	if (!db) return new Response(null, { status: 204 });
	await ensure(db);
	const data = await request.text(); // the World, already JSON-serialized by the client
	if (data.length > 8_000_000) return new Response('world too large', { status: 413 }); // ~8 MB sanity cap
	await db.prepare('INSERT OR REPLACE INTO worlds (id, data, updated) VALUES (?, ?, ?)').bind(ID, data, Date.now()).run();
	return new Response(null, { status: 204 });
};

export const DELETE: RequestHandler = async ({ platform }) => {
	const db = platform?.env?.DB;
	if (!db) return new Response(null, { status: 204 });
	await ensure(db);
	await db.prepare('DELETE FROM worlds WHERE id = ?').bind(ID).run();
	return new Response(null, { status: 204 });
};
