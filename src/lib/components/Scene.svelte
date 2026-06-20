<script lang="ts">
	import { T, useTask } from '@threlte/core';
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
	import { agentManager } from '$lib/agents.svelte';
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

	// Scene lighting follows the sky so NIGHT is actually dark (was constant → night looked like day with a
	// different backdrop, wasting the lit-window/lamp glow). Moonlit cool-blue at night, warm low sun at
	// sunset, flat grey under fog. Intensity + colour shift here; the sun DIRECTION stays fixed (its position
	// tracks the player each frame in the useTask so shadows render across the endless world — same angle).
	const LIGHT: Record<string, { amb: number; dir: number; ambC: string; dirC: string }> = {
		day: { amb: 0.65, dir: 1.15, ambC: '#ffffff', dirC: '#fff6e0' },
		sunset: { amb: 0.5, dir: 0.8, ambC: '#ffd6b0', dirC: '#ff8f4d' },
		fog: { amb: 0.6, dir: 0.3, ambC: '#aeb6c2', dirC: '#c2c9d4' }, // overcast/RAIN: dimmer + greyer than a clear day (was brighter than day!), weak diffuse sun
		night: { amb: 0.58, dir: 0.85, ambC: '#9fb3da', dirC: '#c2d2f4' }, // clearly lit, just cool & blue (not dark)
		space: { amb: 0.52, dir: 0.72, ambC: '#8a9ace', dirC: '#aab8e4' }
	};
	const light = $derived(LIGHT[world.sky] ?? LIGHT.day);

	// tell the food chain how nocturnal it is → prey jumpier, predators keener after dark
	const NIGHTNESS: Record<string, number> = { day: 0, sunset: 0.4, fog: 0.25, night: 1, space: 1 };
	$effect(() => agentManager.setNight(NIGHTNESS[world.sky] ?? 0));

	// the living kinds steer themselves (flocking) — every OTHER object is a solid prop that ambient
	// animals must route around, so feed their footprints to the agent manager whenever the world changes
	const CREATURES = new Set(['person', 'cat', 'lion', 'rabbit', 'kangaroo', 'dinosaur']);
	$effect(() => {
		const props = world.objects
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
		const ponds = (world.zones ?? [])
			.filter((z) => z.material === 'water')
			.map((z) => ({ x: z.pos[0], z: z.pos[2], r: z.size * 1.05 }));
		agentManager.setObstacles([...props, ...ponds]);
		agentManager.setPaths(world.paths ?? []); // so animals skip ambient trees culled on roads
	});

	// Reveal objects a few per frame so a big batch ("add 120 cats") mounts gradually instead of all at once
	// (the hang). World-state is applied instantly (share/undo unaffected) — only the visual mount is
	// staggered; pairs with each Prop's pop-in. Clamps down after removals.
	const REVEAL_CAP = 6; // max mounts/frame — each is real work (meshes + an agent), so cap to avoid a hitch
	const REVEAL_FRAMES = 12; // ...but catch a big backlog up within ~this many frames (≈0.2 s) so it's not a slow drip
	let revealed = $state(0);

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
