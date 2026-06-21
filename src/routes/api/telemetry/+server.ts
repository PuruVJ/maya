// TELEMETRY — a window into the living sim FOR THE AGENT (designed for me, not the player). The sim (worldsim)
// records every kill / starvation / old-age death / birth / build as an event; rustSim batches them here. GET
// returns a digest (counts by type + recent events + tick span) so I can read what ACTUALLY happened in the
// ecosystem instead of guessing. Cloudflare D1-backed (local SQLite in `vite dev`); degrades to 204 with no DB.
import { json } from '@sveltejs/kit';
import type { RequestHandler } from './$types';

const ROLL = 5000; // keep only the most recent N events (a rolling window — D1 stays tiny)

async function ensure(db: D1Database): Promise<void> {
	await db.prepare('CREATE TABLE IF NOT EXISTS events (id INTEGER PRIMARY KEY AUTOINCREMENT, tick INTEGER, t TEXT, kind TEXT, x INTEGER, z INTEGER)').run();
}

type Ev = { t: string; kind: string; x: number; z: number };

export const POST: RequestHandler = async ({ platform, request }) => {
	const db = platform?.env?.DB;
	if (!db) return new Response(null, { status: 204 });
	await ensure(db);
	const body = (await request.json()) as { tick?: number; events?: Ev[] };
	const evs = (body.events ?? []).slice(0, 500); // cap one POST
	if (evs.length) {
		const tick = body.tick! | 0;
		const stmt = db.prepare('INSERT INTO events (tick, t, kind, x, z) VALUES (?, ?, ?, ?, ?)');
		await db.batch(evs.map((e) => stmt.bind(tick, String(e.t).slice(0, 16), String(e.kind).slice(0, 16), e.x | 0, e.z | 0)));
		await db.prepare('DELETE FROM events WHERE id <= (SELECT MAX(id) FROM events) - ?').bind(ROLL).run(); // roll the window
	}
	return new Response(null, { status: 204 });
};

export const GET: RequestHandler = async ({ platform }) => {
	const db = platform?.env?.DB;
	if (!db) return json({ note: 'no DB bound (vite dev without the platform proxy)' });
	await ensure(db);
	const summary = (await db.prepare('SELECT t, COUNT(*) AS n FROM events GROUP BY t ORDER BY n DESC').all()).results;
	const recent = (await db.prepare('SELECT tick, t, kind, x, z FROM events ORDER BY id DESC LIMIT 150').all()).results;
	const span = await db.prepare('SELECT MIN(tick) AS lo, MAX(tick) AS hi, COUNT(*) AS total FROM events').first();
	// also: counts in just the LAST 600 ticks, so I can see recent rates vs lifetime totals
	const recentWindow = (await db.prepare('SELECT t, COUNT(*) AS n FROM events WHERE tick >= (SELECT MAX(tick) FROM events) - 600 GROUP BY t ORDER BY n DESC').all()).results;
	return json({ summary, recentWindow, span, recent });
};

export const DELETE: RequestHandler = async ({ platform }) => {
	const db = platform?.env?.DB;
	if (!db) return new Response(null, { status: 204 });
	await db.prepare('DELETE FROM events').run();
	return new Response(null, { status: 204 });
};
