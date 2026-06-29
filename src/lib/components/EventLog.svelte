<script lang="ts">
	// The SETTLEMENT CHRONICLE — a small feed of the human/structure milestones (a person born or died, a house
	// raised, a well dug, a town founded), bottom-left. Newest at the bottom; older lines fade. The wildlife churn is
	// filtered out upstream (eventLog.svelte.ts) so this reads as a settlement journal, not a rabbit ticker. Pure
	// readout (pointer-events-none → never eats clicks). Styled to match the EcoStats HUD (rounded-2xl, zinc-900/70,
	// white/10 border, backdrop-blur-xl) so it's consistent with the rest of the UI. Hidden until something happens.
	import { fly } from 'svelte/transition';
	import { eventLog } from '$lib/eventLog.svelte';

	const SHOWN = 7; // how many recent lines to show
	const recent = $derived(eventLog.entries.slice(-SHOWN));
</script>

{#if recent.length}
	<div
		class="pointer-events-none fixed bottom-20 left-4 z-10 flex w-60 flex-col gap-1 rounded-2xl border border-white/10 bg-zinc-900/70 px-3 py-2 shadow-2xl shadow-black/40 backdrop-blur-xl [@media(pointer:coarse)]:hidden"
	>
		<div class="text-[10px] uppercase tracking-wide text-white/45">chronicle</div>
		{#each recent as e, i (e.id)}
			<div
				class="flex items-center gap-1.5 text-[12px] text-white/85"
				style:opacity={0.4 + 0.6 * ((i + 1) / recent.length)}
				transition:fly={{ y: 6, duration: 180 }}
			>
				<span class="text-sm leading-none">{e.icon}</span>
				<span class="truncate tabular-nums">{e.text}{e.n > 1 ? ` ×${e.n}` : ''}</span>
			</div>
		{/each}
	</div>
{/if}
