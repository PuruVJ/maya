<script lang="ts">
	import { onMount } from 'svelte';
	import { replaceState } from '$app/navigation';
	import { Canvas } from '@threlte/core';
	import { PCFShadowMap, type WebGLRenderer } from 'three';
	import { World } from '@threlte/rapier';
	import Scene from '$lib/components/Scene.svelte';
	import Player from '$lib/components/Player.svelte';
	import EditController from '$lib/components/EditController.svelte';
	import AdaptiveResolution from '$lib/components/AdaptiveResolution.svelte';
	import { perf } from '$lib/perf.svelte';
	import BuildBar from '$lib/components/BuildBar.svelte';
	import FpsPanel from '$lib/components/FpsPanel.svelte';
	import ModelPicker from '$lib/components/ModelPicker.svelte';
	import TouchControls from '$lib/components/TouchControls.svelte';
	import { demoWorld, emptyWorld } from '$lib/world';
	import { encodeWorld, decodeWorld } from '$lib/share';
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

	// WebGPU MIGRATION (flag-gated, incremental — see the perf-foundation-plan memory + the migration TODO).
	// Default is WebGL (everything as today, perfect). Opt in with ?webgpu (or #webgpu) to drive Threlte's Canvas
	// with three's WebGPURenderer instead — the path we're porting the shaders (GLSL onBeforeCompile → TSL) onto.
	// Until a shader is ported it simply renders with its base material on this path (plain, not broken). We flip
	// the default only once WebGPU reaches visual parity.
	const useWebGPU = typeof location !== 'undefined' && /(?:[?#&])webgpu(?:\b|=)/.test(location.search + location.hash);
	// the renderer factory Threlte's <Canvas createRenderer> calls; undefined → Threlte builds its WebGLRenderer.
	let createRenderer = $state<((canvas: HTMLCanvasElement) => WebGLRenderer) | undefined>(undefined);

	onMount(async () => {
		if (useWebGPU) {
			try {
				const { WebGPURenderer } = await import('three/webgpu');
				createRenderer = (canvas: HTMLCanvasElement) => {
					const r = new WebGPURenderer({ canvas, antialias: true, alpha: true, powerPreference: 'high-performance' });
					// Threlte's render task is SYNC, but WebGPURenderer needs async backend init → no-op render until
					// it's ready (else .render() throws "called before the backend is initialized").
					const realRender = r.render.bind(r) as (...a: unknown[]) => unknown;
					let ready = false;
					(r as unknown as { render: (...a: unknown[]) => unknown }).render = (...a: unknown[]) => (ready ? realRender(...a) : undefined);
					r.init()
						.then(() => ((ready = true), console.info('[webgpu] backend ready')))
						.catch((e) => console.error('[webgpu] init failed', e));
					return r as unknown as WebGLRenderer;
				};
			} catch (e) {
				console.error('[webgpu] could not load three/webgpu — staying on WebGL', e);
			}
		}
		const m = location.hash.match(/[#&]w=([^&]+)/);
		if (m) {
			try {
				world = await decodeWorld(m[1]);
			} catch {
				shareMsg = "That share link couldn't be read.";
				setTimeout(() => (shareMsg = ''), 3000);
			}
		}
		liveUrl = true; // from here on, the address bar mirrors the world live (see effect below)
	});

	// Live shareable URL: the world IS the link. Encode the world MERGED with the live agent snapshot
	// (positions + dead/asleep) into the #w= hash. TWO triggers, so we never gzip pointlessly: (a) on any
	// EDIT (debounced reactive effect — captures builds/moves/paint for ANY world), and (b) ~1 Hz ONLY while
	// animals are present (to capture their wandering/deaths). A static built city therefore re-encodes only
	// when you change it, not every second. replaceState (no history spam); skipped when the hash is the same.
	let linkBytes = $state(0); // size of the current share hash → shown so the URL limit isn't a surprise
	let lastHash = '';
	// the player's live pose → packed into the link so reopening/reloading drops you back where you stood
	const playerPose = () => ({ x: playerState.pos[0], z: playerState.pos[2], yaw: playerState.yaw });
	const pushUrl = async () => {
		try {
			const hash = '#w=' + (await encodeWorld(world, agentManager.liveSnapshot(), playerPose()));
			linkBytes = hash.length;
			if (hash !== lastHash) ((lastHash = hash), replaceState(hash, {}));
		} catch {
			/* encode unsupported (old browser) → Share button still works */
		}
	};
	// (a) edits → debounced. JSON.stringify gives deep dep-tracking; the world only mutates on edits (animal
	// movement lives in the agent manager, not world.objects), so this fires on builds/moves/paint, not frames.
	let editTimer: ReturnType<typeof setTimeout> | undefined;
	$effect(() => {
		JSON.stringify(world);
		if (!liveUrl) return;
		clearTimeout(editTimer);
		editTimer = setTimeout(pushUrl, 500);
		return () => clearTimeout(editTimer);
	});
	// (b) REMOVED for perf (2026-06-21) — the 1 Hz live-snapshot re-encode (gzip the WHOLE world + every agent
	// position + replaceState) was a periodic main-thread STALL every second whenever animals were present
	// (i.e. effectively always), dropping the frame rate even while standing still. Live agent POSITIONS no
	// longer persist across reload (they respawn at their placed spots — a minor nicety); BUILDS still persist
	// via the on-edit effect above, and Share captures the live moment on demand. The durable, ever-living
	// world belongs in the DB (docs/big-world.md), not a per-second URL gzip. See docs/sim-decisions.md C2.

	function reset() {
		if (!confirm('Reset to the demo world? This clears everything you’ve built here.')) return;
		world = demoWorld();
		shareMsg = 'Reset to the demo world';
		setTimeout(() => (shareMsg = ''), 2200);
	}

	async function share() {
		try {
			const hash = '#w=' + (await encodeWorld(world, agentManager.liveSnapshot(), playerPose())); // capture the live moment
			replaceState(hash, {}); // SvelteKit-router-friendly (raw history.replaceState warns)
			const url = location.origin + location.pathname + hash;
			await navigator.clipboard.writeText(url);
			shareMsg = `Link copied — ${url.length} chars`;
		} catch {
			shareMsg = 'Saved to the address bar';
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
	<!-- WebGL renders immediately; the WebGPU path waits one tick for the async three/webgpu import (createRenderer) -->
	{#if !useWebGPU || createRenderer}
	<Canvas shadows={PCFShadowMap} {dpr} {createRenderer}>
		<World>
			<AdaptiveResolution />
			<Scene {world} />
			<Player {world} />
			<EditController {world} />
		</World>
	</Canvas>
	{/if}
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

<!-- home compass — only once you've wandered off; the arrow points back to spawn, with the distance -->
{#if homeDist > 60}
	<div
		class="pointer-events-none fixed left-1/2 top-12 z-10 flex -translate-x-1/2 items-center gap-1.5 rounded-full bg-black/35 px-3 py-1 text-xs font-semibold text-white/85 backdrop-blur"
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
	<!-- link-size readout: the world IS the link, so show how big it's getting (turns amber/red near the
	     practical URL limit) — no surprises about hitting a size wall -->
	{#if liveUrl && world.objects.length}
		<div
			class="pointer-events-none rounded-full bg-black/35 px-2.5 py-0.5 text-[11px] font-medium backdrop-blur {linkBytes > 80000 ? 'text-red-300' : linkBytes > 40000 ? 'text-amber-300' : 'text-white/55'}"
			title="Your whole world is encoded in the share link. Browsers handle hundreds of KB; QR codes / easy pasting want it smaller."
		>
			{world.objects.length} objects · {(linkBytes / 1024).toFixed(1)} KB link
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
