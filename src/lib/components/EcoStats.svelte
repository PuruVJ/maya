<script lang="ts">
	// Ecosystem readout — live population by species, so the 4-pillar sim (nutrition / genetics / aging /
	// gestation) is legible and its BALANCE can be judged: do populations hold steady, boom, or crash? Polls the
	// agent registry at 1 Hz (cheap; never per-frame) and shows a trend arrow vs ~6 s ago so drift is visible.
	import { onMount } from 'svelte';
	import { agentManager } from '$lib/agents.svelte';

	// display order + glyph per species (matches the sim's Kind order)
	const SPECIES = [
		{ k: 'rabbit', icon: '🐇' },
		{ k: 'cat', icon: '🐈' },
		{ k: 'kangaroo', icon: '🦘' },
		{ k: 'person', icon: '🧍' },
		{ k: 'lion', icon: '🦁' },
		{ k: 'dinosaur', icon: '🦖' }
	] as const;

	let counts = $state<Record<string, number>>({});
	let trend = $state<Record<string, number>>({}); // -1 / 0 / +1 vs the reference snapshot
	let total = $state(0);

	onMount(() => {
		let ref: Record<string, number> = {}; // counts ~6 s ago, for the trend arrow
		let n = 0;
		const sample = () => {
			const c: Record<string, number> = {};
			let live = 0;
			agentManager.forEach((m) => {
				if (m.dead) return; // live only (corpses excluded)
				c[m.kind] = (c[m.kind] ?? 0) + 1;
				live++;
			});
			counts = c;
			total = live;
			if (n % 6 === 0) {
				// every ~6 s: recompute the trend vs the previous reference, then re-anchor
				const t: Record<string, number> = {};
				for (const { k } of SPECIES) t[k] = Math.sign((c[k] ?? 0) - (ref[k] ?? 0));
				trend = t;
				ref = c;
			}
			n++;
		};
		sample();
		const id = setInterval(sample, 1000);
		return () => clearInterval(id);
	});
</script>

{#if total > 0}
	<div
		class="pointer-events-none fixed left-1/2 top-3 z-10 flex -translate-x-1/2 items-center gap-2 rounded-full bg-black/40 px-3 py-1 text-[13px] font-medium text-white/85 backdrop-blur"
		title="Live population by species — watch it boom/steady/crash to judge the ecosystem balance"
	>
		{#each SPECIES as { k, icon } (k)}
			{#if counts[k]}
				<span class="tabular-nums">{icon} {counts[k]}{trend[k] > 0 ? '↑' : trend[k] < 0 ? '↓' : ''}</span>
			{/if}
		{/each}
	</div>
{/if}
