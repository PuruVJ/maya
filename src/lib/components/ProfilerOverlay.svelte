<script lang="ts">
	// Fixed HUD readout of the frame profiler (lives OUTSIDE the Threlte canvas — it's plain DOM). Toggle with 'p'.
	// Read it as: high GPU ms → the GPU is the bottleneck (shadows / grass / triangles); high render-cpu or draws →
	// too many draw calls (more instancing); frameMax spiking on a region crossing → the wake-storm materialise spike.
	import { profiler } from '$lib/profiler.svelte';
	const r1 = (n: number) => (n < 0 ? 'n/a' : n.toFixed(1));
</script>

{#if profiler.on}
	<div class="prof">
		<div><b>{profiler.fps} fps</b> · {r1(profiler.frameMs)} avg · <span class:bad={profiler.frameMax > 50}>{r1(profiler.frameMax)} max</span> ms</div>
		<div>GPU <b class:bad={profiler.gpuMs > 6}>{r1(profiler.gpuMs)}</b> ms · render-cpu {r1(profiler.renderMs)} ms</div>
		<div>{profiler.calls} draws · {(profiler.tris / 1000).toFixed(0)}k tris</div>
		<div class="hint">press P to hide</div>
	</div>
{/if}

<style>
	.prof {
		position: fixed;
		bottom: 8px;
		right: 8px;
		z-index: 9999;
		font: 11px/1.55 ui-monospace, 'SF Mono', Menlo, monospace;
		background: rgba(0, 0, 0, 0.62);
		color: #d6efe6;
		padding: 6px 10px;
		border-radius: 7px;
		pointer-events: none;
		white-space: nowrap;
	}
	.bad {
		color: #ff9b6b;
	}
	.hint {
		opacity: 0.45;
		font-size: 9px;
	}
</style>
