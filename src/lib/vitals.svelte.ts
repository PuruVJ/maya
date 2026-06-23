// LIVE VITALS — a tiny rolling record of recent BIRTHS per species, the numerator for the HUD's per-species TFR.
// Scene records each birth as it materialises (the authoritative `sim.drainBirths` feed — one entry per real birth);
// EcoStats reads `ratePerSec(kind, now)` once a second. Timestamps are in SIM SECONDS (sim.tick()/tickHz), NOT wall
// time, so the rate stays correct at any time-lapse speed (a 2× clock makes births arrive 2× faster over 2× the sim
// seconds → same rate). Pure bookkeeping; no reactivity needed (EcoStats already polls on a 1 Hz timer).
const WINDOW = 90; // sim-seconds of history kept — long enough to smooth a bursty, clumpy birth process

class Vitals {
	#log: { t: number; kind: string }[] = [];

	/** Record one birth (called from Scene on each drained birth). `now` = current sim seconds. */
	birth(kind: string, now: number): void {
		this.#log.push({ t: now, kind });
	}

	/** Births per SIM-SECOND per species over the trailing WINDOW, evicting stale entries. `now` = sim seconds. */
	ratePerSec(now: number): Record<string, number> {
		const cut = now - WINDOW;
		while (this.#log.length && this.#log[0].t < cut) this.#log.shift();
		// the effective window is min(WINDOW, time observed) so a freshly-loaded world doesn't read an absurdly low
		// rate (dividing recent births by the full 90 s before 90 s have even elapsed)
		const span = Math.min(WINDOW, Math.max(1, now - (this.#log[0]?.t ?? now)));
		const out: Record<string, number> = {};
		for (const e of this.#log) out[e.kind] = (out[e.kind] ?? 0) + 1;
		for (const k in out) out[k] /= span;
		return out;
	}
}

/** App-wide singleton — Scene feeds it, EcoStats reads it. */
export const vitals = new Vitals();
