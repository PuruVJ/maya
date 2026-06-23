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
	import { useTask } from '@threlte/core';
	import { clock, DT } from '$lib/clock';
	import { sim } from '$lib/sim';

	sim.init(); // eager + mandatory; agents idle until it resolves

	useTask((dt) => {
		if (sim.status() !== 'ready') return; // wasm still loading (or failed) → don't advance the sim
		const n = clock.advance(dt);
		for (let i = 0; i < n; i++) sim.step(DT);
	});
</script>
