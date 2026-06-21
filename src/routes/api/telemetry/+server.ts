// TELEMETRY — a window into the living sim FOR THE AGENT (designed for me, not the player). The sim (worldsim)
// records every kill / starvation / old-age death / birth / build as an event; rustSim batches them here. GET
// returns a digest (counts by type + recent events + tick span) so I can read what ACTUALLY happened in the
// ecosystem instead of guessing. Cloudflare D1-backed (local SQLite in `vite dev`); degrades to 204 with no DB.
import { json } from '@sveltejs/kit';
import type { RequestHandler } from './$types';

const ROLL = 5000; // keep only the most recent N events (a rolling window — D1 stays tiny)
const POP_ROLL = 600; // and the most recent N population snapshots (the time series of who's alive)

async function ensure(db: D1Database): Promise<void> {
	await db.prepare('CREATE TABLE IF NOT EXISTS events (id INTEGER PRIMARY KEY AUTOINCREMENT, tick INTEGER, t TEXT, kind TEXT, x INTEGER, z INTEGER)').run();
	// population snapshots: one row per sample, columns per species (the sim's 6 Kinds) → trend over time
	await db.prepare('CREATE TABLE IF NOT EXISTS pops (id INTEGER PRIMARY KEY AUTOINCREMENT, tick INTEGER, rabbit INTEGER, cat INTEGER, kangaroo INTEGER, person INTEGER, lion INTEGER, dinosaur INTEGER, total INTEGER)').run();
}

type Ev = { t: string; kind: string; x: number; z: number };
type Pop = Record<string, number>;

export const POST: RequestHandler = async ({ platform, request }) => {
	const db = platform?.env?.DB;
	if (!db) return new Response(null, { status: 204 });
	await ensure(db);
	const body = (await request.json()) as { tick?: number; events?: Ev[]; pop?: Pop };
	const tick = body.tick! | 0;
	const evs = (body.events ?? []).slice(0, 500); // cap one POST
	if (evs.length) {
		const stmt = db.prepare('INSERT INTO events (tick, t, kind, x, z) VALUES (?, ?, ?, ?, ?)');
		await db.batch(evs.map((e) => stmt.bind(tick, String(e.t).slice(0, 16), String(e.kind).slice(0, 16), e.x | 0, e.z | 0)));
		await db.prepare('DELETE FROM events WHERE id <= (SELECT MAX(id) FROM events) - ?').bind(ROLL).run(); // roll the window
	}
	if (body.pop) {
		const p = body.pop;
		const g = (k: string) => (p[k] | 0) || 0; // missing species → 0
		const total = (p.rabbit | 0) + (p.cat | 0) + (p.kangaroo | 0) + (p.person | 0) + (p.lion | 0) + (p.dinosaur | 0);
		await db
			.prepare('INSERT INTO pops (tick, rabbit, cat, kangaroo, person, lion, dinosaur, total) VALUES (?, ?, ?, ?, ?, ?, ?, ?)')
			.bind(tick, g('rabbit'), g('cat'), g('kangaroo'), g('person'), g('lion'), g('dinosaur'), total)
			.run();
		await db.prepare('DELETE FROM pops WHERE id <= (SELECT MAX(id) FROM pops) - ?').bind(POP_ROLL).run();
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
	// POPULATION over time: the latest snapshot + every 5th sample of the series (a thinned trend so I can see a
	// boom/crash/steady-state at a glance without a wall of rows). Ordered oldest→newest.
	const popNow = await db.prepare('SELECT tick, rabbit, cat, kangaroo, person, lion, dinosaur, total FROM pops ORDER BY id DESC LIMIT 1').first();
	const popTrend = (
		await db.prepare('SELECT tick, rabbit, cat, kangaroo, person, lion, dinosaur, total FROM pops WHERE id % 5 = 0 ORDER BY id ASC LIMIT 120').all()
	).results;
	return json({ summary, recentWindow, span, popNow, popTrend, recent });
};

export const DELETE: RequestHandler = async ({ platform }) => {
	const db = platform?.env?.DB;
	if (!db) return new Response(null, { status: 204 });
	await db.prepare('DELETE FROM events').run();
	await db.prepare('DELETE FROM pops').run();
	return new Response(null, { status: 204 });
};
