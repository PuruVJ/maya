<script lang="ts">
	import { T, useTask, useThrelte } from '@threlte/core';
	import Prop from './Prop.svelte';
	import Graves from './Graves.svelte';
	import Rocks from './Rocks.svelte';
	import Flowers from './Flowers.svelte';
	import Bushes from './Bushes.svelte';
	import Wells from './Wells.svelte';
	import Bridges from './Bridges.svelte';
	import Npc from './Npc.svelte';
	import Zone from './Zone.svelte';
	import Water from './Water.svelte';
	import LakeFish from './LakeFish.svelte';
	import Tree from './Tree.svelte';
	import Building from './Building.svelte';
	import Lamp from './Lamp.svelte';
	import Path from './Path.svelte';
	import Terrain from './Terrain.svelte';
	import AmbientScatter from './AmbientScatter.svelte';
	import SettlementGlows from './SettlementGlows.svelte';
	import LoveHearts from './LoveHearts.svelte';
	import Chimneys from './Chimneys.svelte';
	import DustPuffs from './DustPuffs.svelte';
	import SpawnPuffs from './SpawnPuffs.svelte';
	import FootPrints from './FootPrints.svelte';
	import SplashDrops from './SplashDrops.svelte';
	import AmbientParticles from './AmbientParticles.svelte';
	import FallingLeaves from './FallingLeaves.svelte';
	import Weather from './Weather.svelte';
	import Mist from './Mist.svelte';
	import Birds from './Birds.svelte';
	import Butterflies from './Butterflies.svelte';
	import Grass from './Grass.svelte';
	import AgentSystem from './AgentSystem.svelte';
	import AgentImpostors from './AgentImpostors.svelte';
	import CreatureShadows from './CreatureShadows.svelte';
	import PlacedShadows from './PlacedShadows.svelte';
	import LampGlow from './LampGlow.svelte';
	import BuildingGlow from './BuildingGlow.svelte';
	import MoveGhost from './MoveGhost.svelte';
	import Critter from './Critter.svelte';
	import InstancedCreatures from './InstancedCreatures.svelte';
	import Profiler from './Profiler.svelte';
	import SkyDome from './SkyDome.svelte';
	import { SKY_FOG, kindDef } from '$lib/kinds';
	import { forEachTreeNear, treeRadius, onPath } from '$lib/scatter';
	import { setEyeshine } from '$lib/sharedAssets';
	import { sim } from '$lib/sim';
	import { math } from '$lib/math';
	import { vitals } from '$lib/vitals.svelte';
	import { agentManager, CORPSE_DECAY_SECS } from '$lib/agents.svelte';
	import { worldAreaScale } from '$lib/world';
	import { streamRegions, regionOf, fastForwardDormant, enforceLiveBudget, drainWakes } from '$lib/streaming';
	import { quality } from '$lib/quality.svelte';
	import { playerState } from '$lib/playerState.svelte';
	import { heightAt } from '$lib/terrain';
	import { packStructures, packWaterZones, kindStr, OP_STRIDE, OP_ADD } from '$lib/structpack';
	import { waterSurfaceY, waterEdgeFactor } from '$lib/water';
	import { nature } from '$lib/nature.svelte';
	import { wind } from '$lib/wind';
	import { weather } from '$lib/weather';
	import * as THREE from 'three';
	import type { World, WorldObject } from '$lib/world';

	let { world }: { world: World } = $props();
	const fog = $derived(SKY_FOG[world.sky] ?? SKY_FOG.day);

	// the sun's shadow frustum follows the player (see the useTask). World units per shadow-map texel =
	// frustum span / mapSize → snap the frustum centre to whole texels so shadow edges don't crawl as you walk.
	let sun = $state<THREE.DirectionalLight>();
	const SHADOW_TEXEL = 120 / 2048;
	// place the light at its offset the moment it binds (the per-frame follow takes over next frame) so there's
	// never a frame with the light at the origin pointing nowhere
	$effect(() => {
		if (sun) sun.position.set(playerState.pos[0] + 30, 45, playerState.pos[2] + 20);
	});

	// SHADOW THROTTLE: the 2048² shadow map is re-rendered EVERY frame — re-rasterising all casters (mostly
	// STATIC trees/props/buildings) into depth — and that cost is fixed regardless of dpr, so as DRS lowers
	// resolution the shadow pass becomes a bigger share. Drive it on a throttle instead: update every Nth frame
	// (≥60 Hz at 120 fps) — imperceptible shadow lag, ~N× cheaper shadow pass, so DRS can hold the target at a
	// crisper colour resolution. autoUpdate off → we flag needsUpdate ourselves in the task.
	const { renderer } = useThrelte();
	const SHADOW_EVERY = 2; // re-render shadows every other frame
	let shadowTick = 0;
	$effect(() => {
		renderer.shadowMap.autoUpdate = false;
		renderer.shadowMap.needsUpdate = true; // one render so we don't start a frame with an empty shadow map
		return () => (renderer.shadowMap.autoUpdate = true); // restore if the scene unmounts
	});

	// Scene lighting follows the sky so NIGHT is actually dark (was constant → night looked like day with a
	// different backdrop, wasting the lit-window/lamp glow). Moonlit cool-blue at night, warm low sun at
	// sunset, flat grey under fog. Intensity + colour shift here; the sun DIRECTION stays fixed (its position
	// tracks the player each frame in the useTask so shadows render across the endless world — same angle).
	const LIGHT: Record<string, { amb: number; dir: number; ambC: string; dirC: string }> = {
		day: { amb: 0.65, dir: 1.15, ambC: '#ffffff', dirC: '#fff6e0' },
		sunset: { amb: 0.5, dir: 0.8, ambC: '#ffd6b0', dirC: '#ff8f4d' },
		fog: { amb: 0.6, dir: 0.3, ambC: '#aeb6c2', dirC: '#c2c9d4' }, // overcast/RAIN: dimmer + greyer than a clear day (was brighter than day!), weak diffuse sun
		night: { amb: 0.82, dir: 1.1, ambC: '#aebfe0', dirC: '#cfddf8' }, // user: "everything too dark" → brighter moonlight (was 0.58/0.85)
		space: { amb: 0.52, dir: 0.72, ambC: '#8a9ace', dirC: '#aab8e4' }
	};
	const light = $derived(LIGHT[world.sky] ?? LIGHT.day);

	// tell the food chain how nocturnal it is → prey jumpier, predators keener after dark
	const NIGHTNESS: Record<string, number> = { day: 0, sunset: 0.4, fog: 0.25, night: 1, space: 1 };
	$effect(() => {
		const n = NIGHTNESS[world.sky] ?? 0;
		agentManager.setNight(n);
		setEyeshine(n); // animal eyes glow after dark (eyeshine) — predators brighter, a threat in the gloom
	});

	// the living kinds steer themselves (flocking) — every OTHER object is a solid prop that ambient
	// animals must route around, so feed their footprints to the agent manager whenever the world changes
	const CREATURES = new Set(['person', 'cat', 'lion', 'rabbit', 'kangaroo', 'dinosaur']);
	type Obstacle = { x: number; z: number; r: number; hx?: number; hz?: number; cos?: number; sin?: number };
	// the STATIC solids (placed props/buildings + ponds) — recomputed only when the world changes
	let baseObstacles: Obstacle[] = [];
	const TREE_FEED_R = 140; // feed the ambient-forest trunks within this radius of the player as obstacles
	const TREE_REFEED2 = 24 * 24; // re-feed once the player has moved this far (the 140m radius gives margin)
	// NATURAL PONDS — Rust owns the world's water (an even, infinite, deterministic pond field). We just DRAW the
	// ones near the player, refreshed as they roam. The sim reads the same field internally for thirst, so animals
	// settle by their local pond instead of all dragging to one shore.
	// WATER LOD: only the ponds right next to you get the full <Water> shader; everything farther is a CHEAP flat
	// blue disc (one shared MeshBasic — no shader pass, no fish). Rendering full Water + a per-frame LakeFish shoal
	// for all ~16 nearby ponds was a real mount-storm (the grey flicker on load) for zero gain at distance/in fog.
	const POND_RENDER_R = 360; // draw natural ponds within this (near = Water, far = blob); beyond is fogged out
	const POND_NEAR_R2 = 130 * 130; // within this → full Water; beyond → the cheap flat blob
	// a flat unit disc laid on the ground, with an ORGANIC rim (one shared blob shape, scaled per pond) so far ponds
	// read as natural water — not perfect circles — at near-zero cost (vs a per-pond geometry). Matches the near
	// <Water> blob's silhouette so the LOD swap doesn't pop from "round" to "blob".
	const POND_BLOB_GEO = (() => {
		const SEG = 22;
		const seed = 5.0; // a fixed seed → all far blobs share this outline (unnoticeable at distance, but not a circle)
		const pos: number[] = [0, 0, 0];
		const idx: number[] = [];
		for (let i = 0; i <= SEG; i++) {
			const a = (i / SEG) * Math.PI * 2;
			const rr = waterEdgeFactor(seed, a);
			pos.push(Math.cos(a) * rr, 0, Math.sin(a) * rr);
		}
		for (let i = 1; i <= SEG; i++) idx.push(0, i + 1, i);
		const g = new THREE.BufferGeometry();
		g.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3));
		g.setIndex(idx);
		return g;
	})();
	const POND_BLOB_MAT = new THREE.MeshBasicMaterial({ color: '#3f6f9e' }); // flat water-blue; cheap, no lighting
	type PondZone = { id: string; material: string; shape: string; pos: [number, number, number]; size: number };
	let nearPonds = $state<PondZone[]>([]); // full Water shader (the few right next to you)
	let farPonds = $state<{ id: string; x: number; y: number; z: number; r: number }[]>([]); // cheap blue discs
	let lastPondX = NaN;
	let lastPondZ = NaN;
	function refreshPonds(px: number, pz: number): void {
		const flat = math.pondsNear(px, pz, POND_RENDER_R);
		if (!flat) return; // wasm not ready yet
		// reuse NEAR pond objects by id (stable identity) so an unchanged one keeps the SAME object → Water doesn't
		// rebuild on a refresh. FAR blobs are cheap, so they just get rebuilt each time.
		const byId = new Map(nearPonds.map((z) => [z.id, z]));
		const near: PondZone[] = [];
		const far: typeof farPonds = [];
		let nearChanged = false;
		for (let k = 0; k < flat.length; k += 3) {
			const x = flat[k]; // flat is [x, z, r] per pond
			const z = flat[k + 1];
			const r = flat[k + 2];
			const id = `np${Math.round(x)}_${Math.round(z)}`;
			if ((x - px) ** 2 + (z - pz) ** 2 < POND_NEAR_R2) {
				const existing = byId.get(id);
				if (existing) near.push(existing);
				else ((nearChanged = true), near.push({ id, material: 'water', shape: 'blob', pos: [x, 0, z], size: r }));
			} else {
				// far → a flat blob at the water surface height (matches Water so there's no jump at the LOD boundary)
				far.push({ id, x, y: waterSurfaceY({ id, material: 'water', shape: 'blob', pos: [x, 0, z], size: r }, world.terrain), z, r });
			}
		}
		if (nearChanged || near.length !== nearPonds.length) nearPonds = near; // only churn Water on a real change
		farPonds = far;
		lastPondX = px;
		lastPondZ = pz;
	}

	let lastTreeFeedX = NaN;
	let lastTreeFeedZ = NaN;

	// The Rust sim has no ambient forest of its own (it's a deterministic per-cell function shared with the
	// renderer + player — NOT bit-exact to port). So instead of porting the scatter, we feed the SAME trunks
	// the renderer draws (treeAt, path-culled) near the player as circle-obstacles → animals route around the
	// very trees you see, no divergence. Combined with the static props/ponds each feed.
	function feedObstacles(px: number, pz: number): void {
		const trees: Obstacle[] = [];
		const paths = world.paths;
		// the SAME Rust forest field the renderer draws (forEachTreeNear), path-culled → animals route around the
		// very trunks you see, no divergence. One wasm call. Combined with the static props/ponds each feed.
		forEachTreeNear(px, pz, TREE_FEED_R, (tr) => {
			if (onPath(paths, tr.x, tr.z)) return; // culled on roads → not drawn → don't collide
			trees.push({ x: tr.x, z: tr.z, r: treeRadius(tr.scale) });
		});
		// NATURAL PONDS are solids too — feed the same ones the renderer draws so agents are pushed out to the BANK
		// (where they drink) instead of wading through the water. Placed lake zones are already in baseObstacles.
		const pondObs: Obstacle[] = [];
		const np = math.pondsNear(px, pz, POND_RENDER_R);
		if (np) for (let k = 0; k < np.length; k += 3) pondObs.push({ x: np[k], z: np[k + 1], r: np[k + 2] });
		lastTreeFeedX = px;
		lastTreeFeedZ = pz;
		sim.setObstacles([...baseObstacles, ...trees, ...pondObs]); // the Rust sim resolves these solids (push-out, no tunnelling)
	}

	// DRINK SOURCES = natural ponds + any WELLS settlers have dug. Re-run whenever a well is placed (emergent jobs),
	// so the new water joins the thirst sim and life can congregate at it. A well is a small drink point (r≈3).
	function feedWaterSources(): void {
		const ponds = (world.zones ?? []).filter((z) => z.material === 'water').map((z) => ({ x: z.pos[0], z: z.pos[2], r: z.size * 1.05 }));
		const wells = world.objects.filter((o) => o.kind === 'well').map((o) => ({ x: o.pos[0], z: o.pos[2], r: 3 }));
		sim.setWater([...ponds, ...wells]);
	}

	// STATIC obstacle / refuge / water rebuild. The OLD code ran this in a `$effect` keyed on `world.objects`, so it
	// re-ran (a filter+map+box-math over EVERY object + a worker re-post — profiled ~40% of the main thread at 300
	// objects) on ANY world change: a birth, a death→grave, a wandered creature, a fence panel. A hunt (constant
	// creature churn) or a T-rex drop thus fired it every frame → the freeze + jank the user hit. Now it runs from a
	// THROTTLED poll (useTask) gated on a cheap STRUCTURE FINGERPRINT — so creature/fence churn never triggers it, only
	// a real structure add/remove/move does, at most a few times a second.
	function rebuildStaticObstacles(): void {
		const props: Obstacle[] = world.objects
			// FENCES are NOT animal collision — the settlement-avoidance (Rust) keeps wildlife OUT; a collidable wall
			// only TRAPPED a creature caught inside. Animals are pushed cleanly out; the PLAYER still collides (Player.svelte).
			.filter((o) => !CREATURES.has(o.kind) && o.kind !== 'fence')
			.map((o) => {
				const def = kindDef(o.kind);
				const sx = o.scale?.[0] ?? 1;
				const sz = o.scale?.[2] ?? 1;
				const wall = def.parts[0];
				// box-footprint kinds (houses/cabins) → ORIENTED BOX so animals hug walls / use streets; round kinds stay circles.
				if (wall && wall.geo === 'box') {
					const hx = (wall.args[0] / 2) * sx;
					const hz = (wall.args[2] / 2) * sz;
					const th = ((o.rot ?? 0) * Math.PI) / 180;
					return { x: o.pos[0], z: o.pos[2], r: Math.hypot(hx, hz), hx, hz, cos: Math.cos(th), sin: Math.sin(th) };
				}
				return { x: o.pos[0], z: o.pos[2], r: def.r * Math.max(sx, sz) };
			});
		// ponds are obstacles too — animals route AROUND water. The organic blob bulges to ~1.03×, so pad the radius.
		const ponds: Obstacle[] = (world.zones ?? []).filter((z) => z.material === 'water').map((z) => ({ x: z.pos[0], z: z.pos[2], r: z.size * 1.05 }));
		baseObstacles = [...props, ...ponds];
		feedObstacles(playerState.pos[0], playerState.pos[2]); // re-feed with the fresh static set (+ trees/ponds near player)
		feedWaterSources(); // ponds + settler-dug WELLS → the sim's thirst drink-sources
		sim.setRefuges(world.objects.filter((o) => BUILDING_KINDS.has(o.kind)).map((o) => ({ x: o.pos[0], z: o.pos[2] }))); // houses → flee-to / guard cluster
	}
	// Cheap fingerprint of the STRUCTURE set (non-creature, non-fence) + water zones — count + position + scale folded
	// in, so it changes on any structure add / remove / move but NOT on creature wandering, births, deaths, or fence
	// panel churn. Polled on a throttle below; a change → one rebuild. O(objects) but no box-math / worker post.
	function structFingerprint(): number {
		let n = 0;
		let acc = 0;
		for (const o of world.objects) {
			if (CREATURES.has(o.kind) || o.kind === 'fence') continue;
			n++;
			acc += o.pos[0] * 3.1 + o.pos[2] * 7.7 + (o.scale?.[0] ?? 1) * 13;
		}
		for (const z of world.zones ?? []) if (z.material === 'water') acc += z.pos[0] + z.pos[2] * 1.3 + z.size;
		return n * 1_000_003 + Math.round(acc * 16);
	}
	let lastStructFp = Number.NaN; // forces the first poll to build
	let obstaclePollT = 1; // seconds since the last fingerprint poll (start high → build on frame 1)
	const OBSTACLE_POLL = 0.25; // s between polls → a new structure becomes an obstacle within ¼ s (imperceptible)

	// Reveal objects a few per frame so a big batch ("add 120 cats") mounts gradually instead of all at once
	// (the hang). World-state is applied instantly (share/undo unaffected) — only the visual mount is
	// staggered; pairs with each Prop's pop-in. Clamps down after removals.
	const REVEAL_CAP = 6; // max mounts/frame — each is real work (meshes + an agent), so cap to avoid a hitch
	const REVEAL_FRAMES = 12; // ...but catch a big backlog up within ~this many frames (≈0.2 s) so it's not a slow drip
	let revealed = $state(0);
	// Rust-bred newborns need GLOBALLY-unique ids. A bare 'b<n>' counter resets to 0 each load/HMR, but babies
	// PERSIST in the saved world — so a new 'b0' collided with a loaded 'b0' → duplicate {#each} keys → Svelte
	// aliased two agents onto one mesh (the "man+trex" hybrid) + threw. A per-load random prefix makes every
	// session's babies disjoint from any persisted ones.
	const babyPrefix = 'b' + Math.random().toString(36).slice(2, 8) + '-';
	let babyN = 0;
	// EMERGENT CITIES: settlers raise houses. Per-load unique id prefix (same lesson as babies); grid-snapped so
	// houses align into blocks; one per plot; a global cap so a town doesn't sprawl unbounded.
	const housePrefix = 'h' + Math.random().toString(36).slice(2, 8) + '-';
	let houseN = 0;
	// GRAVES: a headstone rises where a PERSON died (atmosphere + a record of the settlement's history). Capped so a
	// long-lived world doesn't become an endless graveyard — oldest headstones are reclaimed first (FIFO); they also
	// weather away over a very long time once the world-clock fast-forward lands (see save timestamp work).
	const gravePrefix = 'g' + Math.random().toString(36).slice(2, 8) + '-';
	let graveN = 0;
	const wellPrefix = 'w' + Math.random().toString(36).slice(2, 8) + '-';
	let wellN = 0;
	const GRAVE_CAP = 14; // a small cemetery (was 70 — a stone-per-death clump that ate the live budget; see the burial block)
	// VEGETATION: broadleaf (non-pine) trees slowly take root around a SETTLEMENT — people tend gardens/orchards,
	// so a colony greens over time and reads as inhabited, not a bare cluster of boxes (user request). Bounded per
	// building so it stays a leafy town, not a forest swallowing the streets.
	const treePrefix = 'ct' + Math.random().toString(36).slice(2, 8) + '-';
	let treeN = 0;
	const BUILDING_KINDS = new Set(['house', 'cabin', 'tower']);
	const corpseReap = new Set<string>(); // reused each frame → ids of fully-decayed corpses to remove (no per-frame alloc)
	// HABITATION DECAY: an emergent (NPC-built) home that nobody lives in for too long is reclaimed, so a town only
	// persists where people actually settle (big-world "churning steady state"). PLAYER/LLM builds carry `keep` and
	// are NEVER touched; a home with a human nearby resets its idle clock.
	const houseIdle = new Map<string, number>(); // building id → seconds gone uninhabited
	const OCCUPY_R2 = 16 * 16; // a person within this of a home → it's lived-in
	const DECAY_IDLE = 240; // seconds empty before an emergent home is reclaimed
	// IMMIGRATION FLOOR — a living world must not be able to DIE for good. If a key species falls below a critical
	// count (an over-hunted herd, or a world reloaded after a prior collapse — we found one persisted as a lone
	// cat), a small wave wanders in from beyond the treeline to re-seed it. Spawned as ADULTS in a pair (so they
	// can breed back) at the world's edge, gently (a couple per check) so it reads as migration, not a popup. This
	// is the rescue-effect half of a self-sustaining ecosystem; natural breeding (Rust) carries it from there.
	const migrantPrefix = 'm' + Math.random().toString(36).slice(2, 8) + '-';
	let migrantN = 0;
	const RESTOCK_EVERY = 9; // seconds between restock checks (gradual — a collapsed species trickles back over time)
	let restockT = RESTOCK_EVERY - 2; // first check soon after load (so a barren reloaded world revives quickly)
	// MOTHER NATURE wildcards (see nature.svelte.ts): every couple of minutes a pack/herd/boom rolls in to stir the
	// ecosystem (and re-seed extinct apex like dinos). Arrives clustered at the world edge, like an immigration wave.
	const WILDCARD_MIN = 120; // seconds — soonest the next wildcard can fire
	const WILDCARD_MAX = 240;
	let wildcardT = 70 + Math.random() * 60; // first one a bit after you've settled in
	const CLIMATE_PERIOD = 90; // seconds between macro-director climate checks (slow shocks, not twitchy weather)
	let climateT = 40 + Math.random() * 30;

	// LAZY / DISTANCE-CAPPED REVEAL — only realize STATIC builds (houses/trees/props/lamps) NEAR the player; far
	// ones stay unmounted until you approach (they pop in front of you, emerging from the fog/curve). So reloading
	// next to a HUGE city never has to mount + draw the whole thing — the cost is bounded by the radius, not the
	// city size — and the steady draw cost is bounded too. Creatures always mount (few, and they wander off their
	// spawn — their own LOD/impostor system handles their distance). Hysteresis (KEEP > SHOW) so an object at the
	// boundary doesn't flicker mount/unmount as you jitter. The set is recomputed only after moving RECHECK_MOVE,
	// not every frame. (A far-building SILHOUETTE layer — big-world.md §4 — is the next step so the horizon isn't
	// bare; this first kills the jank.)
	const SHOW_R2 = 125 * 125;
	const KEEP_R2 = 150 * 150;
	// STRUCTURES reveal from MUCH farther than props/trees — they're static (cheap once mounted, no per-frame
	// animation), they're the landmarks you navigate by, and the freed scatter budget affords it. Beyond this the
	// SettlementGlows lamp-blooms take over (their fade-in NEAR is matched to ~BUILD_KEEP so there's no double-draw).
	const BUILDINGS = new Set(['house', 'cabin', 'tower', 'manor']);
	const BUILD_SHOW_R2 = 340 * 340; // reveal structures within this (kept moderate — too far floods the keyed {#each}
	const BUILD_KEEP_R2 = 380 * 380; // with house/fence Props that mount/unmount as you move = the jitter the user hit)


	// ── INCREMENTAL SETTLEMENT PLANNER (EF11) — a house cluster accretes town structure as it grows. NO PERIMETER FENCE:
	// it was purely cosmetic (animals never collided with it — the colony FEAR / settlement-avoidance is what keeps
	// wildlife out), and re-fitting a wall around a growing, streaming cluster churned endlessly (layers / half-built /
	// fenceless-on-wake). Ripped out (user). Wells/roads still accrete; a LONE house (n<2) stays exposed.
	// BINARY worldgen (A0, docs/world-data-architecture.md): every structure the binary ops add gets a unique id from
	// ONE prefix; `idBySlot` maps a returned REMOVE slot (the store's SoA index from the last seed) back to its object id.
	const structPrefix = 's' + Math.random().toString(36).slice(2, 8) + '-';
	let structN = 0;
	const idBySlot: string[] = [];
	const RECHECK_MOVE2 = 6 * 6;
	const CREATURE_KINDS = new Set(['person', 'cat', 'lion', 'rabbit', 'kangaroo', 'dinosaur']);
	// immigration FLOORS order (must match worldgen::immigration_ops_bin) — a returned floorIdx indexes this.
	const IMMIGRATION_KINDS = ['rabbit', 'kangaroo', 'person', 'cat', 'lion'];
	let visible = $state<WorldObject[]>([]);
	const shownIds = new Set<string>();
	let lastRevealX = NaN;
	let lastRevealZ = NaN;
	let lastObjLen = -1;
	let lastBudgetLen = -1; // object count at the last live-budget sweep — lets us skip the sweep while the set is stable
	let lastRegion = ''; // player's region cell last frame → only stream when it changes (crossed a tile)
	let lastPulseTick = 0; // last sim-tick we fast-forwarded the dormant world (the ~10 s "world pulse")
	// LIGHTNING — the rainy 'fog' sky flickers with distant sheet lightning: a bright transient added to the
	// ambient so the whole overcast scene lifts for a beat, then decays fast. No bolt geometry / no sound; the
	// sudden brighten alone reads as a far storm. Gated to fog → other skies are untouched (flash stays 0).
	let flash = $state(0);
	let nextBolt = 7 + Math.random() * 12; // seconds until the next strike
	let reflashN = 0; // return strokes left in the current strike (real lightning FLICKERS, it doesn't single-flash)
	let reflashCd = 0; // seconds to the next return stroke
	useTask((dt) => {
		// BINARY worldgen (A0, docs/world-data-architecture.md): the worldgen ops read structures from a persistent
		// binary StructureStore (in the wasm), NOT JSON.stringify(world). Seed it from world.objects' bounded structure
		// set ONCE per frame (the first op that runs this frame), recording idBySlot so a returned REMOVE slot maps back
		// to its object; pack the water zones once too. No JSON either direction; cost is O(local change), days-proof.
		let _seeded = false;
		const ensureSeeded = () => {
			if (_seeded || !math.hasStore) return;
			_seeded = true;
			math.seedStructures(packStructures(world.objects, idBySlot));
		};
		let _zonesBin: Float64Array | null = null;
		const zonesBin = () => (_zonesBin ??= packWaterZones(world.zones, (id) => math.waterSeed(id) ?? 0));
		// apply a binary op stream [op(0=add,1=remove-slot), kind|slot, x, z, rot, sx, sy, sz, color]×n → world.objects.
		const applyStructOps = (ops: Float64Array | null): number => {
			if (!ops) return 0;
			let adds = 0;
			for (let i = 0; i + OP_STRIDE <= ops.length; i += OP_STRIDE) {
				if (ops[i] === OP_ADD) {
					const kind = kindStr(ops[i + 1]);
					const x = ops[i + 2];
					const z = ops[i + 3];
					const o: WorldObject = { id: structPrefix + structN++, kind, pos: [x, heightAt(x, z, world.terrain), z], rot: ops[i + 4], scale: [ops[i + 5], ops[i + 6], ops[i + 7]] };
					if (kind === 'well') o.keep = true; // settler wells survive habitation decay
					if (ops[i + 8]) o.color = '#' + Math.round(ops[i + 8]).toString(16).padStart(6, '0');
					world.objects.push(o);
					adds++;
				} else {
					// OP_REMOVE: lane 1 = the store slot → the object id JS seeded at that index
					const id = idBySlot[ops[i + 1]];
					if (id !== undefined) {
						const idx = world.objects.findIndex((o) => o.id === id);
						if (idx >= 0) world.objects.splice(idx, 1);
					}
				}
			}
			return adds;
		};

		// REPRODUCTION: Rust bred new animals → turn each into a world-object (a baby of that kind), which mounts
		// its renderer + spawns into the sim (as a maturing juvenile). The per-kind cap keeps this bounded.
		const babies = sim.drainBirths();
		if (babies.length) {
			const nowSec = sim.tick() / math.tickHz(); // SIM seconds → birth rate stays correct at any time-lapse speed
			for (const b of babies) {
				world.objects.push({ id: babyPrefix + babyN++, kind: b.kind, pos: [b.x, 0, b.z], juvenile: true, gene: b.gene, pfamA: b.pfamA, pfamB: b.pfamB, genome: b.genome });
				vitals.birth(b.kind, nowSec); // feed the HUD's per-species TFR estimate (numerator)
			}
		}

		// EMERGENT CITIES — house placement is RUST (worldgen build_ops, now BINARY against the StructureStore): colony
		// rules + FOUND_GAP + water margin. JS packs the settler positions, applies the returned add-ops.
		const builds = sim.drainBuilds();
		if (builds.length) {
			ensureSeeded();
			const reqs = new Float64Array(builds.length * 2);
			for (let i = 0; i < builds.length; i++) {
				reqs[i * 2] = builds[i].x;
				reqs[i * 2 + 1] = builds[i].z;
			}
			applyStructOps(math.wgBuild(reqs, zonesBin()));
		}
		// EMERGENT WELLS — placement is RUST (worldgen well_ops, binary): grid-snapped, never in a lake, deduped. JS
		// applies the add-ops + re-runs feedWaterSources so the new wells join the thirst sim.
		const wells = sim.drainWells();
		if (wells.length) {
			ensureSeeded();
			const reqs = new Float64Array(wells.length * 2);
			for (let i = 0; i < wells.length; i++) {
				reqs[i * 2] = wells[i].x;
				reqs[i * 2 + 1] = wells[i].z;
			}
			if (applyStructOps(math.wgWell(reqs, zonesBin())) > 0) feedWaterSources();
		}
		// CORPSE REAPER: a body that's fully decayed (sunk into the earth, see Critter/Npc) is removed from the
		// world — unmounting its renderer + despawning it from the Rust sim, and dropping it from the save. Keeps
		// the now-cyclic world (births ↔ deaths) bounded. Only allocates the id-set on the rare frame one expires.
		corpseReap.clear();
		const gravePts: [number, number][] = []; // PERSONS reaped this frame → buried in their settlement's graveyard
		const deadPeople: [number, number][] = [];
		agentManager.forEach((m) => {
			if (m.dead && m.objId && m.corpseAge > CORPSE_DECAY_SECS) {
				corpseReap.add(m.objId);
				if (m.kind === 'person') deadPeople.push([m.agent.x, m.agent.z]);
			}
		});
		if (deadPeople.length) {
			// the ENGINE picks the grave site (dry plot outside the settlement, or empty for a wild death) — Rust owns it,
			// so it knows water → no graves in lakes. Reads the binary store (seeded once), no JSON.
			ensureSeeded();
			const zb = zonesBin();
			for (const [dpx, dpz] of deadPeople) {
				const g = math.wgGrave(dpx, dpz, zb);
				if (g && g.length >= 2) gravePts.push([g[0], g[1]]);
			}
		}
		if (corpseReap.size > 0) {
			for (let i = world.objects.length - 1; i >= 0; i--) {
				if (corpseReap.has(world.objects[i].id)) world.objects.splice(i, 1);
			}
		}
		if (gravePts.length) {
			// GRAVES — a small CEMETERY, NOT a headstone per death (user: "we don't need 1:1 gravestones"). Each burial
			// only SOMETIMES raises a new stone, jittered into a plot and never within GRAVE_MIN of an existing one, so a
			// town's graveyard fills to a handful of weathered stones then STOPS growing. (A stone-per-death stacked ~70
			// headstones in a tight clump: pure clutter, and it ate ~70 of the live-object slots — starving distant
			// wildlife out of the live set, which is a big part of why everything collapsed into the settlement.)
			const GRAVE_MIN2 = 4 * 4;
			let graves = 0;
			for (const o of world.objects) if (o.kind === 'grave') graves++;
			for (const [gx0, gz0] of gravePts) {
				if (graves >= GRAVE_CAP) break;
				if (Math.random() > 0.34) continue; // sparse — most deaths add no stone (atmosphere, not a census)
				const gx = gx0 + (Math.random() - 0.5) * 14;
				const gz = gz0 + (Math.random() - 0.5) * 14;
				if (world.objects.some((o) => o.kind === 'grave' && (o.pos[0] - gx) ** 2 + (o.pos[2] - gz) ** 2 < GRAVE_MIN2)) continue; // a stone already marks this plot
				world.objects.push({ id: gravePrefix + graveN++, kind: 'grave', pos: [gx, heightAt(gx, gz, world.terrain), gz], rot: Math.random() * Math.PI * 2 });
				graves++;
			}
		}
		// (The cemetery's FIFO cap is enforced on the slow restock cadence below — see trimGraves — so a freshly LOADED
		// world sheds a legacy pile of headstones promptly, not only when the next person happens to die.)

		// IMMIGRATION: rescue any species that's dropped below its floor (extinction-proofing). Counts the LIVE
		// agents, and for each deficient kind walks in a couple of adults from the edge so a wiped-out herd can
		// rebuild itself (and then breed naturally). Throttled to a slow cadence so it's a trickle, not a flood.
		restockT += dt;
		if (restockT >= RESTOCK_EVERY) {
			restockT = 0;
			// trimGraves: enforce the cemetery FIFO cap — clears a legacy save's pile down to GRAVE_CAP within one
			// restock of load, and bounds ongoing growth. Remove excess graves from the FRONT of the array (insertion
			// order ≈ chronological → oldest first); position-based, NOT id-parsed (legacy saves used dashless grave ids
			// that parsed to NaN → the old trim broke immediately and a 70-grave pile never shed). Cheap O(n), per restock.
			let graveCount = 0;
			for (const o of world.objects) if (o.kind === 'grave') graveCount++;
			if (graveCount > GRAVE_CAP) {
				let toRemove = graveCount - GRAVE_CAP;
				world.objects = world.objects.filter((o) => !(o.kind === 'grave' && toRemove > 0 && toRemove--));
			}
			// feed the world's AREA to the sim → prey caps scale with the (growing) world/city; predators follow prey
			sim.setPopScale(worldAreaScale(world.objects));
			const live: Record<string, number> = {};
			const geneSum: Record<string, number> = {};
			let allGene = 0;
			let allN = 0;
			agentManager.forEach((m) => {
				if (m.dead) return;
				live[m.kind] = (live[m.kind] ?? 0) + 1;
				geneSum[m.kind] = (geneSum[m.kind] ?? 0) + (m.gene ?? 1); // accumulate VIGOR so immigrants match the evolved stock
				allGene += m.gene ?? 1;
				allN++;
			});
			const globalAvg = allN > 0 ? allGene / allN : 1; // fallback vigor for a species that's gone fully extinct
			// the ENGINE decides the rescue (worldgen::immigration_ops): which deficient kinds, how many, rescued vigour,
			// clustered near the player. We hand it the live counts gathered above; it returns add-creature ops (with gene).
			// counts → a flat [n, geneSum]×5 in FLOORS order (rabbit, kangaroo, person, cat, lion). BINARY: no JSON crossing.
			const cbin = new Float64Array(10);
			for (let i = 0; i < IMMIGRATION_KINDS.length; i++) {
				cbin[i * 2] = live[IMMIGRATION_KINDS[i]] ?? 0;
				cbin[i * 2 + 1] = geneSum[IMMIGRATION_KINDS[i]] ?? 0;
			}
			const migStream = math.wgImmigration(cbin, playerState.pos[0], playerState.pos[2], globalAvg, sim.tick());
			if (migStream) {
				for (let i = 0; i + 4 <= migStream.length; i += 4) {
					world.objects.push({ id: migrantPrefix + migrantN++, kind: IMMIGRATION_KINDS[migStream[i]], pos: [migStream[i + 1], 0, migStream[i + 2]], gene: migStream[i + 3] });
				}
			}
				// COLONY VEGETATION — placement is RUST now (worldgen::vegetation_ops): a broadleaf occasionally roots near a
				// home so a town greens over time (bounded, never on a plot/tree/lake). Seeded by the tick for the gradual roll.
				ensureSeeded();
				applyStructOps(math.wgVeg(sim.tick(), zonesBin()));			// HABITATION DECAY: an emergent home that nobody lives in for too long is reclaimed — a town persists
			// only where people actually settle. PLAYER/LLM builds carry `keep` and are NEVER touched; a person
			// within OCCUPY_R of a home resets its idle clock.
			const humansAt: [number, number][] = [];
			agentManager.forEach((m) => {
				if (!m.dead && m.kind === 'person') humansAt.push([m.agent.x, m.agent.z]);
			});
			let anyDecayed = false;
			for (const o of world.objects) {
				if (!BUILDING_KINDS.has(o.kind) || o.keep) continue;
				const lived = humansAt.some(([hx, hz]) => (hx - o.pos[0]) ** 2 + (hz - o.pos[2]) ** 2 < OCCUPY_R2);
				const idle = lived ? 0 : (houseIdle.get(o.id) ?? 0) + RESTOCK_EVERY;
				houseIdle.set(o.id, idle);
				if (idle > DECAY_IDLE) anyDecayed = true;
			}
			if (anyDecayed) {
				for (let i = world.objects.length - 1; i >= 0; i--) {
					const o = world.objects[i];
					if (BUILDING_KINDS.has(o.kind) && !o.keep && (houseIdle.get(o.id) ?? 0) > DECAY_IDLE) {
						world.objects.splice(i, 1);
						houseIdle.delete(o.id);
					}
				}
			}
		}

		// MOTHER NATURE: every couple of minutes, roll a wildcard — a pack/herd/boom enters at the world edge (one
		// clustered arrival, like an immigration wave but bigger + announced) so the world is never static, and
		// rare apex species (dinos) get re-seeded after extinction.
		wildcardT -= dt;
		if (wildcardT <= 0) {
			wildcardT = WILDCARD_MIN + Math.random() * (WILDCARD_MAX - WILDCARD_MIN);
			const wc = nature.roll();
			if (wc) {
				const a = Math.random() * Math.PI * 2;
				const r = 60 + Math.random() * 35; // arrives from beyond the near field
				const bx = playerState.pos[0] + Math.cos(a) * r;
				const bz = playerState.pos[2] + Math.sin(a) * r;
				for (let k = 0; k < wc.count; k++) {
					const x = bx + (Math.random() - 0.5) * 14;
					const z = bz + (Math.random() - 0.5) * 14;
					const gene = math.clampGene(wc.gene - 0.08 + Math.random() * 0.2);
					world.objects.push({ id: migrantPrefix + migrantN++, kind: wc.kind, pos: [x, 0, z], gene });
				}
				nature.announce(wc.banner);
			}
		}

		// MACRO-DIRECTOR climate shock (nature.directClimate, the LLM seam): on a slow timer read the world's pulse
		// and steer the DROUGHT level — a boom brings a drought that thins the herds at the shrinking water; a
		// crash brings the rains. WHOLE-WORLD headcount (live + dormant aggregates), so streaming-away most of the
		// world doesn't fool the director into thinking it's empty. Feeds set_aridity; banner only on a real shift.
		climateT -= dt;
		if (climateT <= 0) {
			climateT = CLIMATE_PERIOD;
			let pop = 0;
			agentManager.forEach((m) => {
				if (!m.dead) pop++;
			});
			if (world.regions) for (const key in world.regions) for (const k in world.regions[key].counts) pop += world.regions[key].counts[k];
			const c = nature.directClimate(pop);
			sim.setAridity(c.aridity);
			if (c.banner) nature.announce(c.banner);
		}

		wind.uTime.value += dt; // the ONE shared foliage clock — Tree + AmbientScatter read it (never tick it)
		weather.uWet.value = world.sky === 'fog' ? 1 : 0; // shared rain-wetness → prop materials soak in the rain
		weather.uSnow.value = world.ground === 'snow' ? 1 : 0; // shared snow → up-faces of props/rocks/bushes cap white
		if (world.sky === 'fog') {
			nextBolt -= dt;
			if (nextBolt <= 0) {
				flash = 1.5; // main stroke
				nextBolt = 6 + Math.random() * 16;
				reflashN = Math.random() < 0.6 ? (Math.random() < 0.4 ? 2 : 1) : 0; // often 1–2 fainter return strokes
				reflashCd = 0.06 + Math.random() * 0.05;
			}
			if (reflashN > 0) {
				reflashCd -= dt;
				if (reflashCd <= 0) {
					flash = Math.max(flash, 0.7 + Math.random() * 0.5); // a fainter flicker, mid-decay → it stutters
					reflashN--;
					reflashCd = 0.05 + Math.random() * 0.06;
				}
			}
		}
		flash = Math.max(0, flash - dt * 8); // ~0.18 s decay (set-to-0 is a no-op once settled → no idle churn)
		weather.uFlash.value = flash; // share the strike → the cloud deck flashes white too (sky-side lightning)

		// Keep the sun's shadow frustum centred on the player so shadows render across the ENDLESS world. The
		// light was pinned to the origin (frustum ±60 m), so walking ~60 m from spawn dropped ALL shadows. Move
		// the light WITH the player at a fixed offset → the light DIRECTION (and thus shadow angle) is unchanged,
		// only the covered box slides. Snap the centre to whole shadow texels so edges don't shimmer while moving.
		if (sun) {
			const cx = Math.round(playerState.pos[0] / SHADOW_TEXEL) * SHADOW_TEXEL;
			const cz = Math.round(playerState.pos[2] / SHADOW_TEXEL) * SHADOW_TEXEL;
			sun.position.set(cx + 30, 45, cz + 20);
			sun.target.position.set(cx, 0, cz);
			sun.target.updateMatrixWorld();
		}
		// throttled shadow re-render — flag needsUpdate BEFORE three renders this frame (autoUpdate is off above)
		shadowTick = (shadowTick + 1) % SHADOW_EVERY;
		if (shadowTick === 0) renderer.shadowMap.needsUpdate = true;
		// recompute the NEAR set only after moving far enough or the world changed (not every frame)
		const px = playerState.pos[0];
		const pz = playerState.pos[2];
		// REGION STREAMING: when the player crosses a region tile, SLEEP the far regions (creatures → cheap aggregate,
		// leave the sim) and WAKE the near ones (fast-forward + re-materialise). This keeps the sim bounded to the near
		// area instead of ticking every creature in the world — a pure PERF mechanism (single-player, all in-memory).
		const [rcx, rcz] = regionOf(px, pz);
		const rkey = rcx + ',' + rcz;
		if (rkey !== lastRegion) {
			lastRegion = rkey;
			// SETUP-ONLY wake (wakeBatch 0): on a crossing, sleep the far regions + fast-forward/restore-statics the near
			// ones, but materialise NO creatures this frame. The per-frame `drainWakes` below dribbles them in (~12/frame)
			// so entering a region no longer WAKE-STORMs its whole population into one frame (visible jitter + pop-in).
			streamRegions(world, px, pz, sim.tick(), 'rg', 0);
		}
		// PER-FRAME WAKE DRAINER — materialise a few pending creatures from each active region's aggregate every frame, so
		// a region's population streams in smoothly over ~N frames instead of all at once on the crossing. No-op (0) once
		// the active regions are fully awake. Runs alongside the world-pulse / live-budget block below (same px/pz/tick).
		drainWakes(world, px, pz, sim.tick());
		// WORLD PULSE: every ~10 s (300 sim ticks @30 Hz), fast-forward EVERY dormant region to now so the far world
		// keeps living (grows toward carrying capacity + evolves) instead of freezing until visited. Cheap closed-form.
		const simTick = sim.tick();
		if (simTick - lastPulseTick > 300) {
			lastPulseTick = simTick;
			fastForwardDormant(world, simTick);
		}
		// (settlement perimeter walls were RIPPED OUT — they were cosmetic; the colony-FEAR, not a wall, keeps wildlife
		// out. No more re-fitting a ring around a growing/streaming cluster. The Rust settlement_ops fitter is now dead.)
		// HARD LIVE BUDGET — a MAX CAP, not a target (user): if the near area is densely packed past the per-class
		// budgets, offload the FARTHEST excess into dormant aggregates — they leave the live set entirely (despawn from
		// the Rust sim AND stop drawing, not just hide), and rejoin as you approach. CREATURES and STRUCTURES have
		// SEPARATE budgets, so a structure-dense town never evicts distant wildlife to fit its own homes/fences/graves
		// (that's what collapsed everything into the settlement). enforceLiveBudget early-outs when both are under cap.
		// only re-run the sweep when the live set actually CHANGED (births / reveals / wake). Its O(objects) count
		// pass runs every frame, but the result can't change while the object set is stable — and any addition bumps
		// the length, so the sweep always runs before the set can grow past the cap (it can never silently overflow).
		if (world.objects.length !== lastBudgetLen) {
			enforceLiveBudget(world, px, pz, sim.tick());
			lastBudgetLen = world.objects.length; // post-sweep length (it may have offloaded the farthest excess)
		}
		const objLen = world.objects.length;
		if (objLen !== lastObjLen || Number.isNaN(lastRevealX) || (px - lastRevealX) ** 2 + (pz - lastRevealZ) ** 2 > RECHECK_MOVE2) {
			lastRevealX = px;
			lastRevealZ = pz;
			lastObjLen = objLen;
			const next: WorldObject[] = [];
			const seen = new Set<string>(); // GUARD: never feed a duplicate id into the keyed {#each} → it throws
			// `each_key_duplicate` and white-screens the app. Skips a stray dup (e.g. a legacy persisted wake-id
			// collision from before the materializeSeq fix) instead of crashing.
			for (const o of world.objects) {
				if (seen.has(o.id)) continue;
				if (o.kind === 'fence' || o.kind === 'grave' || o.kind === 'rock' || o.kind === 'flower' || o.kind === 'bush' || o.kind === 'well' || o.kind === 'bridge') continue;
				// these all render via INSTANCED renderers (Fences/Graves/Rocks/Flowers/Bushes/Wells/Bridges) — keep them OUT of
				// the keyed {#each} so a walled town / scattered field no longer mounts/unmounts Props w/ geometry/material/RigidBody.
				if (CREATURE_KINDS.has(o.kind)) {
					next.push(o); // creatures always render (few; they wander; LOD/impostors handle their distance)
					seen.add(o.id);
					continue;
				}
				const d2 = (o.pos[0] - px) ** 2 + (o.pos[2] - pz) ** 2;
				const isBuild = BUILDINGS.has(o.kind); // structures reveal much farther than props/trees
				const showR = isBuild ? BUILD_SHOW_R2 : SHOW_R2;
				const keepR = isBuild ? BUILD_KEEP_R2 : KEEP_R2;
				const keep = shownIds.has(o.id) ? d2 < keepR : d2 < showR; // hysteresis: hold a shown one until past KEEP
				if (keep) {
					next.push(o);
					seen.add(o.id);
					shownIds.add(o.id);
				} else {
					shownIds.delete(o.id);
				}
			}
			visible = next;
		}
		// re-feed the near-forest trunks to the Rust collision as the player moves (coarse threshold — the 140m
		// feed radius has margin, so animals always have the trees around them even before the next re-feed)
		// STATIC obstacles / refuges / water: poll a cheap structure fingerprint on a throttle (NOT a reactive $effect
		// on world.objects — that re-ran the whole rebuild on every birth/death/wander → the hunt + T-rex-drop jank).
		// Only a real structure add/remove/move rebuilds; creature + fence churn is invisible to it.
		obstaclePollT += dt;
		if (obstaclePollT >= OBSTACLE_POLL) {
			obstaclePollT = 0;
			const fp = structFingerprint();
			if (fp !== lastStructFp) {
				lastStructFp = fp;
				rebuildStaticObstacles();
			}
		}
		if (Number.isNaN(lastTreeFeedX) || (px - lastTreeFeedX) ** 2 + (pz - lastTreeFeedZ) ** 2 > TREE_REFEED2) {
			feedObstacles(px, pz);
		}
		// redraw the nearby NATURAL ponds (Rust's pond field) as the player roams — coarse threshold, ~⅓ the render
		// radius of margin so ponds are always in place before you reach them.
		if (Number.isNaN(lastPondX) || (px - lastPondX) ** 2 + (pz - lastPondZ) ** 2 > 120 * 120) {
			refreshPonds(px, pz);
		}
		// time-slice the MOUNT of the near set (a few/frame) so striding into a dense block doesn't hitch
		const n = visible.length;
		const backlog = n - revealed;
		if (backlog > 0) revealed = Math.min(n, revealed + Math.min(REVEAL_CAP, Math.max(1, Math.ceil(backlog / REVEAL_FRAMES))));
		else if (revealed > n) revealed = n; // clamp after removals / objects leaving range
	});
