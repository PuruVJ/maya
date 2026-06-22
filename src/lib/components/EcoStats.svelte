<script lang="ts">
	// Ecosystem readout — live population by species, so the 4-pillar sim (nutrition / genetics / aging /
	// gestation) is legible and its BALANCE can be judged: do populations hold steady, boom, or crash? Polls the
	// agent registry at 1 Hz (cheap; never per-frame). Each species CHIP flashes when its count changes — GREEN on
	// a gain (a birth / an immigrant arriving), RED on a loss (a kill / starvation / old age) — so the churn of the
	// living world is glanceable without staring at the numbers.
	import { onMount } from 'svelte';
	import { agentManager } from '$lib/agents.svelte';
	import { rustBehaviorIsEmergent, setRustBehaviorMode, rustAgeMeans } from '$lib/rustSim';
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

	// ── detailed breakdown (the "complex HUD" — toggled open with 📊) ───────────────────────────────────────
	let detail = $state(false);
	let sexF = $state<Record<string, number>>({}); // live females per species (seedId even = female, matches Rust)
	let sexM = $state<Record<string, number>>({});
	let migrating = $state<Record<string, number>>({}); // per species: roamers en route to another settlement (Rust flag)
	let structures = $state<Record<string, number>>({}); // structure kind → count (near + dormant)
	let settlements = $state(0); // clumps of ≥3 buildings (a "decently sized" settlement)
	let liveByKind = $state<Record<string, number>>({}); // LIVE (individually simulated) per species — sex is only known for these
	let ageMeans = $state<Float32Array>(new Float32Array(6).fill(-1)); // mean age fraction (0..1) per kind, by Kind index
	const CREATURE_KINDS = new Set(['rabbit', 'cat', 'kangaroo', 'person', 'lion', 'dinosaur']);
	const SETTLE_KINDS = new Set(['house', 'cabin', 'tower', 'manor']);
	const STRUCT_ICON: Record<string, string> = { house: '🏠', cabin: '🛖', tower: '🗼', manor: '🏰', tree: '🌳', bush: '🌿', lamp: '🏮', grave: '🪦', fence: '🚧', rock: '🪨', pond: '💧' };

	onMount(() => {
		let prev: Record<string, number> = {};
		let first = true; // don't flash the whole row green on the very first sample (everything "appears")
		let vref = 1;
		let n = 0;
		const sample = () => {
			const c: Record<string, number> = {};
			let live = 0;
			let geneSum = 0;
			const f: Record<string, number> = {};
			const mle: Record<string, number> = {};
			const mig: Record<string, number> = {};
			agentManager.forEach((m) => {
				if (m.dead) return; // live only (corpses excluded)
				c[m.kind] = (c[m.kind] ?? 0) + 1;
				geneSum += m.gene ?? 1; // founders carry no JS gene → baseline 1.0 (matches the Rust founder gene)
				// sex: even seedId = female (matches Rust is_female); migrating = the Rust roamer flag (bit4)
				if ((m.seedId & 1) === 0) f[m.kind] = (f[m.kind] ?? 0) + 1;
				else mle[m.kind] = (mle[m.kind] ?? 0) + 1;
				if (m.migrating) mig[m.kind] = (mig[m.kind] ?? 0) + 1;
				live++;
			});
			sexF = f;
			sexM = mle;
			migrating = mig;
			liveByKind = { ...c }; // snapshot LIVE counts (with sex) before the dormant aggregates are folded in below
			ageMeans = rustAgeMeans(); // mean age fraction per kind (live) → the age readout below
			// structures (near live + dormant statics) by kind, and settlement count (clumps of ≥3 buildings)
			const st: Record<string, number> = {};
			const homes: [number, number][] = [];
			const tally = (o: { kind: string; pos: number[] }) => {
				if (CREATURE_KINDS.has(o.kind)) return;
				st[o.kind] = (st[o.kind] ?? 0) + 1;
				if (SETTLE_KINDS.has(o.kind)) homes.push([o.pos[0], o.pos[2]]);
			};
			for (const o of world.objects) tally(o);
			if (world.regions) for (const key in world.regions) for (const o of world.regions[key].statics) tally(o);
			structures = st;
			// greedy proximity clustering of homes → a "settlement" is a clump of ≥3 buildings within ~60 m
			const clusters: { sx: number; sz: number; n: number }[] = [];
			for (const [hx, hz] of homes) {
				let hit = clusters.find((cl) => (cl.sx / cl.n - hx) ** 2 + (cl.sz / cl.n - hz) ** 2 < 60 * 60);
				if (hit) {
					hit.sx += hx;
					hit.sz += hz;
					hit.n++;
				} else clusters.push({ sx: hx, sz: hz, n: 1 });
			}
			settlements = clusters.filter((cl) => cl.n >= 3).length;
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
		<button
			type="button"
			onclick={() => (detail = !detail)}
			class="pointer-events-auto ml-1 rounded-full px-1.5 text-[12px] transition-colors"
			class:text-amber-300={detail}
			title="Toggle the detailed world readout (sex split, settlements, structures, migration)"
		>
			📊
		</button>
	</div>

	{#if detail}
		<div
			class="pointer-events-none fixed left-1/2 top-[5.5rem] z-20 w-[21rem] -translate-x-1/2 space-y-2 rounded-xl bg-black/55 px-3 py-2 text-[12px] text-white/85 backdrop-blur"
		>
			<!-- per-species TOTAL ♂/♀ across the whole world. Live ones are sexed exactly; dormant (streamed-away)
			     ones are a headcount, so they're split ~50/50 (sex is seed-parity, ≈even) → an accurate total. ✈ is
			     the live roamers migrating right now. -->
			<div class="text-[10px] uppercase tracking-wide text-white/45">total ♂/♀ (incl. dormant) · ✈ migrating · 🎂 mean age (live)</div>
			<div class="space-y-0.5">
				{#each SPECIES as { k, icon }, idx (k)}
					{#if counts[k]}
						{@const dorm = Math.max(0, (counts[k] ?? 0) - (liveByKind[k] ?? 0))}
						{@const tM = (sexM[k] ?? 0) + Math.ceil(dorm / 2)}
						{@const tF = (sexF[k] ?? 0) + Math.floor(dorm / 2)}
						{@const age = ageMeans[idx] ?? -1}
						<div class="flex items-center justify-between tabular-nums">
							<span class="w-12">{icon} {counts[k]}</span>
							<span class="text-sky-300/80" title="males (live + ~half of {dorm} dormant)">♂{tM}</span>
							<span class="text-pink-300/80" title="females (live + ~half of {dorm} dormant)">♀{tF}</span>
							<span class="text-amber-300/90" title="live roamers migrating to another settlement now">✈{migrating[k] ?? 0}</span>
							<span class="text-lime-300/80" title="mean age as % of lifespan, live (0 = all newborns, 100 = all elderly)">🎂{age >= 0 ? Math.round(age * 100) + '%' : '–'}</span>
						</div>
					{/if}
				{/each}
			</div>
			<div class="border-t border-white/15 pt-1">
				<div class="text-white/70">🏘️ settlements: <span class="tabular-nums text-white">{settlements}</span> <span class="text-white/45">(≥3 buildings)</span></div>
				<div class="mt-1 flex flex-wrap gap-x-2 gap-y-0.5 tabular-nums">
					{#each Object.entries(structures).sort((a, b) => b[1] - a[1]) as [kind, count] (kind)}
						<span title={kind}>{STRUCT_ICON[kind] ?? '▫'} {count}</span>
					{/each}
				</div>
			</div>
		</div>
	{/if}
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
