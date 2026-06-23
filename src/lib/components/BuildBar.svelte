<script lang="ts">
	import { onMount } from 'svelte';
	import { Tween } from 'svelte/motion';
	import { cubicOut } from 'svelte/easing';
	import { fly } from 'svelte/transition';
	import { llm } from '$lib/llm.svelte';
	import { playerState } from '$lib/playerState.svelte';
	import { applyOps } from '$lib/engine';
	import { cityOps, isCityCommand, forestOps, isForestCommand, lakeOps, isLakeCommand } from '$lib/city';
	import { history } from '$lib/history.svelte';
	import { editor } from '$lib/editor.svelte';
	import { KINDS } from '$lib/kinds';
	import { dlog } from '$lib/debug';
	import type { World } from '$lib/world';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';

	let { world }: { world: World } = $props();

	const KIND_LIST = Object.keys(KINDS);

	// shared style for the small, low-contrast utility buttons in the tool tray — kept visually subordinate so the
	// hero command input below is unmistakably the main thing (user: "I want the input to be the big thing").
	const trayBtn =
		'flex size-9 items-center justify-center rounded-full text-[15px] leading-none text-white/55 transition hover:bg-white/10 hover:text-white disabled:opacity-25 disabled:hover:bg-transparent';

	let text = $state('');
	let lastInstruction = $state('');

	// slash-command palette — typing "/" suggests the native (non-AI) commands. Anything that isn't a
	// command shows NO suggestions and falls straight through to the AI.
	const COMMANDS = [
		{ cmd: 'make city', label: '🏙 make city', desc: 'Build a city here · repeat to grow it' },
		{ cmd: 'make forest', label: '🌲 make forest', desc: 'Plant a forest · repeat to thicken it' },
		{ cmd: 'make lake', label: '💧 make lake', desc: 'Dig a lake · repeat to widen it' },
		{ cmd: 'undo', label: '↶ undo', desc: 'Undo the last change' },
		{ cmd: 'redo', label: '↷ redo', desc: 'Redo the last undone change' },
		{ cmd: 'clear', label: '🧹 clear', desc: 'Remove everything you built · undoable' },
		{ cmd: 'home', label: '🏠 home', desc: 'Teleport back to your town' }
	];
	const q = $derived(text.trim().replace(/^\//, '').toLowerCase());
	const slash = $derived(text.trimStart().startsWith('/'));
	const suggestions = $derived(slash ? COMMANDS.filter((c) => c.cmd.startsWith(q)) : []);
	let selIdx = $state(0);
	function runSuggestion(c: { cmd: string }) {
		text = c.cmd;
		selIdx = 0;
		build();
	}
	let canReroll = $state(false);
	let pending = $state<{ ids: string[]; label: string } | null>(null);
	let note = $state<string | null>(null); // boundary / "I can't do that" message from the model

	const LIMITS =
		"I couldn't build that. I can make houses, towers, cabins, trees, rocks, lamps, fences, wells, water, paths, hills, people and cats — try something simpler, or build it up a step at a time.";

	const player = () => ({ pos: playerState.pos, yaw: playerState.yaw });

	// the model downloads as a handful of big shards, so llm.progress JUMPS (~11% a shard). Ease a shown value toward
	// it so the bar CLIMBS smoothly between callbacks instead of lurching. (Resets instantly to 0 when a load starts.)
	const shownProgress = new Tween(0, { duration: 500, easing: cubicOut });
	$effect(() => {
		shownProgress.set(llm.progress, llm.progress === 0 ? { duration: 0 } : undefined);
	});
	const pct = $derived(Math.round(shownProgress.current * 100));
	const placeholder = $derived(
		llm.busy
			? 'Building your world…' // generating — make the wait legible (the model can take a few seconds)
			: llm.phase === 'ready'
			? 'Build one thing at a time…  (e.g. "a house in front of me")  ·  / for commands'
			: llm.phase === 'loading'
				? `Local AI… ${pct}%`
				: llm.phase === 'error'
					? 'AI failed to load — tap retry'
					: 'Local AI…'
	);

	// auto-load the model in the background as soon as the page opens (cached after first time)
	onMount(() => {
		llm.load().catch(() => {});
	});

	// The model emits whole compound/CRUD chains in one shot (verified: Qwen2.5-1.5B 7/7 on the
	// compound battery at temp 0.3), so one call per instruction — later ops reference earlier ones
	// within the same batch (the engine applies them in order).
	async function run(instruction: string, temperature = 0.3) {
		const p = player();
		const ops = await llm.generate(instruction, world, p, temperature);
		// the model talks back via note ops (limits / "I can't do that"); the engine ignores them
		note = ops.filter((o) => o.op === 'note').map((o) => (o as { text: string }).text).join(' ') || null;
		const actionable = ops.filter((o) => o.op !== 'note');
		// SIZE WORDS — "big house", "small cabin", "huge tower" → scale the placed objects. The model doesn't emit
		// scale, so we read the size from the raw instruction and multiply it in (deterministic, no retrain needed).
		const sizeM = /\b(huge|giant|massive|grand)\b/i.test(instruction) ? 2 : /\bbig|large|tall\b/i.test(instruction) ? 1.55 : /\b(tiny|mini)\b/i.test(instruction) ? 0.5 : /\bsmall|little|cosy|cozy\b/i.test(instruction) ? 0.65 : 0;
		if (sizeM > 0) {
			for (const o of actionable) {
				if (o.op === 'add') {
					const s = (o as { scale?: [number, number, number] }).scale ?? [1, 1, 1];
					(o as { scale?: [number, number, number] }).scale = [s[0] * sizeM, s[1] * sizeM, s[2] * sizeM];
				}
			}
		}
		if (actionable.length === 0) {
			// safety net: nothing buildable came back — tell the user the limits if the model didn't
			if (!note) note = LIMITS;
			dlog('build', 'no actionable ops', { instruction, note });
			return false; // nothing built (the boundary note is shown separately)
		}
		const out = { conflicts: [] as { label: string; blockers: string[] }[] };
		applyOps(world, actionable, p, out);
		const ids = [...new Set(out.conflicts.flatMap((c) => c.blockers))];
		pending = ids.length ? { ids, label: out.conflicts[0].label } : null;
		return true;
	}

	async function build() {
		let instruction = text.trim();
		if (instruction.startsWith('/')) instruction = instruction.slice(1).trim(); // "/make city" → "make city"
		if (!instruction || llm.busy) return;
		text = '';
		// non-tech users type these instead of using the buttons — handle directly, don't ask the AI
		const cmd = instruction.toLowerCase().replace(/[.!]+$/, '');
		if (cmd === 'undo' || cmd === 'undo that' || cmd === 'go back' || cmd === 'revert') {
			undo();
			return;
		}
		if (cmd === 'redo' || cmd === 'redo that' || cmd === 'again') {
			redo();
			return;
		}
		// whisk back to your town (centroid of buildings, like the home compass) — handy in the endless world
		if (['home', 'go home', 'take me home'].includes(cmd)) {
			goHome();
			return;
		}
		// wipe everything you've built (keep the ground/sky/terrain canvas) — destructive but UNDOABLE, so no
		// confirm needed; ↶ brings it all back. Bare forms only — "clear the trees" still goes to the AI.
		if (['clear', 'clear all', 'clear everything', 'reset', 'start over', 'remove everything', 'delete everything'].includes(cmd)) {
			if (world.objects.length || world.zones.length || world.paths.length) {
				history.push(world);
				world.objects = [];
				world.zones = [];
				world.paths = [];
			}
			lastInstruction = '';
			canReroll = false;
			note = 'Cleared everything — undo (↶) to bring it back.';
			return;
		}
		// native procedural generators (the small model can't lay these out) — instant, no AI call, and
		// repeating the command grows the same city/forest outward.
		if (isCityCommand(cmd)) {
			history.push(world);
			applyOps(world, cityOps(world, player()), player());
			lastInstruction = '';
			canReroll = false;
			note = 'Built a city — say “make city” again to grow it bigger.';
			return;
		}
		if (isForestCommand(cmd)) {
			history.push(world);
			applyOps(world, forestOps(world, player()), player());
			lastInstruction = '';
			canReroll = false;
			note = 'Planted a forest — say “make forest” again to grow it.';
			return;
		}
		if (isLakeCommand(cmd)) {
			history.push(world);
			applyOps(world, lakeOps(world, player()), player());
			lastInstruction = '';
			canReroll = false;
			note = 'Dug a lake — say “make lake” again to widen it.';
			return;
		}
		history.push(world);
		const ok = await run(instruction);
		if (ok) {
			lastInstruction = instruction;
			canReroll = true;
		} else {
			history.discard();
		}
	}

	// palette: drop a known kind in front of the player (shares undo with everything else)
	function place(kind: string) {
		history.push(world);
		applyOps(world, [{ op: 'add', kind, at: 'front', dist: 4 }], player());
		canReroll = false;
		note = null;
	}

	function undo() {
		history.undo(world);
		canReroll = false;
		pending = null;
		note = null;
	}

	function redo() {
		history.redo(world);
		canReroll = false;
		pending = null;
		note = null;
	}

	// teleport to your town — the centroid of placed buildings (matches the home compass), else the spawn.
	function goHome() {
		const B = new Set(['house', 'cabin', 'tower']);
		const b = world.objects.filter((o) => B.has(o.kind));
		let hx = world.spawn?.[0] ?? 0;
		let hz = world.spawn?.[2] ?? 0;
		if (b.length) {
			hx = 0;
			hz = 0;
			for (const o of b) ((hx += o.pos[0]), (hz += o.pos[2]));
			hx /= b.length;
			hz /= b.length;
		}
		playerState.teleportTo = [hx, 0, hz];
		canReroll = false;
		note = b.length ? 'Whisked you back to your town.' : 'Whisked you back to the start.';
	}

	async function reroll() {
		const snap = history.peek();
		if (!lastInstruction || llm.busy || !snap) return;
		history.restore(world, snap);
		await run(lastInstruction, 0.85);
	}

	function demolish() {
		if (!pending) return;
		history.push(world);
		applyOps(
			world,
			pending.ids.map((id) => ({ op: 'remove' as const, id })),
			player()
		);
		pending = null;
	}

	function onKey(e: KeyboardEvent) {
		if (suggestions.length) {
			if (e.key === 'ArrowDown') {
				e.preventDefault();
				selIdx = (selIdx + 1) % suggestions.length;
				return;
			}
			if (e.key === 'ArrowUp') {
				e.preventDefault();
				selIdx = (selIdx - 1 + suggestions.length) % suggestions.length;
				return;
			}
			if (e.key === 'Tab') {
				e.preventDefault();
				text = suggestions[Math.min(selIdx, suggestions.length - 1)].cmd; // autocomplete, don't run yet
				return;
			}
			if (e.key === 'Escape') {
				text = '';
				return;
			}
		}
		if (e.key === 'Enter') {
			e.preventDefault();
			if (suggestions.length) runSuggestion(suggestions[Math.min(selIdx, suggestions.length - 1)]);
			else build();
		}
	}
</script>

<div
	class="fixed bottom-6 left-1/2 z-10 flex w-[min(620px,calc(100vw-2rem))] -translate-x-1/2 flex-col items-center gap-2"
>
	{#if llm.phase === 'error'}
		<div transition:fly={{ y: 8, duration: 180 }} class="rounded-full border border-red-400/20 bg-red-950/80 px-3.5 py-1.5 text-xs text-red-200 shadow-lg shadow-black/30 backdrop-blur-xl">
			{llm.text} ·
			<button class="font-medium underline" onclick={() => llm.load().catch(() => {})}>retry</button>
		</div>
	{/if}

	{#if pending}
		<div transition:fly={{ y: 8, duration: 180 }} class="rounded-full border border-amber-400/20 bg-amber-950/80 px-3.5 py-1.5 text-xs text-amber-100 shadow-lg shadow-black/30 backdrop-blur-xl">
			The {pending.label} is over {pending.ids.length} object{pending.ids.length > 1 ? 's' : ''} —
			<button class="font-medium underline" onclick={demolish}>demolish</button>
			<span class="opacity-50">·</span>
			<button class="underline opacity-80" onclick={() => (pending = null)}>keep</button>
		</div>
	{/if}

	{#if note}
		<div transition:fly={{ y: 8, duration: 180 }} class="flex max-w-full items-start gap-2 rounded-2xl border border-sky-400/20 bg-sky-950/80 px-3.5 py-2 text-xs text-sky-100 shadow-lg shadow-black/30 backdrop-blur-xl">
			<span class="select-none">💡</span>
			<span class="flex-1">{note}</span>
			<button class="font-medium underline opacity-80" onclick={() => (note = null)}>ok</button>
		</div>
	{/if}

	{#if editor.tool === 'delete'}
		<div transition:fly={{ y: 8, duration: 180 }} class="rounded-full border border-rose-400/20 bg-rose-950/80 px-3.5 py-1.5 text-xs text-rose-100 shadow-lg shadow-black/30 backdrop-blur-xl">
			Delete mode — tap any object to remove it ·
			<button class="font-medium underline" onclick={() => (editor.tool = 'none')}>done</button>
		</div>
	{:else if editor.tool === 'move'}
		<div transition:fly={{ y: 8, duration: 180 }} class="rounded-full border border-sky-400/20 bg-sky-900/80 px-3.5 py-1.5 text-xs text-sky-100 shadow-lg shadow-black/30 backdrop-blur-xl">
			{editor.held ? '✋ carrying it — tap where it should go' : 'Move mode — tap a thing to pick it up'} ·
			<button class="font-medium underline" onclick={() => { editor.tool = 'none'; editor.held = null; }}>done</button>
		</div>
	{/if}

	<!-- BAR GROUP — stays put. The palette + command suggestions float ABOVE it as overlays (absolute, bottom-full)
	     so opening them never reflows the input or the tray (user: opening the palette shoved the build bar up). -->
	<div class="relative flex w-full flex-col items-center gap-2">
		{#if editor.paletteOpen}
			<div
				transition:fly={{ y: 8, duration: 160, easing: cubicOut }}
				class="absolute bottom-full left-0 mb-2 flex w-full flex-wrap justify-center gap-1.5 rounded-2xl border border-white/10 bg-zinc-900/85 p-2.5 shadow-2xl backdrop-blur-md"
			>
				{#each KIND_LIST as kind (kind)}
					<Button variant="secondary" size="sm" class="h-7 px-2.5 text-xs capitalize" onclick={() => place(kind)}>
						{kind}
					</Button>
				{/each}
			</div>
		{/if}

		{#if suggestions.length}
			<div transition:fly={{ y: 8, duration: 160, easing: cubicOut }} class="absolute bottom-full left-0 mb-2 w-full overflow-hidden rounded-2xl border border-white/10 bg-zinc-900/85 shadow-2xl backdrop-blur-md">
				{#each suggestions as s, i (s.cmd)}
					<button
						class="flex w-full items-center gap-2.5 px-3.5 py-2 text-left text-sm transition {i === Math.min(selIdx, suggestions.length - 1) ? 'bg-white/15' : 'hover:bg-white/10'}"
						onmouseenter={() => (selIdx = i)}
						onclick={() => runSuggestion(s)}
					>
						<span class="font-medium text-foreground">{s.label}</span>
						<span class="flex-1 truncate text-xs text-foreground/55">{s.desc}</span>
					</button>
				{/each}
			</div>
		{/if}

		<!-- TOOL TRAY — history + edit modes + palette, deliberately small & low-contrast so the hero input dominates -->
		<div class="flex items-center gap-0.5 rounded-full border border-white/10 bg-zinc-900/60 p-1 shadow-lg shadow-black/30 backdrop-blur-xl">
			<button class={trayBtn} onclick={undo} disabled={!history.canUndo || llm.busy} title="Undo">↶</button>
			<button class={trayBtn} onclick={redo} disabled={!history.canRedo || llm.busy} title="Redo">↷</button>
			<button class={trayBtn} onclick={reroll} disabled={!canReroll || llm.busy} title="Re-roll — try that prompt again">↻</button>
			<span class="mx-1 h-4 w-px bg-white/15"></span>
			<button
				class="{trayBtn} {editor.tool === 'move' ? 'bg-sky-500/80 text-white hover:bg-sky-500/80' : ''}"
				onclick={() => { editor.tool = editor.tool === 'move' ? 'none' : 'move'; editor.held = null; }}
				title="Move mode — tap a thing, then tap where it goes">✋</button>
			<button
				class="{trayBtn} {editor.tool === 'delete' ? 'bg-rose-500/85 text-white hover:bg-rose-500/85' : ''}"
				onclick={() => (editor.tool = editor.tool === 'delete' ? 'none' : 'delete')}
				title="Delete mode — tap objects to remove them">🗑</button>
			<span class="mx-1 h-4 w-px bg-white/15"></span>
			<button
				class="{trayBtn} {editor.paletteOpen ? 'bg-white/15 text-white' : ''}"
				onclick={() => (editor.paletteOpen = !editor.paletteOpen)}
				title="Palette — drop a thing in front of you">⊞</button>
		</div>

		<!-- HERO — the command input is the star: big, inviting, everything else is subordinate to it -->
		<div
			class="group relative flex w-full items-center gap-2.5 rounded-2xl border border-white/12 bg-zinc-900/65 py-2 pl-4 pr-2 shadow-[0_16px_50px_-12px_rgba(0,0,0,0.85)] backdrop-blur-2xl transition-colors focus-within:border-amber-400/40"
		>
			<span class="pointer-events-none select-none text-lg leading-none text-amber-300/70 transition-colors group-focus-within:text-amber-300">✦</span>
			<Input
				bind:value={text}
				onkeydown={onKey}
				{placeholder}
				maxlength={100}
				disabled={llm.phase !== 'ready'}
				class="h-12 flex-1 border-0 bg-transparent px-0 text-base text-foreground shadow-none placeholder:text-white/40 focus-visible:ring-0"
			/>
			<Button
				onclick={build}
				disabled={llm.phase !== 'ready' || llm.busy || !text.trim()}
				class="h-10 rounded-xl bg-amber-500 px-5 text-sm font-semibold text-black hover:bg-amber-400"
			>
				{#if llm.phase === 'loading'}
					{pct}%
				{:else if llm.busy}
					<span class="animate-pulse">building…</span>
				{:else}
					Build
				{/if}
			</Button>
		</div>
	</div>
</div>