</script>

<SkyDome sky={world.sky} ground={world.ground} />

<T.FogExp2 attach="fog" args={[fog.color, fog.density]} />

<T.AmbientLight intensity={light.amb + flash} color={light.ambC} />
<T.DirectionalLight
	bind:ref={sun}
	intensity={light.dir}
	color={light.dirC}
	castShadow
	shadow.mapSize.width={2048}
	shadow.mapSize.height={2048}
	shadow.camera.left={-60}
	shadow.camera.right={60}
	shadow.camera.top={60}
	shadow.camera.bottom={-60}
	shadow.camera.near={1}
	shadow.camera.far={160}
	shadow.bias={-0.0004}
/>

<Terrain {world} />
<AmbientScatter {world} />
<SettlementGlows {world} />
<!-- DECORATIVE LAYERS — dropped on the LOW quality tier (weak devices) to hold the frame rate. Pure eye-candy with
     no gameplay effect; the core world (terrain, grass, agents, shadows, settlement glow) always renders. -->
{#if !quality.low}
	<LoveHearts {world} />
	<Chimneys {world} />
	<DustPuffs {world} />
	<SpawnPuffs {world} />
	<FootPrints {world} />
	<SplashDrops {world} />
	<AmbientParticles sky={world.sky} />
	<FallingLeaves sky={world.sky} ground={world.ground} />
	<Weather ground={world.ground} sky={world.sky} />
	<Mist {world} />
	<Birds {world} />
	<Birds {world} mode="bat" />
	<Butterflies {world} />
{/if}
<Grass {world} />
<AgentSystem />
<Profiler />
<AgentImpostors {world} />
<InstancedCreatures {world} />
<CreatureShadows {world} />
<PlacedShadows {world} />
<LampGlow {world} />
<BuildingGlow {world} />
<MoveGhost {world} />
<Critter {world} species="cat" companion />

{#each world.zones ?? [] as z (z.id)}
	{#if z.material === 'water'}
		<Water zone={z} sky={world.sky} terrain={world.terrain} />
		<LakeFish zone={z} terrain={world.terrain} />
	{:else}
		<Zone zone={z} />
	{/if}
{/each}
<!-- NATURAL ponds (Rust-owned water field). WATER LOD: full shader for the few NEAR you; a cheap flat blue disc
     for the rest. No per-pond LakeFish shoal (a per-frame breeding sim ×N ponds was a needless mount-storm). -->
{#each nearPonds as z (z.id)}
	<Water zone={z} sky={world.sky} terrain={world.terrain} />
{/each}
{#each farPonds as p (p.id)}
	<T.Mesh geometry={POND_BLOB_GEO} material={POND_BLOB_MAT} position={[p.x, p.y, p.z]} scale={[p.r, 1, p.r]} />
{/each}

{#each world.paths ?? [] as p (p.id)}
	<Path path={p} {world} />
{/each}

<!-- all town walls in two instanced draw calls (no per-panel Prop/RigidBody) -->
<!-- all cemetery headstones in four instanced draw calls -->
<Graves {world} />
<!-- all scattered boulders in two instanced draw calls (per-rock colour via instanceColor) -->
<Rocks {world} />
<!-- all wildflowers in two instanced draw calls (per-bloom palette colour via instanceColor) -->
<Flowers {world} />
<!-- all placed shrubs in three instanced draw calls (per-bush palette colour via instanceColor) -->
<Bushes {world} />
<!-- all village wells in three instanced draw calls -->
<Wells {world} />
<!-- all plank bridges: instanced visual + kept per-bridge deck colliders (walkable) -->
<Bridges {world} />

{#each visible.slice(0, revealed) as obj (obj.id)}
	{#if obj.kind === 'person'}
		<Npc {obj} {world} />
	{:else if obj.kind === 'cat' || obj.kind === 'lion' || obj.kind === 'rabbit' || obj.kind === 'kangaroo' || obj.kind === 'dinosaur'}
		<Critter {obj} {world} species={obj.kind} />
	{:else if obj.kind === 'tree' || obj.kind === 'pine'}
		<Tree {obj} />
	{:else if obj.kind === 'house' || obj.kind === 'cabin' || obj.kind === 'tower'}
		<Building {obj} {world} />
	{:else if obj.kind === 'lamp'}
		<Lamp {obj} {world} />
	{:else}
		<Prop {obj} />
	{/if}
{/each}
