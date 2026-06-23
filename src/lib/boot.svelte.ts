// BOOT / splash gate. The first seconds of a session are the ugly part — the scene mounts, objects reveal, shaders
// compile, and the resolution scaler settles — which is exactly when the grey flicker happens. So we hold a splash
// over it and only reveal the world once it's actually settled. Dismiss when ALL of:
//   • a MINIMUM dwell has passed (so the splash never just blinks), AND
//   • the sim/WASM is ready, AND
//   • the frame rate has STABILISED (N consecutive frames without a load-storm spike),
// with a hard MAX failsafe so a slow device is never trapped behind it. The LLM is NOT part of this — it loads
// lazily on the first build command, long after the splash is gone (user: "splashscreen doesn't include the LLM").
class Boot {
	/** True once the world has settled and the splash should fade out. The UI reads this. */
	ready = $state(false);

	#start = 0; // ms timestamp of the first frame (lazy)
	#stable = 0; // consecutive "smooth" frames seen so far
	readonly #MIN_MS = 1000; // never flash by faster than this
	readonly #MAX_MS = 12000; // failsafe: reveal regardless after this, even if it never fully stabilises
	readonly #STABLE_DT = 0.05; // a frame under 50 ms (≥20 fps) counts as "not storming"
	readonly #STABLE_FRAMES = 24; // …this many in a row → the frame rate has stabilised (~0.5 s of smooth)

	/** Feed each rendered frame's dt (seconds) + whether the sim is ready. Flips `ready` when the world has settled. */
	tick(dt: number, simReady: boolean): void {
		if (this.ready) return;
		const now = typeof performance !== 'undefined' ? performance.now() : 0;
		if (this.#start === 0) this.#start = now;
		const elapsed = now - this.#start;
		// count consecutive smooth frames; any spike (a mount/compile hitch) resets the streak
		if (dt > 0 && dt < this.#STABLE_DT) this.#stable++;
		else this.#stable = 0;
		const stabilised = this.#stable >= this.#STABLE_FRAMES;
		if (elapsed >= this.#MAX_MS || (elapsed >= this.#MIN_MS && simReady && stabilised)) this.ready = true;
	}
}

/** The app-wide boot gate — AdaptiveResolution feeds it each frame; SplashScreen fades out when `boot.ready`. */
export const boot = new Boot();
