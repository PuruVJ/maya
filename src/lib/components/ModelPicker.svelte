<script lang="ts">
	// First-run model chooser (and re-opened from the model chip to switch). Local models only —
	// both run in-browser via WebGPU; no key, no server. Choice is remembered in localStorage.
	import { llm, MODELS, type ModelKey } from '$lib/llm.svelte';
	import { editor } from '$lib/editor.svelte';
	import { Button } from '$lib/components/ui/button';

	const open = $derived(!llm.selected || editor.modelPickerOpen);
	// MODELS is Partial (the tuned entry is conditional), but Object.entries only yields present keys
	type Entry = NonNullable<(typeof MODELS)[ModelKey]>;
	const entries = Object.entries(MODELS) as [ModelKey, Entry][];

	function pick(which: ModelKey) {
		editor.modelPickerOpen = false;
		llm.choose(which).catch(() => {});
	}
</script>

{#if open}
	<div class="fixed inset-0 z-30 flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm">
		<div class="w-[min(440px,100%)] rounded-2xl border border-white/10 bg-background/95 p-5 shadow-2xl">
			<div class="text-lg font-bold tracking-tight">Choose your local AI</div>
			<div class="mt-1 text-sm text-muted-foreground">
				Runs 100% in your browser — no key, no server, free. You can switch anytime.
			</div>

			<div class="mt-4 grid gap-2.5">
				{#each entries as [key, m] (key)}
					<button
						class="flex flex-col items-start rounded-xl border bg-white/[0.03] p-3.5 text-left transition hover:border-primary hover:bg-primary/10 {llm.selected === key ? 'border-primary' : 'border-white/10'}"
						onclick={() => pick(key)}
					>
						<div class="flex w-full items-center justify-between">
							<span class="text-base font-semibold">{m.label}</span>
							{#if llm.selected === key}
								<span class="text-xs font-medium text-primary">current</span>
							{/if}
						</div>
						<span class="mt-0.5 text-xs text-muted-foreground">{m.sub}</span>
					</button>
				{/each}
			</div>

			{#if llm.selected}
				<div class="mt-4 flex justify-end">
					<Button variant="secondary" size="sm" onclick={() => (editor.modelPickerOpen = false)}>Cancel</Button>
				</div>
			{:else}
				<div class="mt-3 text-center text-xs text-muted-foreground">Switching downloads the new model once, then it's cached.</div>
			{/if}
		</div>
	</div>
{/if}
