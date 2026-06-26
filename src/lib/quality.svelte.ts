// RENDER QUALITY TIER. Weak devices (phones — iOS Safari especially — few CPU cores, little memory) can't hold the
// frame rate with the full decorative load (particles, birds, butterflies, mist, weather) at Retina resolution, so
// they HANG. The `low` tier drops those decorative layers and hard-caps the resolution ceiling (Retina fill-rate is
// the #1 cost), trading eye-candy for a steady frame rate. Auto-detected once, overridable + persisted (the user can
// force either tier from the HUD). Reactive — components read `quality.low` and skip their heavy work when true.
import { perf } from './perf.svelte';

const KEY = 'maya-quality';

class Quality {
	/** True → the toned-down tier (decorative layers off, resolution capped). UI + Scene read this. */
	low = $state(false);
	#started = false;

	/** Detect (or restore) the tier once at app start, and push the resolution cap into the DRS scaler. */
	start(): void {
		if (this.#started || typeof window === 'undefined') return;
		this.#started = true;
		const stored = localStorage.getItem(KEY);
		if (stored === 'low' || stored === 'high') {
			this.low = stored === 'low'; // a manual choice wins over auto-detect
		} else {
			// AUTO-LOW ONLY on a clear touch-primary device (phone/tablet): coarse primary pointer AND actual touch
			// points. Desktops/laptops — even modest ones, and ESPECIALLY a high-end Mac — stay HIGH; flagging one low
			// caps the resolution to dpr 1 and blurs the screen for no reason (the regression this fixes). Weak desktops
			// can still opt into Lite from the HUD; we just never force it on them. CPU-core / RAM heuristics removed —
			// they false-positived (Macs report few "cores" under some conditions, Safari hides deviceMemory).
			const coarse = typeof matchMedia !== 'undefined' && matchMedia('(pointer: coarse)').matches;
			this.low = coarse && (navigator.maxTouchPoints ?? 0) > 0;
		}
		perf.setLowEnd(this.low);
	}

	/** Force a tier (HUD toggle) — persisted so the choice sticks across reloads. */
	set(low: boolean): void {
		this.low = low;
		try {
			localStorage.setItem(KEY, low ? 'low' : 'high');
		} catch {
			/* private mode → just don't persist */
		}
		perf.setLowEnd(low);
	}

	toggle(): void {
		this.set(!this.low);
	}
}

/** App-wide render-quality tier — `quality.low` gates the heavy decorative layers + caps resolution. */
export const quality = new Quality();
