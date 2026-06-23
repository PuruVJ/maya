<script lang="ts">
	// A small chat-like feed of recent world events (births, kills, builds…), bottom-left. Shows the last few
	// lines newest-at-bottom; older lines fade. Pure readout (eventLog is fed by sim.ts) — pointer-events-none so
	// it never eats clicks. Hidden until something happens.
	import { eventLog } from '$lib/eventLog.svelte';

	const SHOWN = 7; // how many recent lines to show
	const recent = $derived(eventLog.entries.slice(-SHOWN));
</script>

{#if recent.length}
	<div class="pointer-events-none fixed bottom-20 left-4 z-10 flex w-64 flex-col gap-0.5 text-[12px] [@media(pointer:coarse)]:hidden">
		{#each recent as e, i (e.id)}
			<div
				class="flex items-center gap-1.5 rounded-md bg-black/35 px-2 py-0.5 text-white/90 backdrop-blur [text-shadow:0_1px_3px_rgba(0,0,0,0.6)]"
				style:opacity={0.35 + 0.65 * ((i + 1) / recent.length)}
			>
				<span class="text-sm leading-none">{e.icon}</span>
				<span class="truncate">{e.text}{e.n > 1 ? ` ×${e.n}` : ''}</span>
			</div>
		{/each}
	</div>
{/if}
