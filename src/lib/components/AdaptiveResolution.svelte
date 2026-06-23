<script lang="ts">
	// Drives dynamic resolution scaling (DRS): feeds the real per-frame dt to the perf scaler every render frame,
	// which nudges the render pixel-ratio to hold the fps target. No markup — it just needs to live inside the
	// Threlte <Canvas> so useTask fires once per rendered frame. See $lib/perf.svelte.ts.
	import { useTask } from '@threlte/core';
	import { perf } from '$lib/perf.svelte';
	import { llm } from '$lib/llm.svelte';
	import { boot } from '$lib/boot.svelte';
	import { sim } from '$lib/sim';

	// skip while a build is generating: those frames are deliberately throttled (the model saturates the GPU and
	// +page pins dpr to 0.6), so sampling them would teach the scaler the wrong budget.
	useTask((dt) => {
		if (!llm.busy) perf.sample(dt);
		boot.tick(dt, sim.status() === 'ready'); // splash gate: feed every frame's dt → reveal once the world settles
	});
</script>
