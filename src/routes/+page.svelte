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
	import EventLog from '$lib/components/EventLog.svelte';
	import SplashScreen from '$lib/components/SplashScreen.svelte';
	import { fade } from 'svelte/transition';
	import { quality } from '$lib/quality.svelte';
	import { nature } from '$lib/nature.svelte';
	import TouchControls from '$lib/components/TouchControls.svelte';
	import { demoWorld, emptyWorld, fastForward, WORLD_NAME, LEGACY_WORLD_NAMES, type World as WorldData } from '$lib/world';
	import { math } from '$lib/math';
	import { heightAt } from '$lib/terrain';
	import { encodeWorld, decodeWorld } from '$lib/share';
	import { clock } from '$lib/clock';

	// TIME-LAPSE: scale the sim clock so users can watch the world evolve faster. The sim stays frame-rate
	// independent (fixed 30 Hz ticks) — rate just maps more ticks onto each real second (and the creature render
	// animation reads clock.rate so bodies stride faster too, not skate). 1× is normal. Persisted in localStorage
	// so the chosen pace survives a reload.
	const SPEEDS = [1, 1.5, 2] as const;
	const SPEED_KEY = 'sim-speed';
	let speed = $state(1);
	function setSpeed(r: number): void {
		speed = r;
		clock.setRate(r);
		try {
			localStorage.setItem(SPEED_KEY, String(r));
		} catch {
			/* private mode / storage disabled → just don't persist */
		}
	}
	import { loadWorld, saveWorld } from '$lib/worldStore';
	import { SKY_BG } from '$lib/kinds';
	import { enableWorldCurvature } from '$lib/curveWorld';
	import { settlementPlan, SIZES } from '$lib/settlementPlanner';
	import { llm } from '$lib/llm.svelte';
	import { agentManager } from '$lib/agents.svelte';
	import { playerState } from '$lib/playerState.svelte';
	import { sim } from '$lib/sim';

	// Inception-fold valley — ground rears up ahead/behind; smaller radius = tighter walls (before <Canvas>).
	// 800 (was 450) gentles the fold so the far side/city stays low enough to read as land, not lift into the sky.
	enableWorldCurvature(800);

	// if the URL carries a shared world (#w=…), start blank and fill it in onMount; else the demo
	const fromLink = typeof location !== 'undefined' && /[#&]w=/.test(location.hash);
	let world = $state(fromLink ? emptyWorld('Shared world') : demoWorld());
	// total DORMANT (streaming-offloaded) creatures across region aggregates — alive, just not simulated near you.
	const dormantCount = $derived(
		world.regions ? Object.values(world.regions).reduce((s, r) => s + Object.values(r.counts).reduce((a, b) => a + b, 0), 0) : 0
	);
	let shareMsg = $state('');
	let liveUrl = $state(false); // gate the live-URL updater until the initial (maybe shared) world has settled
	let resetting = false; // when true, all persistence is suppressed so a reset isn't re-saved on the way out

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
		quality.start(); // detect the render tier (weak device → low) + apply the resolution cap BEFORE first paint
		// restore the saved time-lapse speed (localStorage) → the chosen pace survives reloads
		const savedSpeed = Number(localStorage.getItem(SPEED_KEY));
		if (SPEEDS.includes(savedSpeed as (typeof SPEEDS)[number])) setSpeed(savedSpeed);
		// Load the main-thread Rust math BEFORE fastForward, so the away-growth uses the real Rust numbers, not the
		// permissive fallback. Same .wasm the worker uses — browser-cached.
		await math.init();
		const m = location.hash.match(/[#&]w=([^&]+)/);
		if (m) {
			// opened a SHARED link → load that world, persist it (store + local cache), then SCRUB the hash from
			// the address bar so it's not stuck there forever.
			try {
				world = dedupeObjects(await decodeWorld(m[1]));
				replaceState(location.pathname + location.search, {});
				saveWorld($state.snapshot(world));
			} catch {
				shareMsg = "That share link couldn't be read.";
				setTimeout(() => (shareMsg = ''), 3000);
			}
		} else {
			// normal open → restore from the world store (shared backend → local IndexedDB cache → else the demo)
			const saved = await loadWorld();
			if (saved && Array.isArray(saved.objects)) {
				const w = dedupeObjects(saved);
				// HEAL (temporary): caches written under an old title flicker name→Maya on reload. The local world's
				// name is app branding (no rename UI), so adopt the current one. Re-saved below, so it self-heals once.
				// TODO: remove once confirmed clean.
				if (LEGACY_WORLD_NAMES.includes(w.name)) w.name = WORLD_NAME;
				// DETERMINISTIC AGGREGATE FAST-FORWARD (big-world.md §3): advance the population to "now" by however
				// long you were away — closed-form, so even a week away is instant (no tick-replay hang).
				const away = saved.savedAt ? Date.now() - saved.savedAt : 0;
				const ff = away > 60_000 ? fastForward(w, away, 'ff' + Math.random().toString(36).slice(2, 7) + '-', (x, z) => heightAt(x, z, w.terrain)) : { creatures: 0, houses: 0 };
				world = w;
				if (away > 60_000) {
					const m = Math.round(away / 60_000);
					const txt = m < 90 ? `${m} min` : m < 2160 ? `${Math.round(m / 60)} h` : `${Math.round(m / 1440)} d`;
					const parts = [];
					if (ff.houses > 0) parts.push(`${ff.houses} new homes`);
					if (ff.creatures > 0) parts.push(`${ff.creatures} more creatures`);
					else if (ff.creatures < 0) parts.push(`${-ff.creatures} fewer creatures`);
					const change = parts.length ? ` · ${parts.join(', ')}` : '';
					nature.announce(`🌍 Welcome back — ${txt} passed while you were away${change}`);
				}
			}
		}
		// GROUND every placed (non-creature) object onto the terrain. The demo seed + legacy saves store y=0, which
		// BURIES houses/fences wherever the ambient relief rises (it's flat <70 m from spawn, then rolls into hills past
		// ~240 m — so a settlement seeded 350 m out sank into a hillside, "no fence, houses buried"). heightAt is pure JS
		// (no wasm) and the Prop renderer draws at pos[1], so re-deriving the ground height here fixes it for every load.
		const CREATURE_KINDS = new Set(['rabbit', 'cat', 'kangaroo', 'person', 'lion', 'dinosaur']);
		for (const o of world.objects) {
			if (!CREATURE_KINDS.has(o.kind)) o.pos[1] = heightAt(o.pos[0], o.pos[2], world.terrain);
		}
		liveUrl = true; // from here on, edits persist to the world store (see effect below)

		// DEV TELEPORT — type `goto(x, z)` in the browser console to jump anywhere (the player re-grounds on
		// arrival). Handy for visiting a far-off build without the long walk.
		if (typeof window !== 'undefined') {
			(window as unknown as { goto: (x: number, z: number) => void }).goto = (x, z) => {
				playerState.teleportTo = [x, 0, z];
			};
			// DEV: `demoSettlements()` in the console drops a spaced GALLERY of planned towns (all sizes), logs each
			// site's coordinates, and you teleport to them with goto(x,z). For previewing the settlement planner.
			(window as unknown as { demoSettlements: () => void }).demoSettlements = () => {
				const GAP = 240;
				const COLS = 4;
				if (!world.paths) world.paths = [];
				const sites: { site: number; size: string; goto: string }[] = [];
				for (let k = 0; k < 12; k++) {
					const size = SIZES[k % SIZES.length];
					const cx = 160 + (k % COLS) * GAP;
					const cz = -GAP + Math.floor(k / COLS) * GAP;
					const plan = settlementPlan(cx, cz, size, k * 1000 + 7, `demo${k}_`);
					world.objects.push(...plan.objects);
					world.paths!.push(...plan.paths);
					sites.push({ site: k + 1, size, goto: `goto(${cx}, ${cz})` });
				}
				console.table(sites);
				console.log('%c🏘️ Settlement gallery placed — teleport to a site with its goto(x, z) above.', 'font-weight:bold;font-size:13px');
			};
			// DEBUG: `await dumpState()` in the console → one JSON blob of all live + locally-stored state (player,
			// world, nearby natural ponds, sim, clock, climate, local/sessionStorage, the IndexedDB world cache).
			// Copied to the clipboard so it can be pasted verbatim. The single source of truth for diagnosing a bug.
			(window as unknown as { dumpState: () => Promise<unknown> }).dumpState = async () => {
				const r1 = (n: number) => Math.round(n * 10) / 10;
				const byKind: Record<string, number> = {};
				for (const o of world.objects) byKind[o.kind] = (byKind[o.kind] ?? 0) + 1;
				const [px, pz] = [playerState.pos[0], playerState.pos[2]];
				const flat = math.pondsNear(px, pz, 250);
				const pondsNearPlayer: { x: number; z: number; r: number }[] = [];
				if (flat) for (let k = 0; k < flat.length; k += 3) pondsNearPlayer.push({ x: r1(flat[k]), z: r1(flat[k + 1]), r: r1(flat[k + 2]) });
				let live = 0;
				// nearby agents (≤45 m) — what you're actually watching; captures a jittering/dying couple's state
				const nearbyAgents: unknown[] = [];
				agentManager.forEach((m) => {
					live++;
					const d2 = (m.agent.x - px) ** 2 + (m.agent.z - pz) ** 2;
					if (d2 <= 45 * 45)
						nearbyAgents.push({
							kind: m.kind,
							pos: [r1(m.agent.x), r1(m.agent.z)],
							dist: r1(Math.sqrt(d2)),
							behavior: m.agent.behavior,
							health: r1(m.health),
							ageFrac: m.ageFrac != null ? r1(m.ageFrac) : undefined,
							flags: [m.dead && 'dead', m.asleep && 'asleep', m.pregnant && 'pregnant', m.guardian && 'guardian', m.migrating && 'migrating', m.drinking && 'drinking'].filter(Boolean)
						});
				});
				nearbyAgents.sort((a, b) => (a as { dist: number }).dist - (b as { dist: number }).dist);
				const dumpStore = (s: Storage) => Object.fromEntries(Array.from({ length: s.length }, (_, i) => [s.key(i)!, s.getItem(s.key(i)!)!]));
				let cache: unknown = null;
				try {
					const w = await loadWorld();
					cache = w ? { name: w.name, objects: w.objects.length, zones: w.zones?.length ?? 0, start: w.start } : null;
				} catch (e) {
					cache = `error: ${e}`;
				}
				const state = {
					player: { pos: playerState.pos.map(r1), yaw: r1(playerState.yaw), state: playerState.state, grounded: playerState.grounded },
					world: { name: world.name, ground: world.ground, sky: world.sky, objects: world.objects.length, byKind, zones: world.zones, paths: world.paths?.length ?? 0, terrain: world.terrain?.length ?? 0, start: world.start, regions: Object.keys(world.regions ?? {}).length },
					pondsNearPlayer,
					nearbyAgents,
					sim: { status: sim.status(), danger: r1(sim.danger()), liveAgents: live },
					clock: { tick: clock.tick, rate: clock.rate, paused: clock.paused },
					nature: { aridity: r1(nature.aridity) },
					localStorage: dumpStore(localStorage),
					sessionStorage: dumpStore(sessionStorage),
					worldCache: cache
				};
				const json = JSON.stringify(state, null, 2);
				console.log('%c=== dumpState ===', 'font-weight:bold;font-size:13px', state);
				console.log(json);
				try {
					await navigator.clipboard.writeText(json);
					console.log('%c✓ copied to clipboard — paste it to share', 'color:#4ade80');
				} catch {
					/* clipboard blocked → copy from the JSON log above */
				}
				return state;
			};
		}
	});

	// Live shareable URL: the world IS the link. Encode the world MERGED with the live agent snapshot
	// (positions + dead/asleep) into the #w= hash. TWO triggers, so we never gzip pointlessly: (a) on any
	// EDIT (debounced reactive effect — captures builds/moves/paint for ANY world), and (b) ~1 Hz ONLY while
	// animals are present (to capture their wandering/deaths). A static built city therefore re-encodes only
	// when you change it, not every second. replaceState (no history spam); skipped when the hash is the same.
	// the player's live pose → packed into a SHARE link (the Share button) so a shared world reopens where you stood
	const playerPose = () => ({ x: playerState.pos[0], y: playerState.pos[1], z: playerState.pos[2], yaw: playerState.yaw });
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
		snap.savedAt = Date.now(); // stamp the save → on next load we know how long you were away (fast-forward seam)
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
		editTimer = setTimeout(() => !resetting && saveWorld(liveWorldSnapshot()), 500);
		return () => clearTimeout(editTimer);
	});

	// LIVE-WORLD PERSISTENCE — agents wander in the sim (not in world.objects), so the on-edit save above never
	// captures their movement; a reload used to reset every creature to its placed spot and the player to spawn.
	// So: (a) a low-frequency periodic save (~15 s — NOT the old 1 Hz URL-gzip that stalled the frame; this is an
	// async DB write of a detached snapshot) snapshots where everyone has wandered, and (b) a save the moment the
	// tab is hidden / unloaded captures the freshest pose right before you leave. Result: the world resumes mid-life.
	let hiddenAt = 0; // wall-clock when the tab went hidden → on return we fast-forward the world by the away span
	let timeTravel = $state(false); // true while the tab-return catch-up plays behind the "time-travelling" splash
	$effect(() => {
		if (!liveUrl) return;
		const persist = () => !resetting && saveWorld(liveWorldSnapshot());
		const id = setInterval(persist, 1000); // sync to the DB every second (user request) — captures wandering + builds promptly
		// TAB-AWAY pauses the sim (visibility gate — no ticks accrue). So on tab-RETURN we FAST-FORWARD the world by
		// however long you were gone (closed-form, the same path a page reload uses) — otherwise it sits frozen at the
		// moment you left and the catch-up never happens (user: "tabbed away, tabbed back, 27, no jump → ff is broken").
		// The live sim then resumes from the advanced state.
		const onVis = () => {
			if (document.visibilityState === 'hidden') {
				persist();
				hiddenAt = Date.now();
				return;
			}
			if (!hiddenAt || resetting) return;
			const away = Date.now() - hiddenAt;
			hiddenAt = 0;
			if (away <= 60_000) return; // a quick tab-flick → nothing worth catching up
			timeTravel = true; // cover the catch-up behind the splash so the population JUMP never flashes on screen
			const ff = fastForward(world, away, 'tf' + Math.random().toString(36).slice(2, 7) + '-', (x, z) => heightAt(x, z, world.terrain));
			setTimeout(() => (timeTravel = false), 1000); // minimum 1 s block, then reveal the advanced world
			if (ff.creatures || ff.houses) {
				const mins = Math.round(away / 60_000);
				const txt = mins < 90 ? `${mins} min` : `${Math.round(mins / 60)} h`;
				const parts: string[] = [];
				if (ff.houses > 0) parts.push(`${ff.houses} new homes`);
				if (ff.creatures > 0) parts.push(`${ff.creatures} more creatures`);
				else if (ff.creatures < 0) parts.push(`${-ff.creatures} fewer creatures`);
				nature.announce(`🌍 Welcome back — ${txt} away${parts.length ? ' · ' + parts.join(', ') : ''}`);
			}
		};
		document.addEventListener('visibilitychange', onVis);
		window.addEventListener('pagehide', persist);
		return () => {
			clearInterval(id);
			document.removeEventListener('visibilitychange', onVis);
			window.removeEventListener('pagehide', persist);
		};
	});
	// HISTORY: live positions once re-encoded into the #w= URL every 1 Hz (gzip the WHOLE world + replaceState) —
	// a per-second main-thread STALL, so it was cut (2026-06-21) and positions stopped persisting. The effect above
	// brings them back the RIGHT way (big-world.md): an async DB snapshot at 15 s / on-hide, no URL, no per-frame
	// cost. See docs/sim-decisions.md C2.

	// A TRUE reset. Just swapping `world = demoWorld()` isn't enough: the autosave + unload-save re-persist it, and
	// the Rust sim worker keeps its own (growing) population independent of world.objects. So: suppress all saving,
	// wipe the local cache (IndexedDB), then RELOAD so the sim worker restarts fresh. (100% local — no server wipe.)
	async function reset() {
		if (!confirm('Reset to the demo world? This clears everything you’ve built here.')) return;
		resetting = true; // stop the 500ms edit-save, the 1s autosave, and the pagehide/visibilitychange save
		shareMsg = 'Resetting…';
		await new Promise<void>((resolve) => {
			const open = indexedDB.open('worldgen', 1);
			open.onsuccess = () => {
				const db = open.result;
				if (!db.objectStoreNames.contains('worlds')) {
					db.close();
					return resolve();
				}
				const tx = db.transaction('worlds', 'readwrite');
				tx.objectStore('worlds').delete('current');
				tx.oncomplete = () => {
					db.close();
					resolve();
				};
				tx.onerror = () => {
					db.close();
					resolve();
				};
			};
			open.onerror = () => resolve();
		});
		location.reload(); // fresh load: empty cache → demo world, and a brand-new sim worker
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
<SplashScreen name={world.name} />

<!-- TIME-TRAVEL splash — covers the tab-return fast-forward so the population JUMP never flashes on screen.
     Shows for a minimum of 1 s (set in the visibilitychange handler), then fades to reveal the advanced world. -->
{#if timeTravel}
	<div
		transition:fade={{ duration: 350 }}
		class="fixed inset-0 z-[60] flex flex-col items-center justify-center gap-5 bg-gradient-to-b from-[#0a0f1e] to-[#070a14] text-white"
	>
		<div class="bg-gradient-to-b from-white to-amber-100/70 bg-clip-text text-2xl font-semibold tracking-tight text-transparent">
			{world.name}
		</div>
		<div class="flex items-center gap-2.5 text-sm text-white/65">
			<span class="h-4 w-4 animate-spin rounded-full border-2 border-white/20 border-t-white/80"></span>
			Time-travelling to now…
		</div>
	</div>
{/if}
<EcoStats {world} />
<EventLog />

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
		<!-- player world coordinates (the arrow still points home) -->
		<span class="tabular-nums">{Math.round(playerState.pos[0])}, {Math.round(playerState.pos[2])}</span>
	</div>
{/if}

<div class="pointer-events-none fixed left-4 top-4 flex flex-col gap-2.5 text-white">
	<div>
		<div class="bg-gradient-to-b from-white to-amber-100/70 bg-clip-text text-2xl font-semibold tracking-tight text-transparent [filter:drop-shadow(0_1px_6px_rgba(0,0,0,0.55))]">
			{world.name}
		</div>
		<div class="mt-0.5 text-[11px] font-medium tracking-wide text-white/45 [text-shadow:0_1px_3px_rgba(0,0,0,0.6)]">
			100% local · no API key · free
		</div>
	</div>
	<!-- TIME-LAPSE speed: watch the simulation evolve faster (the sim stays frame-rate-independent) -->
	<div
		class="pointer-events-auto inline-flex w-fit items-center gap-0.5 rounded-full border border-white/10 bg-zinc-900/55 p-0.5 backdrop-blur-xl"
		title="Simulation speed"
	>
		{#each SPEEDS as s (s)}
			<button
				class="rounded-full px-2.5 py-1 text-xs font-semibold transition {speed === s ? 'bg-amber-500 text-black shadow-sm' : 'text-white/55 hover:text-white'}"
				onclick={() => setSpeed(s)}
			>
				{s}×
			</button>
		{/each}
	</div>
	<!-- QUALITY tier — Lite drops the decorative layers + caps resolution (smoother on weak devices/mobile). Auto-set
	     on weak hardware; this lets you force either way. -->
	<button
		class="pointer-events-auto inline-flex w-fit items-center gap-1.5 rounded-full border border-white/10 bg-zinc-900/55 px-2.5 py-1 text-xs font-semibold backdrop-blur-xl transition {quality.low ? 'text-amber-300' : 'text-white/55 hover:text-white'}"
		onclick={() => quality.toggle()}
		title={quality.low ? 'Lite mode — fewer effects + capped resolution (smoother on weak devices). Tap for full quality.' : 'Full quality. Tap for Lite mode — smoother on weak devices / mobile.'}
	>
		{quality.low ? '📱 Lite' : '✨ HD'}
	</button>
</div>

<!-- Share: the world becomes a link (kept live in the address bar); Reset returns to the demo -->
<div class="fixed right-4 top-4 z-10 flex flex-col items-end gap-1.5">
	<div class="flex items-center gap-1.5">
		<button
			class="rounded-full border border-white/10 bg-zinc-900/55 px-3.5 py-1.5 text-sm font-semibold text-white/90 backdrop-blur-xl transition hover:bg-zinc-800/70 hover:text-white"
			onclick={reset}
			title="Reset to the demo world"
		>
			↺ Reset
		</button>
		<button
			class="rounded-full border border-white/10 bg-zinc-900/55 px-4 py-1.5 text-sm font-semibold text-white/90 backdrop-blur-xl transition hover:bg-zinc-800/70 hover:text-white"
			onclick={share}
		>
			🔗 Share
		</button>
	</div>
	<!-- object-count readout. LIVE = world.objects (static + near creatures, what's simulated/rendered — streaming
	     keeps this bounded). DORMANT = streaming-offloaded creatures, still alive in region aggregates. -->
	{#if liveUrl && world.objects.length}
		<div
			class="pointer-events-none rounded-full border border-white/5 bg-zinc-900/50 px-2.5 py-0.5 text-[11px] font-medium tabular-nums text-white/50 backdrop-blur-xl"
			title="LIVE = simulated/rendered near you (streaming bounds this for perf). DORMANT = offloaded far creatures, still alive in region aggregates — they wake as you approach."
		>
			{world.objects.length} live{#if dormantCount > 0}&nbsp;· {dormantCount} dormant{/if}
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
