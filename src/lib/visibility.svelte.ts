// Tab visibility as a tiny rune-store. When you tab away (document hidden), the world PAUSES — the sim stops
// stepping and the render frameloop idles — and both resume the moment you come back. Saves battery/CPU while the
// tab is in the background, and avoids a catch-up burst on return (a hidden tab accrues no sim ticks at all).
class Visibility {
	/** True while the tab is in the foreground. Drives the sim gate (AgentSystem) + the Canvas frameloop (+page). */
	visible = $state(true);
	#started = false;

	/** Begin listening (idempotent; no-op outside the browser). Call once at app start. */
	start(): void {
		if (this.#started || typeof document === 'undefined') return;
		this.#started = true;
		this.visible = !document.hidden;
		document.addEventListener('visibilitychange', () => {
			this.visible = !document.hidden;
		});
	}
}

/** The app-wide tab-visibility store — `visibility.visible` is false while tabbed away. */
export const visibility = new Visibility();
