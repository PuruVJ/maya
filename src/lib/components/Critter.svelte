<script lang="ts">
	// One roaming animal — cat / lion / rabbit / kangaroo / dinosaur. Each species has a DISTINCT
	// low-poly body (its own snippet below) so silhouettes read apart (a kangaroo is bipedal on a heavy
	// tail, not a standing cat). All snippets bind the SAME refs (head, tail, four legs) so the single
	// procedural-locomotion loop drives them — the gait MODE varies (quad trot / bound / biped). Steering
	// + the food-chain sim (hunt/flee/stamina/death) live in the shared agentManager. See docs/.
	import { untrack } from 'svelte';
	import { T, useTask } from '@threlte/core';
	import { Billboard } from '@threlte/extras';
	import * as THREE from 'three';
	import { heightAt } from '$lib/terrain';
	import { playerState } from '$lib/playerState.svelte';
	import { Agent, Spring, type Behavior } from '$lib/steering';
	import { agentManager, makeManaged, speedFor, LOD2_DIST, CORPSE_DECAY_SECS, CORPSE_SINK_SECS, type ManagedAgent } from '$lib/agents.svelte';
	import { seedFrom } from '$lib/rng';
	import { clock } from '$lib/clock';
	import { PRIM, litMat, creatureMat, EYE_PREY_MAT, EYE_PRED_MAT, EYE_HUNT_MAT, type CoatPattern } from '$lib/sharedAssets';
	import type { World, WorldObject } from '$lib/world';

	type Gait = 'quad' | 'hop' | 'bipedHop' | 'bipedWalk';
	// poses are baked into each species' geometry, so no static lean is needed; `scale` sets overall size.
	// `coat` picks the procedural shader pattern: cat=tabby stripes, lion=fur grain, dinosaur=reptile scales,
	// rabbit/kangaroo get the SOFT coat — a gentle dapple + a pale counter-shaded belly (subtle fluff, not a
	// pattern), so they read as soft mammals with the light underside real rabbits/roos have.
	const SPECIES: Record<string, { scale: number; body: string; accent: string; gait: Gait; coat: CoatPattern }> = {
		cat: { scale: 1.0, body: '#e8924a', accent: '#d9823c', gait: 'quad', coat: 'stripe' },
		lion: { scale: 1.45, body: '#c79a4b', accent: '#8a6a2f', gait: 'quad', coat: 'fur' },
		rabbit: { scale: 0.7, body: '#eceae3', accent: '#d2ccbe', gait: 'hop', coat: 'soft' },
		kangaroo: { scale: 1.2, body: '#b07a4a', accent: '#7d5430', gait: 'bipedHop', coat: 'soft' },
		dinosaur: { scale: 2.6, body: '#5f7d4a', accent: '#3c5232', gait: 'bipedWalk', coat: 'scale' }
	};

	// `obj` set for animals placed via prompt/palette; omitted for the always-present companion cat, which
	// follows the player (its home leash tracks you) and isn't scared of you (see `companion`).
	let { world, obj, species = 'cat', companion = false }: { world: World; obj?: WorldObject; species?: string; companion?: boolean } = $props();
	const S = untrack(() => SPECIES[species] ?? SPECIES.cat);
	const isHerb = untrack(() => species === 'rabbit' || species === 'kangaroo'); // grazers: nibble grass when idle
	// predators get the warm/bright eyeshine (a threat watching from the dark); prey the cool pale glint. A
	// predator CHARGING you swaps to a hot-red glare — set per-agent in the hot loop (the shared eyeshine mats
	// can't vary per-instance, so we switch WHICH material the eyes use). $state so the swap re-renders the eyes.
	const isPredator = untrack(() => species === 'cat' || species === 'lion' || species === 'dinosaur');
	let eyeMat = $state(untrack(() => (isPredator ? EYE_PRED_MAT : EYE_PREY_MAT)));
	// EFFECTIVE size = species scale × the requested obj.scale (averaged → uniform), so "a giant dinosaur" /
	// "tiny cats" actually render (and collide + impostor) at that size. Was fixed to the species default,
	// silently ignoring obj.scale on animals while buildings/props honour it.
	const objScale = untrack(() => {
		const s = obj?.scale;
		return s ? (s[0] + s[1] + s[2]) / 3 : 1;
	});
	const SC = untrack(() => (SPECIES[species] ?? SPECIES.cat).scale * objScale);
	// patterned coat materials (cached per colour+pattern in sharedAssets, so every cat shares one program).
	// REACTIVE on obj.color so the LLM `paint` op ("make the cat blue") actually recolours the animal — both
	// the body and its accent take the painted tone; the coat pattern keeps the texture. (Was fixed → paint
	// silently did nothing on animals while it worked on buildings/trees.)
	// per-individual brightness so a scattered herd reads as INDIVIDUALS, not colour-clones (the same idea as
	// the NPC palette / canopy variety; creatures already vary in speed, now in tone). A few QUANTISED shades
	// → bounded cached materials. Stable by id (survives reloads); explicit paint overrides it.
	const SHADE = [0.78, 0.89, 1.0, 1.12, 1.22];
	const idHash = (s: string) => {
		let h = 2166136261;
		for (let i = 0; i < s.length; i++) ((h ^= s.charCodeAt(i)), (h = Math.imul(h, 16777619)));
		return h >>> 0;
	};
	const tintHex = (hex: string, mul: number): string => {
		const n = parseInt(hex.slice(1), 16);
		const r = Math.min(255, Math.round((n >> 16) * mul));
		const g = Math.min(255, Math.round(((n >> 8) & 255) * mul));
		const b = Math.min(255, Math.round((n & 255) * mul));
		return '#' + ((r << 16) | (g << 8) | b).toString(16).padStart(6, '0');
	};
	const shade = untrack(() => SHADE[idHash(obj?.id ?? '') % SHADE.length]);
	const bodyCol = $derived(obj?.color ?? tintHex(S.body, shade));
	const accentCol = $derived(obj?.color ?? tintHex(S.accent, shade));
	const mBody = $derived(creatureMat(bodyCol, S.coat));
	const mAccent = $derived(creatureMat(accentCol, S.coat));
	const ZZZ_Y = SC * 1.5 + 0.4; // float the sleep "zzz" just above this species' head
	const ZMAT = litMat('#eaf1ff'); // soft, shared material for the zzz glyphs

	const MENU: Behavior[] = ['wander', 'pause', 'lookAround', 'sit', 'groom', 'pounce'];
	// leash + wander scale with SPEED: a fast animal (dinosaur) on a tight 26 m leash overshoots it and gets
	// yanked back every couple of seconds → it just orbits its home. A wider leash + gentler heading-jitter
	// lets it actually ROAM and wander instead of circling in place.
	// stable per-agent seed (from the saved id) → deterministic traits/RNG stream (docs/self-sustaining-world §1.6)
	const seedId = untrack(() => seedFrom(obj?.id ?? ''));
	// SEX — same rule as the Rust sim's is_female(seed) (even seed = female). Females render a touch smaller, and
	// male LIONS get the mane (females don't) → you can read the sexes apart, especially on a breeding pair.
	const female = untrack(() => (seedId & 1) === 0);
	const ms = untrack(() => speedFor(species, seedId));
	const agent = untrack(
		() =>
			new Agent(obj?.pos[0] ?? 5, obj?.pos[2] ?? 5, {
				maxSpeed: ms,
				homeRadius: companion ? 14 : 18 + ms * 9, // companion stays close (leash tracks you below)
				wanderRate: 1.7,
				wanderlust: companion ? 0 : 0.3 // a companion never strikes off as a far-roaming explorer
			})
	);
	const managed: ManagedAgent = untrack(() => {
		const m = makeManaged(agent, species, 0.35 * SC, MENU, obj?.id, seedId);
		if (companion) m.companion = true; // manager won't scare it off, and it can't be your own pet's prey-panic
		if (obj?.dead) m.dead = true; // restore live state from a shared link (a corpse stays a corpse)
		else if (obj?.asleep) ((m.asleep = true), (m.sleepTimer = 6));
		if (obj?.juvenile) m.juvenile = true; // a Rust-bred newborn → rustSim stamps a maturation breed-cooldown
		if (obj?.gene != null) m.gene = obj.gene; // inherited vigor → rustSim scales its speed at spawn (genetics)
		return m;
	});
	$effect(() => {
		agentManager.register(managed);
		return () => agentManager.unregister(managed);
	});
	// keep the far-LOD impostor's tint in sync with the body colour (so a painted animal matches at distance)
	$effect(() => {
		managed.tint = bodyCol; // includes the per-individual shade → distant impostors vary too
	});
	let lastShadow = true;

	let group = $state<THREE.Group>();
	let core = $state<THREE.Group>();
	let head = $state<THREE.Group>();
	let tail = $state<THREE.Group>();
	let legFR = $state<THREE.Group>(); // quad: front-right · biped: right arm
	let legFL = $state<THREE.Group>(); // quad: front-left  · biped: left arm
	let legBR = $state<THREE.Group>(); // quad: back-right  · biped: right hind leg
	let legBL = $state<THREE.Group>(); // quad: back-left   · biped: left hind leg
	let sleeping = $state(false); // reactive mirror of managed.asleep → drives the zzz billboard
	let zzz = $state<THREE.Group>(); // the rising sleep glyphs (bobs while asleep)

	const lean = new Spring(0, 10, 0.7);
	const pitch = new Spring(0, 9, 0.8);
	const bodyY = new Spring(0, 12, 0.9);
	const headYaw = new Spring(0, 7, 0.8);
	const headPitch = new Spring(0, 7, 0.85);
	const tailYaw = new Spring(0, 6, 0.6);
	const flop = new Spring(0, 8, 0.55); // tip onto its side when killed

	let phase = 0;
	let t = 0;
	// JUVENILE GROWTH: a Rust-bred newborn is born small and visibly GROWS to adult size over its maturation
	// window (≈ the breed-cooldown), so babies read as babies and you watch them grow up. Render-only — the
	// collision radius + far impostor stay adult-sized (a baby colliding/impostoring as full-size is unnoticeable).
	const BABY_SCALE = 0.45; // born at 45% of adult size
	const GROW_SECS = 34; // ~tracks JUVENILE_CD → matures into a full-size breeder
	const GROW_RATE = (1 - BABY_SCALE) / GROW_SECS;
	let growth = untrack(() => (obj?.juvenile ? BABY_SCALE : 1)); // current size fraction (1 = adult)
	// MESH-LOD: a far+alive agent is drawn by the instanced impostor (AgentImpostors), so it doesn't need its
	// articulated ~15-node mesh hierarchy at all — we shed it (the `{#if showMesh}` below) to keep the scene
	// graph small at herd scale (1000 agents). Spawned-far agents start mesh-LESS so a big scatter never builds
	// 1000 bodies up front (the mount-storm hang). Corpses + the companion always keep their mesh.
	const MESH_GRACE = 1.2; // seconds an agent stays meshed after going far → no thrash at the LOD2 boundary
	const spawnDist = untrack(() => Math.hypot((obj?.pos[0] ?? 5) - playerState.pos[0], (obj?.pos[2] ?? 5) - playerState.pos[2]));
	let showMesh = $state(untrack(() => companion || spawnDist < LOD2_DIST));
	let farTime = 0;

	useTask((dt) => {
		t += dt;
		if (growth < 1) growth = Math.min(1, growth + GROW_RATE * dt); // a juvenile matures toward adult size
		const eSC = SC * growth * (female ? 0.9 : 1); // effective render scale (females a touch smaller — a sex cue)
		if (sleeping !== managed.asleep) sleeping = managed.asleep; // toggle the zzz billboard
		// companion pet → its wander-leash centre tracks you, so it trails along and never strays far (set
		// before any early-out so it keeps following even if it briefly falls behind)
		if (companion) agent.setHome(playerState.pos[0], playerState.pos[2]);

		// FAR → the impostor draws it (alive OR a corpse, tipped on its side): hide the body and, after a short
		// grace, SHED the articulated mesh entirely. Impostoring far corpses too bounds the scene graph when a
		// 1000-agent food chain piles up corpses (each used to keep a full mesh forever → unbounded growth).
		if (managed.lod === 2) {
			if (group) group.visible = false;
			if (showMesh) {
				farTime += dt;
				if (farTime > MESH_GRACE) showMesh = false; // unmount the articulated body
			}
			return;
		}
		farTime = 0;
		if (!showMesh) showMesh = true; // came near → remount the body (refs bind next frame)
		if (!group || !core) return; // mesh still mounting this frame
		group.visible = true;

		// a predator locked onto YOU glares red (only writes the $state on the rare flip → no churn)
		const wantEye = managed.hunting ? EYE_HUNT_MAT : isPredator ? EYE_PRED_MAT : EYE_PREY_MAT;
		if (eyeMat !== wantEye) eyeMat = wantEye;

		agent.interpolate(clock.alpha); // smooth the fixed-rate (30 Hz) sim across render frames
		const gy = heightAt(agent.rx, agent.rz, world.terrain);
		group.position.set(agent.rx, gy, agent.rz);
		group.rotation.y = agent.rh;

		if (managed.castShadow !== lastShadow) {
			lastShadow = managed.castShadow;
			group.traverse((o) => {
				const m = o as THREE.Mesh;
				if (m.isMesh) m.castShadow = managed.castShadow;
			});
		}

		// DEAD → a corpse on its side, frozen where it fell (impostor skips corpses → always draw it). In its
		// final seconds it SINKS into the earth and shrinks, so the reaper (Scene) removing it reads as decay,
		// not a pop. sink 0→1 over CORPSE_SINK_SECS; buried proportional to body size so big corpses fully vanish.
		if (managed.dead) {
			group.visible = true;
			const sink = Math.max(0, (managed.corpseAge - (CORPSE_DECAY_SECS - CORPSE_SINK_SECS)) / CORPSE_SINK_SECS);
			core.rotation.z = flop.step(dt, Math.PI / 2);
			core.rotation.x = 0;
			core.position.y = bodyY.step(dt, -0.05) - sink * (1.0 + SC);
			const ds = eSC * (1 - 0.35 * sink);
			core.scale.set(ds, ds, ds);
			return;
		}

		// (far living agents already returned above; corpses fell through the dead branch)

		// ASLEEP → exhausted hunter resting: body sinks, legs tuck, head lowers, slow breathing
		if (managed.asleep) {
			core.position.y = bodyY.step(dt, -0.15) + Math.sin(t * 1.6) * 0.02;
			core.rotation.z = lean.step(dt, 0);
			core.rotation.x = pitch.step(dt, -0.3);
			core.scale.set(eSC, eSC, eSC);
			const tuck = -0.5;
			if (legFR) legFR.rotation.x = tuck;
			if (legFL) legFL.rotation.x = tuck;
			if (legBR) legBR.rotation.x = tuck;
			if (legBL) legBL.rotation.x = tuck;
			if (head) {
				head.rotation.y = headYaw.step(dt, 0);
				head.rotation.x = headPitch.step(dt, -0.5);
			}
			if (zzz) zzz.position.y = Math.sin(t * 1.5) * 0.12; // gentle drift of the floating zzz
			return;
		}

		// --- locomotion: leg drive depends on the gait MODE ---
		const gait = agent.gaitRate();
		if (S.gait === 'hop' || S.gait === 'bipedHop') {
			phase += (1.4 + 6 * gait) * dt;
			const tuck = -Math.abs(Math.sin(phase)) * 0.9 * gait;
			if (S.gait === 'bipedHop') {
				// kangaroo: hind legs bound together; small arms held up, barely moving
				if (legBR) legBR.rotation.x = tuck;
				if (legBL) legBL.rotation.x = tuck;
				if (legFR) legFR.rotation.x = -0.35 + tuck * 0.15;
				if (legFL) legFL.rotation.x = -0.35 + tuck * 0.15;
			} else {
				// rabbit: all four tuck on the bound
				if (legFR) legFR.rotation.x = tuck * 0.7;
				if (legFL) legFL.rotation.x = tuck * 0.7;
				if (legBR) legBR.rotation.x = tuck;
				if (legBL) legBL.rotation.x = tuck;
			}
		} else if (S.gait === 'bipedWalk') {
			// dinosaur: hind legs stride alternately; tiny arms sway gently opposite
			phase += (1.6 + 8 * gait) * dt;
			const swing = Math.sin(phase) * 0.55 * gait;
			if (legBR) legBR.rotation.x = swing;
			if (legBL) legBL.rotation.x = -swing;
			if (legFR) legFR.rotation.x = -swing * 0.3 - 0.2;
			if (legFL) legFL.rotation.x = swing * 0.3 - 0.2;
		} else {
			// cat / lion: diagonal-pair trot
			phase += (2 + 9 * gait) * dt;
			const swing = Math.sin(phase) * 0.6 * gait;
			if (legFR) legFR.rotation.x = swing;
			if (legBL) legBL.rotation.x = swing;
			if (legFL) legFL.rotation.x = -swing;
			if (legBR) legBR.rotation.x = -swing;
		}

		// --- behaviour poses ---
		let leanT = -agent.turnRate * 0.12;
		let pitchT = 0;
		let bodyYT = 0;
		let headYawT = 0;
		let headPitchT = 0;
		let hop = 0;
		let stretch = 1;
		switch (agent.behavior) {
			case 'sit':
				bodyYT = -0.13;
				pitchT = -0.22;
				break;
			case 'groom':
				bodyYT = -0.05;
				headPitchT = -0.95 + Math.sin(t * 9) * 0.14;
				headYawT = Math.sin(t * 4) * 0.12;
				break;
			case 'lookAround':
				headYawT = Math.sin(agent.progress * Math.PI * 2) * 0.7;
				headPitchT = 0.12;
				break;
			case 'pounce': {
				const p = agent.progress;
				hop = Math.sin(Math.PI * p) * 0.5;
				stretch = 1 + Math.sin(Math.PI * p) * 0.14;
				pitchT = -0.25 * Math.cos(Math.PI * p);
				break;
			}
			default: {
				// DRINKING: a calm animal standing just outside a pond turns to the water and dips its head to
				// lap (all species drink). Animals treat water as an obstacle so they naturally stop at the bank;
				// this catches them there. Takes priority over grazing (water > grass); the player-gaze override
				// below still wins, so they look UP from drinking to watch you pass.
				let drinking = false;
				if (gait < 0.22) {
					for (const z of world.zones ?? []) {
						if (z.material !== 'water') continue;
						const dwx = z.pos[0] - agent.x;
						const dwz = z.pos[2] - agent.z;
						const dw = Math.hypot(dwx, dwz);
						if (dw > z.size && dw < z.size + 2.6) {
							const rel = Math.atan2(dwx, dwz) - agent.heading; // face the water
							headYawT = Math.max(-1.2, Math.min(1.2, Math.atan2(Math.sin(rel), Math.cos(rel))));
							headPitchT = -1.05 + Math.sin(t * 6.0) * 0.1; // head down, gentle lapping
							bodyYT = -0.05;
							drinking = true;
							break;
						}
					}
				}
				if (!drinking && isHerb && gait < 0.22 && world.ground === 'grass') {
					// idle grazer: head dipped to the grass with a gentle nibble (only while nearly stationary)
					headPitchT = -0.8 + Math.sin(t * 5.5) * 0.12;
					bodyYT = -0.04;
				} else if (!drinking) {
					bodyYT = Math.sin(t * 1.6) * 0.012;
				}
			}
		}

		// WATCH THE PLAYER: a calm animal lifts its head to track you as you walk by (wary prey / curious pet /
		// menacing predator). Only while not busy moving (gait < 0.3) so the head still leads a flee/charge —
		// overrides the idle/graze head pose, easing via the existing spring. Wild animals reacting to you = alive.
		const dxp = playerState.pos[0] - agent.x;
		const dzp = playerState.pos[2] - agent.z;
		const pd2 = dxp * dxp + dzp * dzp;
		if (gait < 0.3 && pd2 < 64 && pd2 > 0.25) {
			const rel = Math.atan2(dxp, dzp) - agent.heading; // world bearing to you, minus the body's facing
			headYawT = Math.max(-1.3, Math.min(1.3, Math.atan2(Math.sin(rel), Math.cos(rel)))); // clamp to neck range
			headPitchT = 0.0; // level alert gaze (lifts a grazing head up off the grass to look at you)
		}

		const isBound = S.gait === 'hop' || S.gait === 'bipedHop';
		if (!isBound) leanT += Math.sin(phase) * 0.04 * gait; // trot weight-shift (not for bounders)
		const bob = Math.abs(Math.sin(phase)) * (isBound ? 0.5 * eSC : 0.06) * gait;

		core.position.y = bodyY.step(dt, bodyYT) + bob + hop;
		core.rotation.z = lean.step(dt, leanT);
		core.rotation.x = pitch.step(dt, pitchT);
		core.scale.set(eSC, eSC * stretch, eSC / Math.sqrt(stretch));

		if (head) {
			head.rotation.y = headYaw.step(dt, headYawT);
			head.rotation.x = headPitch.step(dt, headPitchT);
		}
		if (tail) {
			tail.rotation.y = tailYaw.step(dt, -agent.turnRate * 0.18) + Math.sin(t * 2.2) * 0.18 * (1 - gait);
		}
	});
