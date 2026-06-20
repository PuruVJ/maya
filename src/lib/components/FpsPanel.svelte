<script lang="ts">
	// Lightweight on-screen FPS / frame-time meter (top-centre HUD). A plain requestAnimationFrame loop so it
	// measures the real display cadence independent of the Threlte render graph; recomputes ~3×/s (smoothed,
	// no jitter via tabular-nums). Also tracks the worst frame in each window → surfaces hitches the average
	// hides (e.g. a GC stall or a scatter rebuild). Colour-coded green ≥50 / amber ≥30 / red below. Pure
	// overlay, pointer-events-none. Handy right now to watch the new fixed-30 Hz sim's render cost.
	import { onMount } from 'svelte';

	let fps = $state(0);
	let ms = $state(0);
	let lo = $state(0); // worst (lowest) instantaneous fps in the recent window

	onMount(() => {
		let last = performance.now();
		let frames = 0;
		let acc = 0;
		let worst = Infinity;
		let raf = 0;
		const loop = (now: number) => {
			const dt = now - last;
			last = now;
			if (dt > 0) {
				frames++;
				acc += dt;
				const inst = 1000 / dt;
				if (inst < worst) worst = inst;
				if (acc >= 333) {
					fps = Math.round(frames / (acc / 1000));
					ms = Math.round((acc / frames) * 10) / 10;
					lo = Math.round(worst);
					frames = 0;
					acc = 0;
					worst = Infinity;
				}
			}
			raf = requestAnimationFrame(loop);
		};
		raf = requestAnimationFrame(loop);
		return () => cancelAnimationFrame(raf);
	});

	const col = $derived(fps >= 50 ? '#74e39b' : fps >= 30 ? '#ffd56b' : '#ff6b6b');
</script>

<div
	class="pointer-events-none fixed left-1/2 top-3 z-30 flex -translate-x-1/2 items-center gap-2 rounded-full bg-black/45 px-3 py-1 font-mono text-xs font-semibold tabular-nums text-white/70 backdrop-blur"
>
	<span style:color={col}>{fps}<span class="opacity-60"> fps</span></span>
	<span class="text-white/30">·</span>
	<span>{ms}<span class="opacity-60"> ms</span></span>
	<span class="text-white/30">·</span>
	<span class="text-white/45">min {lo}</span>
</div>
