<script lang="ts">
	// A roaming person NPC. Placed via prompt/palette as a `person` world-object (its spawn is saved
	// & deterministic); the wandering itself is ambient (not saved). Same steering core as the cat,
	// with bipedal procedural locomotion: contralateral leg/arm swing, body bob, spring lean + head.
	import { untrack } from 'svelte';
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { heightAt } from '$lib/terrain';
	import { Agent, Spring, type Behavior } from '$lib/steering';
	import { agentManager, makeManaged, speedFor, LOD2_DIST, type ManagedAgent } from '$lib/agents.svelte';
	import { seedFrom } from '$lib/rng';
	import { clock } from '$lib/clock';
	import { NPC, PRIM, creatureMat, EYE_MAT } from '$lib/sharedAssets';
	import { playerState } from '$lib/playerState.svelte';
	import type { World, WorldObject } from '$lib/world';

	let { obj, world }: { obj: WorldObject; world: World } = $props();

	// honour obj.scale ("a giant person" / "tiny people") — was ignored, so people rendered at a fixed size
	// while buildings/props (and now animals) scale. Uniform (avg) → matches the Critter convention.
	const objScale = untrack(() => {
		const s = obj.scale;
		return s ? (s[0] + s[1] + s[2]) / 3 : 1;
	});

	const MENU: Behavior[] = ['wander', 'pause', 'lookAround'];
	// seed once from the saved spawn; the agent owns its position thereafter (snapshot is intentional).
	// People used to clump near spawn, jitter back-and-forth at a tight 14 m leash, and never explore (only
	// the default 14% were "explorers"). Now: a WIDE leash (40 m → they roam a town-sized patch, not orbit a
	// post), HIGH wanderlust (0.55 → most strike out and journey 70–200 m across the map, so a crowd disperses
	// and actually explores), and a calmer wander jitter (1.3 → smooth ambling, not nervous shuffling).
	// stable per-agent seed (from the saved id) → deterministic traits/RNG stream (docs/self-sustaining-world §1.6)
	const seedId = untrack(() => seedFrom(obj.id));
	const agent = untrack(
		() => new Agent(obj.pos[0], obj.pos[2], { maxSpeed: speedFor('person', seedId), homeRadius: 40, wanderRate: 1.3, turnSpeed: 5, wanderlust: 0.55 })
	);
	// the shared manager owns stepping + flocking + the food-chain sim; we only read & render
	const managed: ManagedAgent = untrack(() => {
		const m = makeManaged(agent, 'person', 0.4 * objScale, MENU, obj.id, seedId); // radius scales → collision + far impostor
		if (obj.dead) m.dead = true; // restore live state from a shared link
		else if (obj.asleep) ((m.asleep = true), (m.sleepTimer = 6));
		return m;
	});
	$effect(() => {
		agentManager.register(managed);
		return () => agentManager.unregister(managed);
	});
	let lastShadow = true;

	// Deterministic per-person palette from the (saved) object id → a crowd reads as individuals, not a clone
	// army in identical blue. Stable across reloads / share links (the id is saved). Independent bit-slices of
	// one hash pick shirt / pants / skin so they vary together but uncorrelated. Explicit paint overrides the shirt.
	const SHIRTS = ['#4a73c4', '#c4554a', '#4aa86b', '#caa23e', '#7a5bc4', '#3ba0a8', '#c4708f', '#5a6470', '#d0853f', '#8a9a3a'];
	const PANTSES = ['#34507f', '#3a3a42', '#5a4632', '#414655', '#6b5240', '#2f4a3a'];
	const SKINS = ['#f0c9a8', '#e8b894', '#d39c6e', '#c98a5b', '#a8703f', '#8a5a32'];
	const idHash = (s: string) => {
		let h = 2166136261;
		for (let i = 0; i < s.length; i++) ((h ^= s.charCodeAt(i)), (h = Math.imul(h, 16777619)));
		return h >>> 0;
	};
	const H = untrack(() => idHash(obj.id));
	const SHIRT = $derived(obj.color ?? SHIRTS[H % SHIRTS.length]);
	const PANTS = PANTSES[(H >>> 4) % PANTSES.length];
	const SKIN = SKINS[(H >>> 9) % SKINS.length];
	// share the shirt colour with the manager so the FAR impostor (AgentImpostors) tints this person the same
	// → a distant crowd stays varied instead of popping to uniform blue at the LOD boundary. Tracks paint.
	$effect(() => {
		managed.tint = SHIRT;
	});

	let group = $state<THREE.Group>();
	let core = $state<THREE.Group>();
	let head = $state<THREE.Group>();
	let legL = $state<THREE.Group>();
	let legR = $state<THREE.Group>();
	let armL = $state<THREE.Group>();
	let armR = $state<THREE.Group>();

	const lean = new Spring(0, 9, 0.7);
	const headYaw = new Spring(0, 6, 0.85);
	const flop = new Spring(0, 8, 0.55); // collapse flat when killed

	const idlePhase = (H % 628) / 100; // 0–6.28, per-person → standing-idle sway/breath is desynchronised across a crowd
	let phase = 0;
	let t = 0;
	// MESH-LOD (see Critter.svelte): a far+alive person is drawn by the instanced impostor, so shed the
	// articulated body to keep the scene graph small at crowd scale. Spawned-far people start mesh-LESS.
	const MESH_GRACE = 1.2; // seconds meshed after going far → no thrash at the LOD2 boundary
	const spawnDist = untrack(() => Math.hypot(obj.pos[0] - playerState.pos[0], obj.pos[2] - playerState.pos[2]));
	let showMesh = $state(untrack(() => !!obj.dead || spawnDist < LOD2_DIST));
	let farTime = 0;

	useTask((dt) => {
		t += dt;
		// the manager already stepped `agent` this frame — we only read & render it

		// far & ALIVE → the impostor draws it: hide and, after a short grace, SHED the mesh. Corpses are NOT
		// impostored, so the dead branch below still runs (keeps its mesh) at any distance.
		if (managed.lod === 2 && !managed.dead) {
			if (group) group.visible = false;
			if (showMesh) {
				farTime += dt;
				if (farTime > MESH_GRACE) showMesh = false;
			}
			return;
		}
		farTime = 0;
		if (!showMesh) showMesh = true; // came near (or died) → remount (refs bind next frame)
		if (!group || !core) return; // mesh still mounting this frame
		group.visible = true;

		agent.interpolate(clock.alpha); // smooth the fixed-rate (30 Hz) sim across render frames
		const gy = heightAt(agent.rx, agent.rz, world.terrain);
		group.position.set(agent.rx, gy, agent.rz);
		group.rotation.y = agent.rh;

		// DEAD → collapse flat where it fell, frozen (impostor skips corpses, so always draw it)
		if (managed.dead) {
			group.visible = true;
			core.rotation.x = flop.step(dt, Math.PI / 2);
			core.position.y = 0;
			return;
		}

		// shadow budget — only the nearest few cast; re-apply only when it flips
		if (managed.castShadow !== lastShadow) {
			lastShadow = managed.castShadow;
			group.traverse((o) => {
				const m = o as THREE.Mesh;
				if (m.isMesh) m.castShadow = managed.castShadow;
			});
		}

		// (far living agents already returned above; corpses fell through the dead branch)

		// ASLEEP → lie down and rest (people rarely tire — only predators sleep — but handle it safely)
		if (managed.asleep) {
			core.rotation.x = flop.step(dt, Math.PI / 2);
			core.position.y = Math.sin(t * 1.6) * 0.02;
			return;
		}

		// gait — contralateral limbs, cadence scales with speed
		const gait = agent.gaitRate();
		phase += (1.5 + 7 * gait) * dt;
		const amp = 0.5 * gait;
		const swing = Math.sin(phase) * amp;
		if (legL) legL.rotation.x = swing;
		if (legR) legR.rotation.x = -swing;
		if (armL) armL.rotation.x = -swing * 0.85; // arms opposite their same-side leg
		if (armR) armR.rotation.x = swing * 0.85;

		// secondary motion + a gentle STANDING idle so a paused person isn't a frozen mannequin: a slow breathing
		// bob and a weight-shift sway, faded in as they slow (idle≈1 standing → 0 walking) and desynchronised
		// per-person (idlePhase) so a crowd doesn't sway in robotic unison.
		const idle = Math.max(0, 1 - gait * 4);
		const bob = Math.abs(Math.sin(phase)) * 0.05 * gait;
		core.position.y = bob + idle * Math.sin(t * 1.4 + idlePhase) * 0.015;
		core.rotation.z = lean.step(dt, -agent.turnRate * 0.1 + Math.sin(phase) * 0.03 * gait + idle * Math.sin(t * 0.6 + idlePhase) * 0.05);

		// idle head glances when not walking — but if you're nearby and they're not walking, they turn to WATCH
		// you (a town that notices you pass). Same gaze as the animals; the head still leads an actual walk.
		let lookT = agent.behavior === 'lookAround' ? Math.sin(agent.progress * Math.PI * 2) * 0.6 : 0;
		const dxp = playerState.pos[0] - agent.x;
		const dzp = playerState.pos[2] - agent.z;
		const pd2 = dxp * dxp + dzp * dzp;
		if (gait < 0.25 && pd2 < 49 && pd2 > 0.25) {
			const rel = Math.atan2(dxp, dzp) - agent.heading; // bearing to you minus the body's facing
			lookT = Math.max(-1.2, Math.min(1.2, Math.atan2(Math.sin(rel), Math.cos(rel)))); // clamp to neck range
		}
		if (head) head.rotation.y = headYaw.step(dt, lookT);
	});