</script>

<!-- the articulated body is mounted only when NEAR (showMesh); a far+alive agent draws via the instanced
     impostor instead, so the scene graph stays small at herd scale. -->
{#if showMesh}
	<T.Group bind:ref={group} userData={{ objectId: obj?.id }}>
		<T.Group bind:ref={core}>
			{#if species === 'rabbit'}
				{@render rabbitBody()}
			{:else if species === 'kangaroo'}
				{@render kangarooBody()}
			{:else if species === 'lion'}
				{@render lionBody()}
			{:else if species === 'dinosaur'}
				{@render dinoBody()}
			{:else}
				{@render catBody()}
			{/if}
		</T.Group>

		<!-- sleep "zzz" — a camera-facing cluster of glyphs that floats above a resting animal -->
		{#if sleeping}
			<Billboard position={[0, ZZZ_Y, 0]}>
				<T.Group bind:ref={zzz}>
					{@render zChar(0, 0, 0.22)}
					{@render zChar(0.28, 0.34, 0.32)}
					{@render zChar(0.64, 0.8, 0.46)}
				</T.Group>
			</Billboard>
		{/if}
	</T.Group>
{/if}

{#snippet zChar(x: number, y: number, s: number)}
	<T.Group position={[x, y, 0]} scale={s}>
		<T.Mesh geometry={PRIM.box} scale={[0.6, 0.12, 0.08]} position={[0, 0.3, 0]} material={ZMAT} />
		<T.Mesh geometry={PRIM.box} scale={[0.12, 0.78, 0.08]} rotation={[0, 0, -0.73]} material={ZMAT} />
		<T.Mesh geometry={PRIM.box} scale={[0.6, 0.12, 0.08]} position={[0, -0.3, 0]} material={ZMAT} />
	</T.Group>
{/snippet}

<!-- a pair of glossy dark eyes on the head front (child of the head group → they turn with the gaze, so the
	 creature visibly LOOKS at you). dx=spacing, (y,z)=position on the head, s=size. Sized per species below. -->
{#snippet eyes(dx: number, y: number, z: number, s: number)}
	<T.Mesh geometry={PRIM.sphere} scale={[s, s, s]} position={[dx, y, z]} material={eyeMat} />
	<T.Mesh geometry={PRIM.sphere} scale={[s, s, s]} position={[-dx, y, z]} material={eyeMat} />
{/snippet}

{#snippet catBody()}
	<T.Mesh geometry={PRIM.box} scale={[0.42, 0.34, 0.95]} position={[0, 0.32, 0]} material={mBody} castShadow />
	<T.Group bind:ref={head} position={[0, 0.46, 0.55]}>
		<T.Mesh geometry={PRIM.sphere} scale={[0.52, 0.52, 0.52]} material={mBody} castShadow />
		{@render eyes(0.11, 0.06, 0.21, 0.085)}
		<T.Mesh geometry={PRIM.cone} scale={[0.16, 0.22, 0.16]} position={[0.13, 0.22, 0]} material={mAccent} castShadow />
		<T.Mesh geometry={PRIM.cone} scale={[0.16, 0.22, 0.16]} position={[-0.13, 0.22, 0]} material={mAccent} castShadow />
	</T.Group>
	<T.Group bind:ref={tail} position={[0, 0.5, -0.55]} rotation={[0.7, 0, 0]}>
		<T.Mesh geometry={PRIM.cyl} scale={[0.1, 0.55, 0.1]} position={[0, 0.22, 0]} material={mAccent} castShadow />
	</T.Group>
	<T.Group bind:ref={legFR} position={[0.14, 0.21, 0.32]}>
		<T.Mesh geometry={PRIM.box} scale={[0.11, 0.22, 0.11]} position={[0, -0.11, 0]} material={mBody} castShadow />
	</T.Group>
	<T.Group bind:ref={legFL} position={[-0.14, 0.21, 0.32]}>
		<T.Mesh geometry={PRIM.box} scale={[0.11, 0.22, 0.11]} position={[0, -0.11, 0]} material={mBody} castShadow />
	</T.Group>
	<T.Group bind:ref={legBR} position={[0.14, 0.21, -0.3]}>
		<T.Mesh geometry={PRIM.box} scale={[0.11, 0.22, 0.11]} position={[0, -0.11, 0]} material={mBody} castShadow />
	</T.Group>
	<T.Group bind:ref={legBL} position={[-0.14, 0.21, -0.3]}>
		<T.Mesh geometry={PRIM.box} scale={[0.11, 0.22, 0.11]} position={[0, -0.11, 0]} material={mBody} castShadow />
	</T.Group>
{/snippet}

{#snippet lionBody()}
	<T.Mesh geometry={PRIM.box} scale={[0.6, 0.5, 1.3]} position={[0, 0.52, 0]} material={mBody} castShadow />
	<T.Group bind:ref={head} position={[0, 0.66, 0.82]}>
		{#if !female}<!-- only MALE lions grow the mane → reads the sexes apart. Pushed BACK + flatter in z so it
			rings/frames the face from behind instead of swallowing it (it used to bury the eyes). -->
			<T.Mesh geometry={PRIM.sphere} scale={[1.0, 0.95, 0.82]} position={[0, 0.03, -0.22]} material={mAccent} castShadow />
		{/if}
		<T.Mesh geometry={PRIM.sphere} scale={[0.5, 0.5, 0.5]} material={mBody} castShadow />
		{@render eyes(0.12, 0.07, 0.27, 0.085)}<!-- eyes forward of the mane's front so a maned male still has a face -->
		<T.Mesh geometry={PRIM.sphere} scale={[0.26, 0.22, 0.3]} position={[0, -0.04, 0.28]} material={mBody} castShadow /><!-- muzzle: protrudes past the mane -->
		<T.Mesh geometry={PRIM.sphere} scale={[0.08, 0.06, 0.08]} position={[0, -0.05, 0.42]} material={mAccent} castShadow /><!-- nose tip -->
		<T.Mesh geometry={PRIM.cone} scale={[0.13, 0.16, 0.13]} position={[0.18, 0.34, 0]} material={mAccent} castShadow />
		<T.Mesh geometry={PRIM.cone} scale={[0.13, 0.16, 0.13]} position={[-0.18, 0.34, 0]} material={mAccent} castShadow />
	</T.Group>
	<T.Group bind:ref={tail} position={[0, 0.68, -0.78]} rotation={[0.8, 0, 0]}>
		<T.Mesh geometry={PRIM.cyl} scale={[0.07, 0.7, 0.07]} position={[0, 0.3, 0]} material={mBody} castShadow />
		<T.Mesh geometry={PRIM.sphere} scale={[0.18, 0.2, 0.18]} position={[0, 0.62, 0]} material={mAccent} castShadow />
	</T.Group>
	<T.Group bind:ref={legFR} position={[0.22, 0.4, 0.46]}>
		<T.Mesh geometry={PRIM.box} scale={[0.17, 0.42, 0.17]} position={[0, -0.21, 0]} material={mBody} castShadow />
	</T.Group>
	<T.Group bind:ref={legFL} position={[-0.22, 0.4, 0.46]}>
		<T.Mesh geometry={PRIM.box} scale={[0.17, 0.42, 0.17]} position={[0, -0.21, 0]} material={mBody} castShadow />
	</T.Group>
	<T.Group bind:ref={legBR} position={[0.22, 0.4, -0.46]}>
		<T.Mesh geometry={PRIM.box} scale={[0.18, 0.42, 0.18]} position={[0, -0.21, 0]} material={mBody} castShadow />
	</T.Group>
	<T.Group bind:ref={legBL} position={[-0.22, 0.4, -0.46]}>
		<T.Mesh geometry={PRIM.box} scale={[0.18, 0.42, 0.18]} position={[0, -0.21, 0]} material={mBody} castShadow />
	</T.Group>
{/snippet}

{#snippet rabbitBody()}
	<T.Mesh geometry={PRIM.sphere} scale={[0.36, 0.34, 0.5]} position={[0, 0.26, 0]} material={mBody} castShadow />
	<T.Group bind:ref={head} position={[0, 0.42, 0.28]}>
		<T.Mesh geometry={PRIM.sphere} scale={[0.3, 0.3, 0.3]} material={mBody} castShadow />
		{@render eyes(0.085, 0.04, 0.13, 0.06)}
		<T.Mesh geometry={PRIM.box} scale={[0.08, 0.42, 0.04]} position={[0.08, 0.32, 0]} rotation={[0, 0, -0.12]} material={mBody} castShadow />
		<T.Mesh geometry={PRIM.box} scale={[0.08, 0.42, 0.04]} position={[-0.08, 0.32, 0]} rotation={[0, 0, 0.12]} material={mBody} castShadow />
	</T.Group>
	<T.Group bind:ref={tail} position={[0, 0.3, -0.32]}>
		<T.Mesh geometry={PRIM.sphere} scale={[0.16, 0.16, 0.16]} material={creatureMat('#ffffff')} castShadow />
	</T.Group>
	<T.Group bind:ref={legFR} position={[0.1, 0.14, 0.2]}>
		<T.Mesh geometry={PRIM.box} scale={[0.08, 0.14, 0.08]} position={[0, -0.07, 0]} material={mBody} castShadow />
	</T.Group>
	<T.Group bind:ref={legFL} position={[-0.1, 0.14, 0.2]}>
		<T.Mesh geometry={PRIM.box} scale={[0.08, 0.14, 0.08]} position={[0, -0.07, 0]} material={mBody} castShadow />
	</T.Group>
	<T.Group bind:ref={legBR} position={[0.13, 0.16, -0.16]}>
		<T.Mesh geometry={PRIM.box} scale={[0.11, 0.16, 0.24]} position={[0, -0.08, 0]} material={mBody} castShadow />
	</T.Group>
	<T.Group bind:ref={legBL} position={[-0.13, 0.16, -0.16]}>
		<T.Mesh geometry={PRIM.box} scale={[0.11, 0.16, 0.24]} position={[0, -0.08, 0]} material={mBody} castShadow />
	</T.Group>
{/snippet}

{#snippet kangarooBody()}
	<T.Mesh geometry={PRIM.box} scale={[0.36, 0.5, 0.34]} position={[0, 0.95, 0.02]} material={mBody} castShadow />
	<T.Mesh geometry={PRIM.box} scale={[0.42, 0.4, 0.4]} position={[0, 0.62, 0]} material={mBody} castShadow />
	<T.Group bind:ref={head} position={[0, 1.3, 0.08]}>
		<T.Mesh geometry={PRIM.sphere} scale={[0.26, 0.3, 0.34]} material={mBody} castShadow />
		{@render eyes(0.075, 0.05, 0.15, 0.055)}
		<T.Mesh geometry={PRIM.cone} scale={[0.09, 0.26, 0.09]} position={[0.1, 0.26, 0]} material={mAccent} castShadow />
		<T.Mesh geometry={PRIM.cone} scale={[0.09, 0.26, 0.09]} position={[-0.1, 0.26, 0]} material={mAccent} castShadow />
	</T.Group>
	<T.Group bind:ref={tail} position={[0, 0.5, -0.18]} rotation={[-0.9, 0, 0]}>
		<T.Mesh geometry={PRIM.box} scale={[0.18, 0.18, 0.95]} position={[0, 0, 0.42]} material={mAccent} castShadow />
	</T.Group>
	<T.Group bind:ref={legFR} position={[0.2, 1.0, 0.18]}>
		<T.Mesh geometry={PRIM.box} scale={[0.07, 0.3, 0.07]} position={[0, -0.15, 0]} material={mBody} castShadow />
	</T.Group>
	<T.Group bind:ref={legFL} position={[-0.2, 1.0, 0.18]}>
		<T.Mesh geometry={PRIM.box} scale={[0.07, 0.3, 0.07]} position={[0, -0.15, 0]} material={mBody} castShadow />
	</T.Group>
	<T.Group bind:ref={legBR} position={[0.18, 0.5, 0.04]}>
		<T.Mesh geometry={PRIM.box} scale={[0.17, 0.5, 0.36]} position={[0, -0.25, 0.06]} material={mBody} castShadow />
	</T.Group>
	<T.Group bind:ref={legBL} position={[-0.18, 0.5, 0.04]}>
		<T.Mesh geometry={PRIM.box} scale={[0.17, 0.5, 0.36]} position={[0, -0.25, 0.06]} material={mBody} castShadow />
	</T.Group>
{/snippet}

{#snippet dinoBody()}
	<T.Mesh geometry={PRIM.box} scale={[0.62, 0.66, 1.25]} position={[0, 1.0, 0.05]} material={mBody} castShadow />
	<T.Mesh geometry={PRIM.box} scale={[0.5, 0.56, 0.6]} position={[0, 1.05, -0.5]} material={mBody} castShadow />
	<T.Group bind:ref={head} position={[0, 1.22, 0.78]}>
		<T.Mesh geometry={PRIM.box} scale={[0.42, 0.46, 0.5]} material={mBody} castShadow />
		{@render eyes(0.14, 0.12, 0.21, 0.09)}
		<T.Mesh geometry={PRIM.box} scale={[0.34, 0.26, 0.42]} position={[0, -0.08, 0.42]} material={mAccent} castShadow />
		<T.Mesh geometry={PRIM.sphere} scale={[0.08, 0.08, 0.08]} position={[0.17, 0.14, 0.18]} material={creatureMat('#1c1c1c')} castShadow />
		<T.Mesh geometry={PRIM.sphere} scale={[0.08, 0.08, 0.08]} position={[-0.17, 0.14, 0.18]} material={creatureMat('#1c1c1c')} castShadow />
	</T.Group>
	<T.Group bind:ref={tail} position={[0, 0.95, -0.78]} rotation={[0.35, 0, 0]}>
		<T.Mesh geometry={PRIM.box} scale={[0.42, 0.42, 0.9]} position={[0, 0, -0.4]} material={mBody} castShadow />
		<T.Mesh geometry={PRIM.box} scale={[0.22, 0.22, 0.8]} position={[0, -0.05, -1.0]} material={mAccent} castShadow />
	</T.Group>
	<T.Group bind:ref={legFR} position={[0.26, 1.05, 0.55]}>
		<T.Mesh geometry={PRIM.box} scale={[0.08, 0.32, 0.08]} position={[0, -0.16, 0]} material={mAccent} castShadow />
	</T.Group>
	<T.Group bind:ref={legFL} position={[-0.26, 1.05, 0.55]}>
		<T.Mesh geometry={PRIM.box} scale={[0.08, 0.32, 0.08]} position={[0, -0.16, 0]} material={mAccent} castShadow />
	</T.Group>
	<T.Group bind:ref={legBR} position={[0.28, 0.78, -0.05]}>
		<T.Mesh geometry={PRIM.box} scale={[0.26, 0.78, 0.4]} position={[0, -0.39, 0.04]} material={mBody} castShadow />
	</T.Group>
	<T.Group bind:ref={legBL} position={[-0.28, 0.78, -0.05]}>
		<T.Mesh geometry={PRIM.box} scale={[0.26, 0.78, 0.4]} position={[0, -0.39, 0.04]} material={mBody} castShadow />
	</T.Group>
{/snippet}
