<script lang="ts">
	// Ecosystem readout — live population by species, so the 4-pillar sim (nutrition / genetics / aging /
	// gestation) is legible and its BALANCE can be judged: do populations hold steady, boom, or crash? Polls the
	// agent registry at 1 Hz (cheap; never per-frame). Each species CHIP flashes when its count changes — GREEN on
	// a gain (a birth / an immigrant arriving), RED on a loss (a kill / starvation / old age) — so the churn of the
	// living world is glanceable without staring at the numbers.
	import { onMount } from 'svelte';
	import { agentManager } from '$lib/agents.svelte';
	import { rustBehaviorIsEmergent, setRustBehaviorMode } from '$lib/rustSim';
	import type { World } from '$lib/world';

	// the decision BRAIN driving the agents (docs/emergent-behavior.md): Emergent (needs+primitives+utility, the
	// default) vs Manual (the hand-coded sim). A dev A/B toggle — flip it live in the same world to compare.
	let emergent = $state(rustBehaviorIsEmergent());
	function toggleBrain() {
		emergent = setRustBehaviorMode(!emergent);
	}

	// the world — so the readout includes DORMANT region aggregates (streaming-offloaded creatures are still alive,
	// just not individually simulated). Counting near + dormant gives the TRUE total, so streaming never reads as a crash.
	let { world }: { world: World } = $props();

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
	let pulse = $state<Record<string, number>>({}); // bumps on every change → re-keys the chip so the flash replays
	let dir = $state<Record<string, number>>({}); // +1 = gained (green), -1 = lost (red) — picks which flash plays
	let total = $state(0);
	// average VIGOR (the inherited speed gene) across the live population. It's static per agent for life, so we
	// can read it straight off the ManagedAgent (founders = 1.0) — no snapshot plumbing. Watch it drift upward as
	// selection favours faster lineages: that's EVOLUTION, made visible.
	let vigor = $state(1);
	let vigorTrend = $state(0);

	onMount(() => {
		let prev: Record<string, number> = {};
		let first = true; // don't flash the whole row green on the very first sample (everything "appears")
		let vref = 1;
		let n = 0;
		const sample = () => {
			const c: Record<string, number> = {};
			let live = 0;
			let geneSum = 0;
			agentManager.forEach((m) => {
				if (m.dead) return; // live only (corpses excluded)
				c[m.kind] = (c[m.kind] ?? 0) + 1;
				geneSum += m.gene ?? 1; // founders carry no JS gene → baseline 1.0 (matches the Rust founder gene)
				live++;
			});
			// + DORMANT region aggregates — streaming-offloaded creatures are alive too, just not individually
			// simulated, so the total stays consistent as you roam (no apparent "crash" when a region sleeps).
			if (world.regions) {
				for (const key in world.regions) {
					const agg = world.regions[key];
					let regN = 0;
					for (const k in agg.counts) {
						c[k] = (c[k] ?? 0) + agg.counts[k];
						regN += agg.counts[k];
					}
					live += regN;
					geneSum += agg.gene * regN;
				}
			}
			if (!first) {
				for (const { k } of SPECIES) {
					const cur = c[k] ?? 0;
					const was = prev[k] ?? 0;
					if (cur !== was) {
						dir[k] = cur > was ? 1 : -1;
						pulse[k] = (pulse[k] ?? 0) + 1; // re-key → CSS flash animation restarts from the top
					}
				}
			}
			counts = c;
			total = live;
			vigor = live > 0 ? geneSum / live : 1;
			prev = c; // absent species → 0 on the next compare, so a drop-to-zero still registers (chip just vanishes)
			first = false;
			if (n % 6 === 0) {
				vigorTrend = Math.sign(vigor - vref - 0.002); // small deadband so noise doesn't flicker the arrow
				vref = vigor;
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
		class="pointer-events-none fixed left-1/2 top-12 z-20 flex -translate-x-1/2 items-center gap-1 rounded-full bg-black/40 px-3 py-1 text-[13px] font-medium text-white/85 backdrop-blur"
		title="Live population by species — chips flash green on a gain (birth / immigrant), red on a loss (kill / starve / old age)"
	>
		{#each SPECIES as { k, icon } (k)}
			{#if counts[k]}
				{#key pulse[k]}
					<span class="chip tabular-nums" class:up={dir[k] > 0} class:down={dir[k] < 0}>{icon} {counts[k]}</span>
				{/key}
			{/if}
		{/each}
		<span class="ml-1 tabular-nums text-emerald-300/90" title="Average inherited vigor (speed gene) — drifts up as evolution selects faster lineages">
			⚡{vigor.toFixed(2)}{vigorTrend > 0 ? '↑' : vigorTrend < 0 ? '↓' : ''}
		</span>
		<button
			type="button"
			onclick={toggleBrain}
			class="pointer-events-auto ml-1 rounded-full px-1.5 text-[12px] transition-colors"
			class:text-fuchsia-300={emergent}
			class:text-sky-300={!emergent}
			title={emergent ? 'Brain: EMERGENT (needs + utility) — click to switch to Manual' : 'Brain: MANUAL (hand-coded) — click to switch to Emergent'}
		>
			{emergent ? '🧠 emergent' : '⚙️ manual'}
		</button>
	</div>
{/if}

<style>
	.chip {
		border-radius: 6px;
		padding: 0 4px;
		background-color: rgba(0, 0, 0, 0); /* baseline → the flash animation tints then fades back to this */
	}
	.chip.up {
		animation: flash-up 1.1s ease-out;
	}
	.chip.down {
		animation: flash-down 1.1s ease-out;
	}
	@keyframes flash-up {
		0%,
		8% {
			background-color: rgba(16, 185, 129, 0.9);
		}
		100% {
			background-color: rgba(16, 185, 129, 0);
		}
	}
	@keyframes flash-down {
		0%,
		8% {
			background-color: rgba(239, 68, 68, 0.9);
		}
		100% {
			background-color: rgba(239, 68, 68, 0);
		}
	}
</style>
