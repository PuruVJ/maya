<script lang="ts">
	import { onMount } from 'svelte';
	import { replaceState } from '$app/navigation';
	import { Canvas } from '@threlte/core';
	import { PCFShadowMap } from 'three';
	import { World } from '@threlte/rapier';
	import Scene from '$lib/components/Scene.svelte';
	import Player from '$lib/components/Player.svelte';
	import EditController from '$lib/components/EditController.svelte';
	import AdaptiveResolution from '$lib/components/AdaptiveResolution.svelte';
	import { perf } from '$lib/perf.svelte';
	import BuildBar from '$lib/components/BuildBar.svelte';
	import FpsPanel from '$lib/components/FpsPanel.svelte';
	import EcoStats from '$lib/components/EcoStats.svelte';
	import { nature } from '$lib/nature.svelte';
	import ModelPicker from '$lib/components/ModelPicker.svelte';
	import TouchControls from '$lib/components/TouchControls.svelte';
	import { demoWorld, emptyWorld, capCreatures, type World as WorldData } from '$lib/world';
	import { encodeWorld, decodeWorld } from '$lib/share';
	import { loadWorld, saveWorld } from '$lib/worldStore';
	import { SKY_BG } from '$lib/kinds';
	import { enableWorldCurvature } from '$lib/curveWorld';
	import { llm } from '$lib/llm.svelte';
	import { agentManager } from '$lib/agents.svelte';
	import { editor } from '$lib/editor.svelte';
	import { playerState } from '$lib/playerState.svelte';

	// Inception-fold valley — ground rears up ahead/behind; smaller radius = tighter walls (before <Canvas>).
	// 800 (was 450) gentles the fold so the far side/city stays low enough to read as land, not lift into the sky.
	enableWorldCurvature(800);

	// if the URL carries a shared world (#w=…), start blank and fill it in onMount; else the demo
	const fromLink = typeof location !== 'undefined' && /[#&]w=/.test(location.hash);
	let world = $state(fromLink ? emptyWorld('Shared world') : demoWorld());
	let shareMsg = $state('');
	let liveUrl = $state(false); // gate the live-URL updater until the initial (maybe shared) world has settled

	// Render pixel-ratio. Two drivers, in priority: (1) while a build is GENERATING, the model saturates the
	// shared GPU → pin to a low 0.6 for those ~2s (soft but smooth), then release. (2) otherwise, DYNAMIC
	// RESOLUTION SCALING (perf.dpr, driven by AdaptiveResolution inside the Canvas) holds the fps target by
	// trading resolution under load — the "120fps no matter what" knob. Threlte applies dpr reactively.
	const dpr = $derived(llm.busy ? 0.6 : perf.dpr);

	// Drop world-objects that share an id — a past baby-id collision corrupted some saves, and duplicate keys
	// crash Svelte's keyed {#each}. Keeps the first occurrence of each id; cleans the save on next persist.
	const dedupeObjects = <T extends { objects: { id: string }[] }>(w: T): T => {
		const seen = new Set<string>();
		w.objects = w.objects.filter((o) => {
			if (seen.has(o.id)) return false;
			seen.add(o.id);
			return true;
		});
		return w;
	};

	onMount(async () => {
		const m = location.hash.match(/[#&]w=([^&]+)/);
		if (m) {
			// opened a SHARED link → load that world, persist it (store + local cache), then SCRUB the hash from
			// the address bar so it's not stuck there forever.
			try {
				world = capCreatures(dedupeObjects(await decodeWorld(m[1])));
				replaceState(location.pathname + location.search, {});
				saveWorld($state.snapshot(world));
			} catch {
				shareMsg = "That share link couldn't be read.";
				setTimeout(() => (shareMsg = ''), 3000);
			}
		} else {
			// normal open → restore from the world store (shared backend → local IndexedDB cache → else the demo)
			const saved = await loadWorld();
			if (saved && Array.isArray(saved.objects)) world = capCreatures(dedupeObjects(saved));
		}
		liveUrl = true; // from here on, edits persist to the world store (see effect below)
	});

	// Live shareable URL: the world IS the link. Encode the world MERGED with the live agent snapshot
	// (positions + dead/asleep) into the #w= hash. TWO triggers, so we never gzip pointlessly: (a) on any
	// EDIT (debounced reactive effect — captures builds/moves/paint for ANY world), and (b) ~1 Hz ONLY while
	// animals are present (to capture their wandering/deaths). A static built city therefore re-encodes only
	// when you change it, not every second. replaceState (no history spam); skipped when the hash is the same.
	// the player's live pose → packed into a SHARE link (the Share button) so a shared world reopens where you stood
	const playerPose = () => ({ x: playerState.pos[0], z: playerState.pos[2], yaw: playerState.yaw });
	// A save snapshot that captures the LIVE moment, not the placed-at spots: each creature's world-object pos is
	// rewritten to where it ACTUALLY is now (so a reloaded world resumes mid-wander, not reset to spawn), dead ones
	// are dropped (don't resurrect a corpse), and the player's pose rides along in `start` so you reopen standing
	// where you left. Operates on a detached $state.snapshot copy — the live world is untouched.
	function liveWorldSnapshot(): WorldData {
		const snap = $state.snapshot(world) as WorldData;
		const live = agentManager.liveSnapshot(); // objId → live {x, z, dead, asleep}
		snap.objects = snap.objects.filter((o) => {
			const ls = live.get(o.id);
			if (!ls) return true; // a static prop / house / tree → keep as placed
			if (ls.dead) return false; // a creature that died → don't persist it back to life
			o.pos = [ls.x, o.pos[1], ls.z]; // a live creature → save where it wandered to
			return true;
		});
		snap.start = playerPose(); // resume the player where they stood (Player restores world.start on load)
		return snap;
	}
	// EDITS → debounced save to the world store (local IndexedDB cache + best-effort sync to the shared backend).
	// JSON.stringify gives deep dep-tracking; the world only mutates on edits (animal movement lives in the agent
	// manager, not world.objects), so this fires on builds/moves/paint, NOT frames — and never touches the URL.
	let editTimer: ReturnType<typeof setTimeout> | undefined;
	$effect(() => {
		JSON.stringify(world);
		if (!liveUrl) return;
		clearTimeout(editTimer);
		editTimer = setTimeout(() => saveWorld(liveWorldSnapshot()), 500);
		return () => clearTimeout(editTimer);
	});

	// LIVE-WORLD PERSISTENCE — agents wander in the sim (not in world.objects), so the on-edit save above never
	// captures their movement; a reload used to reset every creature to its placed spot and the player to spawn.
	// So: (a) a low-frequency periodic save (~15 s — NOT the old 1 Hz URL-gzip that stalled the frame; this is an
	// async DB write of a detached snapshot) snapshots where everyone has wandered, and (b) a save the moment the
	// tab is hidden / unloaded captures the freshest pose right before you leave. Result: the world resumes mid-life.
	$effect(() => {
		if (!liveUrl) return;
		const persist = () => saveWorld(liveWorldSnapshot());
		const id = setInterval(persist, 15000);
		const onHide = () => {
			if (document.visibilityState === 'hidden') persist();
		};
		document.addEventListener('visibilitychange', onHide);
		window.addEventListener('pagehide', persist);
		return () => {
			clearInterval(id);
			document.removeEventListener('visibilitychange', onHide);
			window.removeEventListener('pagehide', persist);
		};
	});
	// HISTORY: live positions once re-encoded into the #w= URL every 1 Hz (gzip the WHOLE world + replaceState) —
	// a per-second main-thread STALL, so it was cut (2026-06-21) and positions stopped persisting. The effect above
	// brings them back the RIGHT way (big-world.md): an async DB snapshot at 15 s / on-hide, no URL, no per-frame
	// cost. See docs/sim-decisions.md C2.

	function reset() {
		if (!confirm('Reset to the demo world? This clears everything you’ve built here.')) return;
		world = demoWorld();
		shareMsg = 'Reset to the demo world';
		setTimeout(() => (shareMsg = ''), 2200);
	}

	async function share() {
		// Build the share link on demand and copy it — WITHOUT writing it to the address bar (the whole point of
		// moving state to the store was a clean URL; a share is an explicit, momentary action).
		try {
			const url = location.origin + location.pathname + '#w=' + (await encodeWorld(world, agentManager.liveSnapshot(), playerPose()));
			await navigator.clipboard.writeText(url);
			shareMsg = `Link copied — ${url.length} chars`;
		} catch {
			shareMsg = 'Could not create a share link';
		}
		setTimeout(() => (shareMsg = ''), 2800);
	}

	// HOME COMPASS — in an endless world it's easy to wander off and lose your build. A subtle indicator
	// points back to YOUR TOWN (the centroid of the buildings you placed — that's what you actually want to
	// find, not the arbitrary spawn point), falling back to spawn before you've built anything. Distance shown,
	// only once you've strayed (> 60 m). The target only recomputes when the world's buildings change.
	const BUILDING_K = new Set(['house', 'cabin', 'tower']);
	const home = $derived.by(() => {
		const b = world.objects.filter((o) => BUILDING_K.has(o.kind));
		if (!b.length) return (world.spawn ?? [0, 0, 0]) as [number, number, number];
		let sx = 0;
		let sz = 0;
		for (const o of b) ((sx += o.pos[0]), (sz += o.pos[2]));
		return [sx / b.length, 0, sz / b.length] as [number, number, number];
	});
	const homeDist = $derived(Math.hypot(playerState.pos[0] - home[0], playerState.pos[2] - home[2]));
	// arrow rotation (screen-space, 0 = up = your forward): bearing-to-home minus the player's facing.
	// forward is (sin yaw, −cos yaw) in (x,z), so the bearing of a point is atan2(dx, −dz). CW = +.
	const homeDeg = $derived.by(() => {
		let rel = Math.atan2(home[0] - playerState.pos[0], -(home[2] - playerState.pos[2])) - playerState.yaw;
		while (rel > Math.PI) rel -= 2 * Math.PI;
		while (rel < -Math.PI) rel += 2 * Math.PI;
		return (rel * 180) / Math.PI;
	});
</script>

<div class="fixed inset-0" style:background={SKY_BG[world.sky] ?? SKY_BG.day}>
	<Canvas shadows={PCFShadowMap} {dpr}>
		<World>
			<AdaptiveResolution />
			<Scene {world} />
			<Player {world} />
			<EditController {world} />
		</World>
	</Canvas>
</div>

<!-- danger vignette — red edges swell as a predator closes in to hunt you (manager → playerState.danger) -->
<div
	class="pointer-events-none fixed inset-0 z-20 transition-opacity duration-200"
	style:opacity={Math.min(1, playerState.danger * 1.15)}
	style:box-shadow="inset 0 0 16vw 3vw rgba(150,0,0,0.6)"
></div>
<!-- ...and a HEAVIER, darker closing-in when the hunter is BEHIND you (out of view) → the dread of the unseen
     pursuer: turn around to face it (and meet its red-glaring eyes). Fades as you turn toward it. -->
<div
	class="pointer-events-none fixed inset-0 z-20 transition-opacity duration-300"
	style:opacity={Math.min(1, playerState.danger * playerState.dangerBehind * 1.3)}
	style:box-shadow="inset 0 0 26vw 7vw rgba(85,0,0,0.85)"
></div>

<!-- live FPS / frame-time meter, top-centre -->
<FpsPanel />
<EcoStats />

<!-- MOTHER NATURE wildcard announcement — a dramatic banner when she sends in a pack/herd/boom -->
{#if nature.banner}
	<div
		class="pointer-events-none fixed left-1/2 top-24 z-20 max-w-[90vw] -translate-x-1/2 rounded-lg border border-white/10 bg-black/55 px-5 py-2.5 text-center text-sm font-semibold text-amber-100/95 shadow-xl backdrop-blur"
		style="animation: natureIn 0.5s ease-out;"
	>
		{nature.banner}
	</div>
{/if}

<!-- home compass — only once you've wandered off; the arrow points back to spawn, with the distance -->
{#if homeDist > 60}
	<div
		class="pointer-events-none fixed left-1/2 top-20 z-10 flex -translate-x-1/2 items-center gap-1.5 rounded-full bg-black/35 px-3 py-1 text-xs font-semibold text-white/85 backdrop-blur"
	>
		<span class="inline-block text-sm leading-none" style:transform="rotate({homeDeg}deg)">↑</span>
		<span>home · {homeDist < 1000 ? Math.round(homeDist) + ' m' : (homeDist / 1000).toFixed(1) + ' km'}</span>
	</div>
{/if}

<div class="pointer-events-none fixed left-4 top-4 text-white [text-shadow:0_1px_4px_rgba(0,0,0,0.5)]">
	<div class="text-xl font-bold tracking-tight">{world.name}</div>
	<div class="mt-1.5 flex items-center gap-1.5">
		<span class="inline-block rounded-full bg-black/35 px-2.5 py-1 text-xs font-semibold backdrop-blur [text-shadow:none]">
			100% local · no API key · free
		</span>
		<button
			class="pointer-events-auto rounded-full bg-black/35 px-2.5 py-1 text-xs font-semibold backdrop-blur transition hover:bg-black/55 [text-shadow:none]"
			onclick={() => (editor.modelPickerOpen = true)}
			title="Switch local model"
		>
			AI: {llm.model?.label ?? '…'} ⌄
		</button>
	</div>
</div>

<!-- Share: the world becomes a link (kept live in the address bar); Reset returns to the demo -->
<div class="fixed right-4 top-4 z-10 flex flex-col items-end gap-1.5">
	<div class="flex items-center gap-1.5">
		<button
			class="rounded-full bg-black/40 px-3 py-1.5 text-sm font-semibold text-white backdrop-blur transition hover:bg-black/60"
			onclick={reset}
			title="Reset to the demo world"
		>
			↺ Reset
		</button>
		<button
			class="rounded-full bg-black/40 px-4 py-1.5 text-sm font-semibold text-white backdrop-blur transition hover:bg-black/60"
			onclick={share}
		>
			🔗 Share
		</button>
	</div>
	<!-- object-count readout (the world now persists to the store, not the URL — no size-wall to warn about) -->
	{#if liveUrl && world.objects.length}
		<div
			class="pointer-events-none rounded-full bg-black/35 px-2.5 py-0.5 text-[11px] font-medium text-white/55 backdrop-blur"
			title="Your world auto-saves locally (and to the shared backend when deployed)."
		>
			{world.objects.length} objects
		</div>
	{/if}
	{#if shareMsg}
		<div class="rounded-full bg-emerald-600/90 px-3 py-1 text-xs font-medium text-white shadow-lg">{shareMsg}</div>
	{/if}
</div>

<div
	class="pointer-events-none fixed bottom-4 left-4 text-[13px] text-white/90 [text-shadow:0_1px_4px_rgba(0,0,0,0.6)] [@media(pointer:coarse)]:hidden"
>
	WASD move · Shift sprint · Space jump · drag to look
</div>

<BuildBar {world} />
<ModelPicker />
<TouchControls />

<style>
	@keyframes natureIn {
		0% {
			opacity: 0;
			transform: translate(-50%, -8px) scale(0.96);
		}
		100% {
			opacity: 1;
			transform: translate(-50%, 0) scale(1);
		}
	}
</style>
