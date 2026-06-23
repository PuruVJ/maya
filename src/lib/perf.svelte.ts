// DYNAMIC RESOLUTION SCALING (DRS) — the "120fps no matter what" knob. Each frame we measure the real frame
// time and nudge the render PIXEL-RATIO toward a target budget: over budget (load spike — a 1000-cat crowd,
// heavy shaders) → drop resolution to hold the framerate; comfortably under → climb back toward crisp. This is
// the standard console technique for locking a framerate, and it's the right lever HERE because the scene is
// FILL-RATE bound (Retina dpr 2 = 4× the pixels, plus a 2048² shadow map every frame) — scaling pixels is the
// single biggest dial. Threlte applies `dpr` reactively to renderer.setPixelRatio, so we just drive `dpr`.
//
// Vsync-aware: rAF frame time is quantised to the display's refresh interval, so "headroom" is invisible once
// we're locked at the target. We therefore CLIMB by occasionally PROBING the resolution up; if that breaks the
// budget the drop rule pulls it straight back — so it converges just under the break point. We also auto-detect
// the display refresh from the fastest frames so targeting 120 on a 60 Hz screen renders crisp at 60 instead of
// tanking resolution chasing an unreachable 120.

class PerfScaler {
	target = $state(120); // desired fps (user-facing; the achievable rate is capped to the detected refresh)
	dpr = $state(1.5); // current adaptive pixel-ratio — what <Canvas> renders at
	auto = $state(true); // DRS on? (off → dpr pinned at #max, full native resolution)

	#max = 1.5; // crispest pixel-ratio (native, capped at 2 so 3× phones don't melt)
	#min = 0.75; // floor — below this it looks too soft; better to miss the target than go blurrier
	#ema = 1 / 60; // smoothed frame time (s)
	#minDt = 1 / 120; // fastest frame seen (≈ the vsync interval → the display refresh), slowly decays up
	#cooldown = 240; // warm-up before the FIRST adjustment (~4 s) — rides out the load/mount storm (objects revealing +
	// shaders compiling spike frame times) so DRS doesn't thrash the pixel-ratio (each change = a framebuffer resize =
	// a grey flash) while the scene is still settling. After this it adapts normally.
	#probeWait = 150; // frames between upward probes; BACKS OFF (×2, capped) each time a probe breaks budget + reverts
	#probedUp = false; // last change was an upward probe → if the very next adjustment is a drop, that probe FAILED
	#PROBE_MAX = 3600; // ~60 s ceiling on the probe interval — at the converged edge, probing (and its flash) goes rare

	constructor() {
		const full = typeof window !== 'undefined' ? Math.min(window.devicePixelRatio || 1, 2) : 1.5;
		this.#max = full;
		this.#min = Math.min(full, 0.75);
		this.dpr = full;
	}

	/** Cycle the fps target (UI affordance) — 120 → 60 → uncapped(240). */
	cycleTarget(): void {
		this.target = this.target >= 120 ? 60 : this.target >= 60 ? 240 : 120;
	}

	/** Feed the real frame dt (seconds) each render frame; adjusts `dpr` toward the target budget. */
	sample(dt: number): void {
		if (!(dt > 0) || dt > 0.5) return; // ignore stalls / tab-switches (huge dt would falsely trigger a drop)
		this.#ema += (dt - this.#ema) * 0.12; // ~8-frame smoothing
		// track the display refresh from the fastest frames (decay the estimate up so it follows a real change)
		if (dt > 0.004 && dt < this.#minDt) this.#minDt = dt;
		this.#minDt = Math.min(this.#minDt * 1.0005, 1 / 30);

		if (!this.auto) {
			if (this.dpr !== this.#max) this.dpr = this.#max;
			return;
		}
		if (this.#cooldown > 0) {
			this.#cooldown--;
			return;
		}

		const refresh = 1 / this.#minDt; // achievable ceiling (Hz)
		const budget = 1 / Math.min(this.target, refresh); // seconds/frame we're aiming to stay under
		if (this.#ema > budget * 1.15 && this.dpr > this.#min) {
			this.dpr = Math.max(this.#min, this.dpr - 0.09); // over budget → shed pixels fast (stays responsive)
			this.#cooldown = 24;
			// if this drop is undoing a probe we JUST made, that probe broke the budget → wait (much) longer before the
			// next one. At the converged edge this pushes the probe interval out toward a minute, so the framebuffer
			// realloc — and its 1-frame grey flash — stops happening every few seconds while you stand still.
			if (this.#probedUp) this.#probeWait = Math.min(this.#probeWait * 2, this.#PROBE_MAX);
			this.#probedUp = false;
		} else if (this.#ema < budget * 1.05 && this.dpr < this.#max) {
			this.dpr = Math.min(this.#max, this.dpr + 0.06); // meeting it with room → probe crisper (revert-safe)
			this.#cooldown = this.#probeWait; // backed-off interval → fewer probes → fewer resize flashes at steady state
			this.#probedUp = true;
		} else if (this.#probedUp) {
			this.#probeWait = 150; // a probe HELD (no revert) → conditions are good; reset so real headroom is found fast
			this.#probedUp = false;
		}
	}
}

export const perf = new PerfScaler();
