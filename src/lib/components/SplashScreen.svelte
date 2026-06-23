<script lang="ts">
	// Full-screen splash over the first seconds of a session — it covers the scene mount / shader-compile / resolution
	// settle (where the grey flicker lives) and fades out once `boot.ready` (min dwell + sim ready + frame rate
	// stabilised; see boot.svelte). NOT tied to the LLM — that loads lazily, long after this is gone.
	import { boot } from '$lib/boot.svelte';
	import { fade } from 'svelte/transition';

	let { name = 'Maya' }: { name?: string } = $props();
</script>

{#if !boot.ready}
	<div
		transition:fade={{ duration: 450 }}
		class="fixed inset-0 z-[60] flex flex-col items-center justify-center gap-5 bg-gradient-to-b from-[#0a0f1e] to-[#070a14] text-white"
	>
		<div class="text-center">
			<div class="text-2xl font-bold tracking-tight">{name}</div>
			<div class="mt-1 text-xs font-medium text-white/45">100% local · no API key · free</div>
		</div>
		<div class="flex items-center gap-2.5 text-sm text-white/60">
			<span class="h-4 w-4 animate-spin rounded-full border-2 border-white/20 border-t-white/80"></span>
			Loading world…
		</div>
	</div>
{/if}
