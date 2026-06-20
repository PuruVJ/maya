// The simulation clock — the single source of "now" for the living world. Decoupled from wall-clock: it
// accumulates real frame dt (scaled by `rate`) into whole FIXED-SIZE ticks, so the sim always advances in
// deterministic integer steps. Canonical time IS the integer `tick` (seconds = tick × DT). Paired with the
// stateless RNG (rng.ts, keyed by tick) the whole world becomes a pure function of (seed, tick): you can
// pause, speed up / slow down, single-step, and SEEK to any tick — the basis for time travel.
//
// Determinism contract: identical TICK ⇒ identical world (because all randomness is rand(seed, tick, …)
// and the sim's per-tick update is a pure function of the previous tick). Two machines may sit at
// different ticks at the same wall-clock instant — frame pacing varies — but seeking both to tick T shows
// the SAME world. That's the right guarantee for a real-time sim, and it's what lets a share link say
// "this world at tick T" and have it reproduce.
//
// Plain (no runes) on purpose: it's read every frame in the sim hot path — UI mirrors clock.tick/.time
// into its own $state for display (same pattern as agentManager). See docs/self-sustaining-world.md.

export const DT = 1 / 30; // seconds per sim tick (fixed timestep)
const MAX_CATCHUP = 6; // cap ticks emitted per advance() so a long frame stall can't spiral the sim
const EPS = 1e-9; // float slack: a full tick's worth of accumulated dt should never be lost to rounding

export type TickListener = (tick: number) => void;
export type SeekListener = (tick: number, from: number) => void;

export class SimClock {
	tick = 0; // integer sim step — the canonical clock position
	rate = 1; // tick-speed multiplier (2 = double-time, 10 = time-lapse); 0 ≈ paused
	playing = true;
	#acc = 0; // leftover sub-tick real-time (seconds), carried between frames
	#onTick: TickListener[] = [];
	#onSeek: SeekListener[] = [];

	/** Seconds of simulated time elapsed (tick × DT). */
	get time(): number {
		return this.tick * DT;
	}

	/** Not advancing right now (stopped or rate 0). */
	get paused(): boolean {
		return !this.playing || this.rate === 0;
	}

	/** Feed real elapsed seconds each frame; emits whole, rate-scaled ticks to onTick listeners.
	 *  Returns how many ticks advanced this call. */
	advance(realDt: number): number {
		if (this.paused || !(realDt > 0)) return 0;
		this.#acc += realDt * this.rate;
		let n = 0;
		while (this.#acc >= DT - EPS && n < MAX_CATCHUP) {
			this.#acc -= DT;
			n++;
		}
		if (this.#acc >= DT - EPS) this.#acc = 0; // dropped backlog beyond the catch-up cap (doesn't affect
		// reproducibility AT a given tick — only how fast wall-clock maps to ticks during a stall)
		for (let i = 0; i < n; i++) {
			this.tick++;
			this.#emit(this.tick);
		}
		return n;
	}

	/** Advance exactly n ticks now (manual single-step while paused, or scripted replay/fast-forward). */
	step(n = 1): void {
		const k = Math.max(0, Math.floor(n));
		for (let i = 0; i < k; i++) {
			this.tick++;
			this.#emit(this.tick);
		}
	}

	/** TIME TRAVEL: jump to an absolute tick. Fires onSeek(target, from) so the sim can reconstruct state
	 *  for that moment (restore the nearest checkpoint ≤ target, then replay forward — deterministic,
	 *  because every tick is rand(seed, tick, …)). Does NOT emit per-tick; reconstruction is the sim's job. */
	seek(targetTick: number): void {
		const t = Math.max(0, Math.floor(targetTick));
		const from = this.tick;
		if (t === from) return;
		this.tick = t;
		this.#acc = 0;
		for (const cb of this.#onSeek) cb(t, from);
	}

	pause(): void {
		this.playing = false;
	}
	play(): void {
		this.playing = true;
	}
	toggle(): void {
		this.playing = !this.playing;
	}
	/** Set the tick-speed multiplier (clamped ≥ 0). */
	setRate(r: number): void {
		this.rate = Math.max(0, r);
	}
	/** Hard reset to a tick (default 0) with no replay signal — for loading a fresh/shared world. */
	reset(tick = 0): void {
		this.tick = Math.max(0, Math.floor(tick));
		this.#acc = 0;
	}

	/** Subscribe to each whole tick advance; returns an unsubscribe fn. */
	onTick(cb: TickListener): () => void {
		this.#onTick.push(cb);
		return () => {
			const i = this.#onTick.indexOf(cb);
			if (i >= 0) this.#onTick.splice(i, 1);
		};
	}

	/** Subscribe to seeks (time-travel jumps); returns an unsubscribe fn. */
	onSeek(cb: SeekListener): () => void {
		this.#onSeek.push(cb);
		return () => {
			const i = this.#onSeek.indexOf(cb);
			if (i >= 0) this.#onSeek.splice(i, 1);
		};
	}

	#emit(tick: number): void {
		for (const cb of this.#onTick) cb(tick);
	}
}

/** App-wide simulation clock. */
export const clock = new SimClock();
