<script lang="ts">
	// FIRST-OPEN intro — one line on what this world is, shown ONCE. On dismiss we flag it in localStorage so it never
	// shows again; the world renders (blurred) behind it the whole time, then it fades and play continues. A single
	// boolean belongs in localStorage (the world state itself lives in IndexedDB); bump KEY's version to re-show it.
	import { onMount } from 'svelte';
	import { fade, scale } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';

	let { name }: { name: string } = $props();

	const KEY = 'gw:intro:v1';
	let show = $state(false);

	onMount(() => {
		try {
			if (!localStorage.getItem(KEY)) show = true; // never seen → greet
		} catch {
			show = true; // storage blocked (private mode) → still greet them this once
		}
	});

	function begin() {
		show = false;
		try {
			localStorage.setItem(KEY, '1');
		} catch {
			/* storage blocked → it'll greet again next open; acceptable */
		}
	}
</script>

{#if show}
	<!-- z above the HUD (timeTravel splash is z-60); pointer-events-auto so it captures clicks until dismissed -->
	<div
		transition:fade={{ duration: 300 }}
		class="pointer-events-auto fixed inset-0 z-[70] flex items-center justify-center bg-black/55 px-6 backdrop-blur-md"
	>
		<div
			transition:scale={{ duration: 320, start: 0.94, easing: cubicOut }}
			class="flex max-w-sm flex-col items-center gap-4 rounded-2xl border border-white/10 bg-zinc-900/85 px-8 py-8 text-center shadow-2xl shadow-black/60 backdrop-blur-xl"
		>
			<div class="bg-gradient-to-b from-white to-amber-100/70 bg-clip-text text-3xl font-semibold tracking-tight text-transparent">
				{name}
			</div>
			<p class="text-[15px] leading-relaxed text-white/70">
				A tiny world that lives, breeds and grows on its own — wander it, and type anything to shape it.
			</p>
			<button
				type="button"
				onclick={begin}
				class="mt-1 rounded-full bg-amber-500 px-7 py-2 text-sm font-semibold text-black shadow-lg shadow-amber-500/20 transition hover:bg-amber-400"
			>
				Begin
			</button>
			<div class="text-[11px] tracking-wide text-white/35">WASD to move · Space / C to fly · type to build</div>
		</div>
	</div>
{/if}