</script>

<!-- the articulated body mounts only when NEAR (showMesh); a far+alive person draws via the instanced
     impostor instead, so the scene graph stays small at crowd scale. -->
{#if showMesh}
	<T.Group bind:ref={group} userData={{ objectId: obj.id }}>
		<!-- objScale scales the whole avatar from the feet (group origin = ground); world pos is set on `group` -->
		<T.Group bind:ref={core} scale={objScale}>
		<!-- torso · SHARED geometry + cached material -->
		<T.Mesh position={[0, 1.05, 0]} geometry={NPC.torso} material={creatureMat(SHIRT)} castShadow />
		<!-- head pivot -->
		<T.Group bind:ref={head} position={[0, 1.62, 0]}>
			<T.Mesh geometry={NPC.head} material={creatureMat(SKIN)} castShadow />
			<!-- eyes (child of the head → turn with the gaze, so a villager visibly looks at you) -->
			<T.Mesh geometry={PRIM.sphere} scale={[0.07, 0.07, 0.07]} position={[0.09, 0.05, 0.2]} material={EYE_MAT} />
			<T.Mesh geometry={PRIM.sphere} scale={[0.07, 0.07, 0.07]} position={[-0.09, 0.05, 0.2]} material={EYE_MAT} />
		</T.Group>
		<!-- arms (shoulder pivots) -->
		<T.Group bind:ref={armL} position={[0.34, 1.4, 0]}>
			<T.Mesh position={[0, -0.3, 0]} geometry={NPC.arm} material={creatureMat(SHIRT)} castShadow />
		</T.Group>
		<T.Group bind:ref={armR} position={[-0.34, 1.4, 0]}>
			<T.Mesh position={[0, -0.3, 0]} geometry={NPC.arm} material={creatureMat(SHIRT)} castShadow />
		</T.Group>
		<!-- legs (hip pivots; foot reaches the ground) -->
		<T.Group bind:ref={legL} position={[0.14, 0.7, 0]}>
			<T.Mesh position={[0, -0.35, 0]} geometry={NPC.leg} material={creatureMat(PANTS)} castShadow />
		</T.Group>
		<T.Group bind:ref={legR} position={[-0.14, 0.7, 0]}>
			<T.Mesh position={[0, -0.35, 0]} geometry={NPC.leg} material={creatureMat(PANTS)} castShadow />
		</T.Group>
	</T.Group>
</T.Group>
{/if}
