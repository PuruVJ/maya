<script lang="ts">
	import { T, useTask, useThrelte } from '@threlte/core';
	import Prop from './Prop.svelte';
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
	import Skyline from './Skyline.svelte';
	import SettlementGlows from './SettlementGlows.svelte';
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
	import SkyDome from './SkyDome.svelte';
	import { SKY_FOG, kindDef } from '$lib/kinds';
	import { treeAt, treeRadius, onPath, SCATTER_STEP } from '$lib/scatter';
	import { setEyeshine } from '$lib/sharedAssets';
	import { drainBirths, drainBuilds } from '$lib/rustSim';
	import { agentManager, CORPSE_DECAY_SECS } from '$lib/agents.svelte';
	import { setRustObstacles } from '$lib/rustSim';
	import { playerState } from '$lib/playerState.svelte';
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
	let lastTreeFeedX = NaN;
	let lastTreeFeedZ = NaN;

	// The Rust sim has no ambient forest of its own (it's a deterministic per-cell function shared with the
	// renderer + player — NOT bit-exact to port). So instead of porting the scatter, we feed the SAME trunks
	// the renderer draws (treeAt, path-culled) near the player as circle-obstacles → animals route around the
	// very trees you see, no divergence. Combined with the static props/ponds each feed.
	function feedObstacles(px: number, pz: number): void {
		const trees: Obstacle[] = [];
		const c0 = Math.floor((px - TREE_FEED_R) / SCATTER_STEP);
		const c1 = Math.floor((px + TREE_FEED_R) / SCATTER_STEP);
		const d0 = Math.floor((pz - TREE_FEED_R) / SCATTER_STEP);
		const d1 = Math.floor((pz + TREE_FEED_R) / SCATTER_STEP);
		const paths = world.paths;
		for (let ci = c0; ci <= c1; ci++) {
			for (let cj = d0; cj <= d1; cj++) {
				const tr = treeAt(ci, cj);
				if (!tr || onPath(paths, tr.x, tr.z)) continue; // culled on roads → not drawn → don't collide
				trees.push({ x: tr.x, z: tr.z, r: treeRadius(tr.scale) });
			}
		}
		lastTreeFeedX = px;
		lastTreeFeedZ = pz;
		setRustObstacles([...baseObstacles, ...trees]); // the Rust sim resolves these solids (push-out, no tunnelling)
	}

	$effect(() => {
		const props: Obstacle[] = world.objects
			.filter((o) => !CREATURES.has(o.kind))
			.map((o) => {
				const def = kindDef(o.kind);
				const sx = o.scale?.[0] ?? 1;
				const sz = o.scale?.[2] ?? 1;
				const wall = def.parts[0];
				// box-footprint kinds (houses/cabins) → ORIENTED BOX so animals hug walls / use streets like the
				// player; round kinds stay circles. r = bounding radius so the obstacle grid still finds the box.
				if (wall && wall.geo === 'box') {
					const hx = (wall.args[0] / 2) * sx;
					const hz = (wall.args[2] / 2) * sz;
					const th = ((o.rot ?? 0) * Math.PI) / 180;
					return { x: o.pos[0], z: o.pos[2], r: Math.hypot(hx, hz), hx, hz, cos: Math.cos(th), sin: Math.sin(th) };
				}
				return { x: o.pos[0], z: o.pos[2], r: def.r * Math.max(sx, sz) };
			});
		// ponds are obstacles too — animals route AROUND water (the player may still wade in). The organic blob
		// BULGES out to ~1.03× size at some angles, so the avoidance radius must be a bit bigger than `size` or
		// animals wander onto the bulges (looked like "cats walking on the lake").
		const ponds: Obstacle[] = (world.zones ?? [])
			.filter((z) => z.material === 'water')
			.map((z) => ({ x: z.pos[0], z: z.pos[2], r: z.size * 1.05 }));
		baseObstacles = [...props, ...ponds];
		feedObstacles(playerState.pos[0], playerState.pos[2]); // re-feed with the current near-forest
	});

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
	const BUILDING_KINDS = new Set(['house', 'cabin', 'tower']);
	const HOUSE_CAP = 140;
	const corpseReap = new Set<string>(); // reused each frame → ids of fully-decayed corpses to remove (no per-frame alloc)
	// IMMIGRATION FLOOR — a living world must not be able to DIE for good. If a key species falls below a critical
	// count (an over-hunted herd, or a world reloaded after a prior collapse — we found one persisted as a lone
	// cat), a small wave wanders in from beyond the treeline to re-seed it. Spawned as ADULTS in a pair (so they
	// can breed back) at the world's edge, gently (a couple per check) so it reads as migration, not a popup. This
	// is the rescue-effect half of a self-sustaining ecosystem; natural breeding (Rust) carries it from there.
	const migrantPrefix = 'm' + Math.random().toString(36).slice(2, 8) + '-';
	let migrantN = 0;
	const IMMIGRATION: Record<string, number> = { rabbit: 6, kangaroo: 4, person: 4, cat: 2, lion: 1 };
	const RESTOCK_EVERY = 9; // seconds between restock checks (gradual — a collapsed species trickles back over time)
	let restockT = RESTOCK_EVERY - 2; // first check soon after load (so a barren reloaded world revives quickly)

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
	const RECHECK_MOVE2 = 6 * 6;
	const CREATURE_KINDS = new Set(['person', 'cat', 'lion', 'rabbit', 'kangaroo', 'dinosaur']);
	let visible = $state<WorldObject[]>([]);
	const shownIds = new Set<string>();
	let lastRevealX = NaN;
	let lastRevealZ = NaN;
	let lastObjLen = -1;
	// LIGHTNING — the rainy 'fog' sky flickers with distant sheet lightning: a bright transient added to the
	// ambient so the whole overcast scene lifts for a beat, then decays fast. No bolt geometry / no sound; the
	// sudden brighten alone reads as a far storm. Gated to fog → other skies are untouched (flash stays 0).
	let flash = $state(0);
	let nextBolt = 7 + Math.random() * 12; // seconds until the next strike
	let reflashN = 0; // return strokes left in the current strike (real lightning FLICKERS, it doesn't single-flash)
	let reflashCd = 0; // seconds to the next return stroke
	useTask((dt) => {
		// REPRODUCTION: Rust bred new animals → turn each into a world-object (a baby of that kind), which mounts
		// its renderer + spawns into the sim (as a maturing juvenile). The per-kind cap keeps this bounded.
		const babies = drainBirths();
		for (const b of babies) {
			world.objects.push({ id: babyPrefix + babyN++, kind: b.kind, pos: [b.x, 0, b.z], juvenile: true, gene: b.gene });
		}

		// EMERGENT CITIES: place the houses settlers raised this frame. Snap to an 8 m grid (→ aligned blocks),
		// skip an already-occupied plot, and stop at HOUSE_CAP so a town grows but never sprawls without bound.
		const builds = drainBuilds();
		if (builds.length) {
			let houses = 0;
			for (const o of world.objects) if (BUILDING_KINDS.has(o.kind)) houses++;
			for (const bd of builds) {
				if (houses >= HOUSE_CAP) break;
				const gx = Math.round(bd.x / 8) * 8;
				const gz = Math.round(bd.z / 8) * 8;
				if (world.objects.some((o) => BUILDING_KINDS.has(o.kind) && Math.abs(o.pos[0] - gx) < 6 && Math.abs(o.pos[2] - gz) < 6)) continue; // plot taken
				world.objects.push({ id: housePrefix + houseN++, kind: 'house', pos: [gx, 0, gz] });
				houses++;
			}
		}

		// CORPSE REAPER: a body that's fully decayed (sunk into the earth, see Critter/Npc) is removed from the
		// world — unmounting its renderer + despawning it from the Rust sim, and dropping it from the save. Keeps
		// the now-cyclic world (births ↔ deaths) bounded. Only allocates the id-set on the rare frame one expires.
		corpseReap.clear();
		agentManager.forEach((m) => {
			if (m.dead && m.objId && m.corpseAge > CORPSE_DECAY_SECS) corpseReap.add(m.objId);
		});
		if (corpseReap.size > 0) {
			for (let i = world.objects.length - 1; i >= 0; i--) {
				if (corpseReap.has(world.objects[i].id)) world.objects.splice(i, 1);
			}
		}

		// IMMIGRATION: rescue any species that's dropped below its floor (extinction-proofing). Counts the LIVE
		// agents, and for each deficient kind walks in a couple of adults from the edge so a wiped-out herd can
		// rebuild itself (and then breed naturally). Throttled to a slow cadence so it's a trickle, not a flood.
		restockT += dt;
		if (restockT >= RESTOCK_EVERY) {
			restockT = 0;
			const live: Record<string, number> = {};
			agentManager.forEach((m) => {
				if (!m.dead) live[m.kind] = (live[m.kind] ?? 0) + 1;
			});
			for (const kind in IMMIGRATION) {
				const deficit = IMMIGRATION[kind] - (live[kind] ?? 0);
				if (deficit <= 0) continue;
				const bring = Math.min(2, deficit); // a pair at a time → both sexes arrive over successive waves
				for (let k = 0; k < bring; k++) {
					const a = Math.random() * Math.PI * 2;
					const r = 55 + Math.random() * 30; // beyond the immediate clearing, within the wander range
					const x = playerState.pos[0] + Math.cos(a) * r;
					const z = playerState.pos[2] + Math.sin(a) * r;
					world.objects.push({ id: migrantPrefix + migrantN++, kind, pos: [x, 0, z] });
				}
			}
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
		const objLen = world.objects.length;
		if (objLen !== lastObjLen || Number.isNaN(lastRevealX) || (px - lastRevealX) ** 2 + (pz - lastRevealZ) ** 2 > RECHECK_MOVE2) {
			lastRevealX = px;
			lastRevealZ = pz;
			lastObjLen = objLen;
			const next: WorldObject[] = [];
			for (const o of world.objects) {
				if (CREATURE_KINDS.has(o.kind)) {
					next.push(o); // creatures always render (few; they wander; LOD/impostors handle their distance)
					continue;
				}
				const d2 = (o.pos[0] - px) ** 2 + (o.pos[2] - pz) ** 2;
				const keep = shownIds.has(o.id) ? d2 < KEEP_R2 : d2 < SHOW_R2; // hysteresis: hold a shown one until past KEEP
				if (keep) {
					next.push(o);
					shownIds.add(o.id);
				} else {
					shownIds.delete(o.id);
				}
			}
			visible = next;
		}
		// re-feed the near-forest trunks to the Rust collision as the player moves (coarse threshold — the 140m
		// feed radius has margin, so animals always have the trees around them even before the next re-feed)
		if (Number.isNaN(lastTreeFeedX) || (px - lastTreeFeedX) ** 2 + (pz - lastTreeFeedZ) ** 2 > TREE_REFEED2) {
			feedObstacles(px, pz);
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
<Skyline {world} />
<SettlementGlows {world} />
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
<Grass {world} />
<AgentSystem />
<AgentImpostors {world} />
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

{#each world.paths ?? [] as p (p.id)}
	<Path path={p} {world} />
{/each}

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
