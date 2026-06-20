<script lang="ts">
	// Headless: the single per-frame loop that steps every ambient agent (cats + people) through the
	// shared manager — one grid rebuild + one flock pass for all of them, instead of N separate loops.
	//
	// CLOCK-DRIVEN (docs/self-sustaining-world.md §1.6): instead of stepping by the raw render dt, we feed
	// real time into the SimClock, which accumulates it into whole FIXED-SIZE (30 Hz) ticks. The sim then
	// advances in deterministic integer steps — frame-rate-independent (identical behaviour at 30/60/144 fps)
	// and the basis for pause / time-lapse / seek. A long frame stall emits a few catch-up ticks (capped
	// inside advance()). Renderers interpolate between steps by clock.alpha, so motion stays smooth at the
	// display rate despite the 30 Hz sim.
	import { useTask } from '@threlte/core';
	import { agentManager } from '$lib/agents.svelte';
	import { clock, DT } from '$lib/clock';

	useTask((dt) => {
		const n = clock.advance(dt);
		for (let i = 0; i < n; i++) agentManager.tick(DT);
	});
</script>
