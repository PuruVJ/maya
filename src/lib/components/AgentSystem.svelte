<script lang="ts">
	// Headless: the single fixed-tick loop that drives the whole agent world. The simulation IS the Rust/WASM
	// core (crates/worldsim) — JS owns no agent sim anymore (see the rust-owns-all-compute memory). This just
	// pumps it.
	//
	// CLOCK-DRIVEN (docs/self-sustaining-world.md §1.6): instead of stepping by the raw render dt, we feed real
	// time into the SimClock, which accumulates it into whole FIXED-SIZE (30 Hz) ticks → frame-rate-independent
	// (identical at 30/60/144 fps), the basis for pause / time-lapse / seek. A long frame stall emits a few
	// catch-up ticks (capped inside advance()). Renderers interpolate between steps by clock.alpha, so motion
	// stays smooth at the display rate despite the 30 Hz sim.
	//
	// The wasm loads eagerly + mandatorily; until it's ready the agents simply don't tick (no JS fallback — if
	// the load fails they stay put and sim logs an error). Run `pnpm build:wasm` to produce the bundle.
	import { useTask, useThrelte } from '@threlte/core';
	import { clock, DT } from '$lib/clock';
	import { sim } from '$lib/sim';
	import { visibility } from '$lib/visibility.svelte';

	sim.init(); // eager + mandatory; agents idle until it resolves
	visibility.start(); // tab-away → pause (the gate below skips advancing; no dt accrues while hidden)

	// PAUSE RENDERING when tabbed away: flip Threlte to manual render-mode (its auto-render task stops) and back to
	// continuous on return, kicking one frame so the scene repaints immediately. The sim is frozen separately below.
	const { renderMode, invalidate } = useThrelte();
	$effect(() => {
		renderMode.set(visibility.visible ? 'always' : 'manual');
		if (visibility.visible) invalidate();
	});

	useTask((dt) => {
		if (sim.status() !== 'ready') return; // wasm still loading (or failed) → don't advance the sim
		if (!visibility.visible) return; // tabbed away → freeze the sim (no advance → no catch-up burst on return)
		const n = clock.advance(dt);
		if (n > 0) sim.step(n, DT); // BATCHED: advance all n ticks in ONE worker round-trip + ONE roster-diff/apply.
		// Was a per-tick loop — at 2× (60 Hz sim) that ran the O(agents) diff/apply + a 28 KB snapshot transfer twice
		// per frame, which is what dropped the frame rate in time-lapse. Cost is now flat regardless of sim speed.
	});
</script>
