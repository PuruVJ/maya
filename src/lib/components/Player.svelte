<script lang="ts">
	import { T, useTask } from '@threlte/core';
	import { RigidBody, Collider, useRapier } from '@threlte/rapier';
	import { onMount } from 'svelte';
	import * as THREE from 'three';
	import { dlog } from '$lib/debug';
	import { playerState } from '$lib/playerState.svelte';
	import { agentManager, type ManagedAgent } from '$lib/agents.svelte';
	import { heightAt } from '$lib/terrain';
	import { forEachTreeNear, treeRadius, onPath } from '$lib/scatter';
	import { inWater } from '$lib/water';
	import { touchInput } from '$lib/touchControls';
	import { NPC, creatureMat } from '$lib/sharedAssets';
	import { kindDef } from '$lib/kinds';
	import type { World } from '$lib/world';

	// creatures are handled by the animal push-out (agentManager); everything else is a solid placed object
	const CREATURES = new Set(['person', 'cat', 'lion', 'rabbit', 'kangaroo', 'dinosaur']);
	// Camera occlusion is intentionally narrower than player collision. Pulling the camera in for every lamp,
	// rock, bush, or ambient trunk made normal movement feel like a zoom pump. Only substantial buildings
	// should block the view, and even then the camera keeps a comfortable over-the-shoulder distance.
	const CAMERA_BLOCKERS = new Set(['house', 'cabin', 'tower']);

	const ss = (a: number, b: number, x: number) => {
		const t = Math.max(0, Math.min(1, (x - a) / (b - a)));
		return t * t * (3 - 2 * t);
	};

	let { world }: { world: World } = $props();

	const rapierCtx = useRapier();

	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	let body = $state<any>();
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	let collider = $state<any>();
	let cam = $state<THREE.PerspectiveCamera>();
	let visual = $state<THREE.Group>();
	let legL = $state<THREE.Group>();
	let legR = $state<THREE.Group>();
	let armL = $state<THREE.Group>();
	let armR = $state<THREE.Group>();
	let gait = 0;

	const keys = new Set<string>();
	let yaw = 0;
	let pitch = 0.5;
	let vy = 0;
	let jumpHeld = false; // Space held → keep ascending (infinite jump) to a bird's-eye view, up to FLY_CEILING
	let dragging = false;
	let lastPointerX = 0;
	let lastPointerY = 0;
	let sprinting = false; // Shift toggles; only matters while moving
	// HMR / RELOAD RESUME — read the live pose from sessionStorage (survives a Vite module re-eval AND a reload,
	// unlike a module singleton), so a code change no longer teleports you back to spawn. A SHARE link (#w=) is the
	// exception: there the URL's start pose is authoritative, so we ignore the stored pose.
	let _resume: { x: number; y: number; z: number; yaw: number } | null = null;
	if (typeof location === 'undefined' || !/[#&]w=/.test(location.hash)) {
		try {
			const s = sessionStorage.getItem('playerPose');
			if (s) _resume = JSON.parse(s);
		} catch {
			/* unavailable */
		}
	}
	let px = _resume ? _resume.x : 0;
	let py = _resume ? _resume.y : 0.9;
	let pz = _resume ? _resume.z : 6;
	let moved = !!_resume; // resuming → skip the saved-start restore below
	if (_resume) yaw = _resume.yaw;

	// Restore the saved player pose from a shared/reloaded link (world.start, decoded from the URL) the
	// moment it arrives — as long as you haven't moved yet (and aren't resuming a stored live session) — so you
	// reopen standing where you left off.
	$effect(() => {
		const s = world.start;
		if (s && !moved) {
			px = s.x;
			pz = s.z;
			yaw = s.yaw;
			py = heightAt(px, pz, world.terrain) + 1;
			spawned = false; // re-teleport the kinematic body to here on the next frame
		}
	});

	const SPEED = 9; // m/s
	const SPRINT = 18; // m/s — tap Shift to toggle sprint (applies while moving)
	const SENS = 0.005;
	const CAM_DIST = 10;
	const PITCH_MIN = -0.45; // tilt right down → camera swings close & under you to look up at the sky
	const PITCH_MAX = 1.5; // ...up to nearly top-down
	const GRAVITY = 22;
	const JUMP_V = 10;
	const CLIMB_V = 8; // sustained ascent speed while Space is held (infinite jump → bird's-eye)
	const FLY_CEILING = 85; // max altitude (m above the ground beneath you) the hold-to-rise tops out at
	const CAPSULE_HALF = 0.9; // capsule centre above the feet
	const MAX_WALK_SLOPE = 1.3; // max climbable terrain gradient (rise/run ≈ tan 52°); steeper faces block the uphill move
	const WADE = 0.45; // movement multiplier while wading through water (you can enter; it slows you)
	const SINK = 0.55; // how far the avatar visually sinks when wading
	let sink = 0; // lerped sink offset for smooth in/out
	let camHoriz = CAM_DIST; // smoothed camera-collision distance → eases in/out of obstacles (no snap-jitter)
	// predator strike — when a big hunter (lion/dino) reaches you it knocks you back, briefly stuns
	// (slowed control) and shakes the camera. Non-lethal; the chase has stakes without a death system.
	const KB = 16; // initial knockback speed (m/s, decays)
	const STUN = 0.8; // seconds of slowed control after a hit (also the re-hit cooldown)
	let stunT = 0;
	let kbX = 0;
	let kbZ = 0;
	let shakeT = 0;

	// is (x,z) inside any water zone? — blob-shaped, matching what's drawn (see water.ts). You wade in
	// (slowed, sinking); animals avoid ponds entirely.
	const isWater = (x: number, z: number): boolean => inWater(world.zones, x, z);
	// WADING only counts when you're actually down IN the water — feet near the (submerged) bed — not when a
	// BRIDGE/dock spans the pond or you're mid-JUMP over it. heightAt at a lake is the bed; without this the
	// XZ-only test slowed + sank + rippled you while walking a bridge above the water.
	const submerged = (x: number, z: number, y: number): boolean =>
		isWater(x, z) && y - CAPSULE_HALF <= heightAt(x, z, world.terrain) + 0.3;

	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	let controller: any = null;
	let spawned = false;

	function tryJump() {
		vy = Math.max(vy, JUMP_V); // an initial hop; holding Space then SUSTAINS the climb (see the physics tick)
	}

	onMount(() => {
		const isTyping = () =>
			document.activeElement instanceof HTMLInputElement ||
			document.activeElement instanceof HTMLTextAreaElement;
		const down = (e: KeyboardEvent) => {
			if (isTyping()) return;
			keys.add(e.key.toLowerCase());
			if (e.key === 'Shift' && !e.repeat) sprinting = !sprinting; // tap to toggle sprint
			if (e.code === 'Space') {
				e.preventDefault();
				jumpHeld = true; // hold → keep rising to the ceiling (infinite jump)
				if (!e.repeat) tryJump();
			}
		};
		const up = (e: KeyboardEvent) => {
			keys.delete(e.key.toLowerCase());
			if (e.code === 'Space') jumpHeld = false; // release → gravity takes over, you descend
		};
		// look = drag on the canvas. Track the last pointer position and use the delta (works for BOTH mouse
		// and touch — movementX/Y is unreliable for touch pointers). The on-screen joystick/jump live in
		// their own corners (non-canvas targets), so they don't rotate the view.
		const pdown = (e: PointerEvent) => {
			if ((e.target as HTMLElement)?.tagName === 'CANVAS') {
				dragging = true;
				lastPointerX = e.clientX;
				lastPointerY = e.clientY;
			}
		};
		const pup = () => (dragging = false);
		const pmove = (e: PointerEvent) => {
			if (!dragging) return;
			yaw -= (e.clientX - lastPointerX) * SENS;
			pitch = Math.min(PITCH_MAX, Math.max(PITCH_MIN, pitch - (e.clientY - lastPointerY) * SENS));
			lastPointerX = e.clientX;
			lastPointerY = e.clientY;
		};
		window.addEventListener('keydown', down);
		window.addEventListener('keyup', up);
		window.addEventListener('pointerdown', pdown);
		window.addEventListener('pointerup', pup);
		window.addEventListener('pointermove', pmove);
		dlog('player', 'controller mounted (rapier KCC)');
		return () => {
			window.removeEventListener('keydown', down);
			window.removeEventListener('keyup', up);
			window.removeEventListener('pointerdown', pdown);
			window.removeEventListener('pointerup', pup);
			window.removeEventListener('pointermove', pmove);
			if (controller) rapierCtx.world.removeCharacterController(controller);
		};
	});

	useTask((delta) => {
		// cap the step so a one-off frame hitch (e.g. a grass/terrain rebuild) can't make the kinematic
		// body lurch a big distance in a single integration → no periodic "glitch" while walking
		delta = Math.min(delta, 1 / 20);
		const phys = rapierCtx.world;
		if (!body || !collider || !cam || !phys) return;

		if (!controller) {
			controller = phys.createCharacterController(0.01);
			controller.enableAutostep(0.6, 0.3, true); // maxHeight, minWidth, climb dynamic
			controller.enableSnapToGround(0.5);
			controller.setApplyImpulsesToDynamicBodies(true);
			controller.setMaxSlopeClimbAngle((50 * Math.PI) / 180);
		}

		// "go home" (or any) teleport request → drop the avatar at the target and re-fire the spawn-teleport
		// below (reuses the same kinematic-body move; one-shot, cleared immediately).
		if (playerState.teleportTo) {
			px = playerState.teleportTo[0];
			pz = playerState.teleportTo[2];
			py = heightAt(px, pz, world.terrain) + 1;
			vy = 0;
			spawned = false;
			playerState.teleportTo = null;
		}

		// teleport the kinematic body to the spawn point on the first frame, then integrate
		if (!spawned) {
			body.setNextKinematicTranslation({ x: px, y: py, z: pz });
			spawned = true;
			return;
		}

		// horizontal input relative to facing
		const fx = -Math.sin(yaw);
		const fz = -Math.cos(yaw);
		const rx = -fz;
		const rz = fx;
		// keyboard + on-screen joystick (analog) combined
		if (touchInput.jump) {
			tryJump();
			touchInput.jump = false;
		}
		const fwd = (keys.has('w') ? 1 : 0) - (keys.has('s') ? 1 : 0) + touchInput.moveZ;
		const strafe = (keys.has('d') ? 1 : 0) - (keys.has('a') ? 1 : 0) + touchInput.moveX;
		let mx = fx * fwd + rx * strafe;
		let mz = fz * fwd + rz * strafe;
		const len = Math.hypot(mx, mz);
		if (len > 0) moved = true; // you've taken control → an incoming saved start pos no longer overrides you
		if (len > 1) {
			mx /= len; // clamp keyboard diagonals to unit; preserve sub-unit joystick tilt (analog speed)
			mz /= len;
		}

		vy -= GRAVITY * delta;
		// INFINITE JUMP → bird's-eye: while Space is held, keep ascending at CLIMB_V until you reach FLY_CEILING
		// metres above the ground beneath you (the "ceiling"), then hold there; release and gravity brings you down.
		const altAbove = py - (heightAt(px, pz, world.terrain) + CAPSULE_HALF);
		if (jumpHeld && altAbove < FLY_CEILING) vy = Math.max(vy, CLIMB_V);
		else if (altAbove >= FLY_CEILING && vy > 0) vy = 0; // bonk the ceiling → stop rising

		// wading through a pond slows you right down (you can still enter — animals can't)
		const wading = submerged(px, pz, py);
		const moveSpeed = (sprinting ? SPRINT : SPEED) * (wading ? WADE : 1) * (stunT > 0 ? 0.45 : 1);

		// let Rapier resolve the move against all colliders (collide-and-slide, auto-step, snap)
		controller.computeColliderMovement(collider, {
			x: mx * moveSpeed * delta,
			y: vy * delta,
			z: mz * moveSpeed * delta
		});
		const mv = controller.computedMovement();
		const t = body.translation();
		px = t.x + mv.x;
		py = t.y + mv.y;
		pz = t.z + mv.z;

		// SLOPE-LIMITED WALKING — the terrain has no collider (we snap to heightAt below), so without this you
		// could stroll straight up a near-vertical mountain face. Sample the terrain gradient at the destination
		// and, only when you're walking ON the ground (not mid-jump → feet at/below the surface), strip the
		// too-steep UPHILL part of this frame's move so you slide along the base of a cliff instead of climbing
		// it. Only steep far-out mountains trip it (terrain is flat near spawn); descending and traversing stay
		// free and downhill/along-slope moves are always allowed, so you can never get trapped against a face.
		const gH = heightAt(px, pz, world.terrain);
		if (py - CAPSULE_HALF <= gH + 0.1) {
			const e = 0.4;
			const gx = (heightAt(px + e, pz, world.terrain) - gH) / e;
			const gz = (heightAt(px, pz + e, world.terrain) - gH) / e;
			const gmag = Math.hypot(gx, gz);
			if (gmag > MAX_WALK_SLOPE) {
				const ux = gx / gmag;
				const uz = gz / gmag;
				const into = mv.x * ux + mv.z * uz; // how much of the move heads uphill
				if (into > 0) {
					px -= ux * into;
					pz -= uz * into;
				}
			}
		}

		// analytic terrain floor — the ground has no collider; heightAt is the surface.
		// (Rapier still resolves objects above, so jump-on-things keeps working.)
		const groundY = heightAt(px, pz, world.terrain) + CAPSULE_HALF;
		let grounded = controller.computedGrounded();
		if (py <= groundY) {
			py = groundY;
			if (vy < 0) vy = 0;
			grounded = true;
		}

		// predator-strike knockback + stun decay (the hit is detected in the animal loop below)
		if (stunT > 0) {
			stunT -= delta;
			px += kbX * delta;
			pz += kbZ * delta;
			const decay = Math.max(0, 1 - 7 * delta);
			kbX *= decay;
			kbZ *= decay;
		}

		// Resolve overlaps with living animals. Prey/small animals YIELD — they get nudged aside and you
		// barely move, so a lone frolicker or fleer never shoves you (they also steer around you via the
		// manager's personal-space avoidance). Only crowd pressure (a STAMPEDE) or a big predator moves YOU:
		// `share` ramps 0→1 as more animals press at once, and is always 1 for an apex predator.
		const PR = 0.5; // player body radius
		const overlaps: { m: ManagedAgent; nx: number; nz: number; pen: number }[] = [];
		agentManager.forEach((m) => {
			if (m.dead) return;
			const dx = px - m.agent.x;
			const dz = pz - m.agent.z;
			const d2 = dx * dx + dz * dz;
			const r = PR + m.radius;
			if (d2 < r * r && d2 > 1e-6) {
				const d = Math.sqrt(d2);
				overlaps.push({ m, nx: dx / d, nz: dz / d, pen: r - d });
				// a big predator (lion/dino) reaching you STRIKES — shove + stun + shake (non-lethal).
				// Gated by stunT so it can't re-hit until you've reeled (a cooldown).
				if (stunT <= 0 && m.rank >= 4) {
					stunT = STUN;
					kbX = (dx / d) * KB;
					kbZ = (dz / d) * KB;
					shakeT = 0.45;
				}
			}
		});
		// crowd: 1 animal → 0 (it yields entirely), 2 → 0.5, 3+ → 1 (a herd has too much mass to push through)
		const crowd = Math.min(1, Math.max(0, overlaps.length - 1) / 2);
		for (const o of overlaps) {
			const share = o.m.rank >= 4 ? 1 : crowd; // apex predators always shove you; prey only en masse
			px += o.nx * o.pen * share;
			pz += o.nz * o.pen * share;
			o.m.agent.x -= o.nx * o.pen * (1 - share); // the rest is taken out of the animal (it steps aside)
			o.m.agent.z -= o.nz * o.pen * (1 - share);
		}

		// solid ambient-forest trees — push out of any trunk you'd walk into (deterministic placement, so
		// this matches exactly what AmbientScatter draws). PR + max trunk radius ≈ 1.4 m → search reach 1.5.
		forEachTreeNear(px, pz, 1.5, (tr) => {
			if (inWater(world.zones, tr.x, tr.z) || onPath(world.paths, tr.x, tr.z)) return; // AmbientScatter culls trees in lakes / on roads → don't collide with the ghost
			const dx = px - tr.x;
			const dz = pz - tr.z;
			const r = PR + treeRadius(tr.scale);
			const d2 = dx * dx + dz * dz;
			if (d2 < r * r && d2 > 1e-6) {
				const d = Math.sqrt(d2);
				px += (dx / d) * (r - d);
				pz += (dz / d) * (r - d);
			}
		});

		// solid PLACED objects — buildings, props, lamps, placed trees: push out of their footprint so you
		// can't walk through a house. Creatures are skipped (the animal push-out above handles them). XZ-only.
		// Box-footprint kinds (houses/cabins) use an ORIENTED BOX so you can walk right up to a wall and follow
		// streets instead of bumping an oversized circle; round kinds (towers/rocks/wells/lamps) stay circles.
		for (const o of world.objects) {
			if (CREATURES.has(o.kind)) continue;
			const def = kindDef(o.kind);
			const sx = o.scale?.[0] ?? 1;
			const sz = o.scale?.[2] ?? 1;
			// JUMP-ON-THINGS: if your feet have cleared the object's top you're standing ON it (a prop's Rapier
			// collider holds you up there), so DON'T shove you off — the XZ push-out is only meant to stop you
			// walking THROUGH it at ground level. (Can't open a walk-through gap: at ground level your feet sit at
			// the base, well below the top, so this only fires for objects under ~0.15 m tall.)
			if (py - CAPSULE_HALF >= o.pos[1] + def.h * (o.scale?.[1] ?? 1) - 0.15) continue;
			const dx = px - o.pos[0];
			const dz = pz - o.pos[2];
			const wall = def.parts[0];
			if (wall && wall.geo === 'box') {
				// rotate the player into the building's local frame (rotation.y = obj.rot), clamp to the box
				const th = ((o.rot ?? 0) * Math.PI) / 180;
				const cs = Math.cos(th);
				const sn = Math.sin(th);
				const lx = dx * cs - dz * sn; // world → local
				const lz = dx * sn + dz * cs;
				const hx = (wall.args[0] / 2) * sx + PR;
				const hz = (wall.args[2] / 2) * sz + PR;
				if (Math.abs(lx) < hx && Math.abs(lz) < hz) {
					// inside the inflated box → eject along the axis of least penetration, then rotate back
					const nlx = hx - Math.abs(lx) < hz - Math.abs(lz) ? (lx >= 0 ? hx : -hx) : lx;
					const nlz = nlx === lx ? (lz >= 0 ? hz : -hz) : lz;
					px = o.pos[0] + (nlx * cs + nlz * sn); // local → world
					pz = o.pos[2] + (-nlx * sn + nlz * cs);
				}
			} else {
				const d2 = dx * dx + dz * dz;
				const r = PR + def.r * Math.max(sx, sz);
				if (d2 < r * r && d2 > 1e-6) {
					const d = Math.sqrt(d2);
					px += (dx / d) * (r - d);
					pz += (dz / d) * (r - d);
				}
			}
		}

		body.setNextKinematicTranslation({ x: px, y: py, z: pz });

		// wading: flag it (the water shader rings ripples around you) and sink the avatar a little
		playerState.inWater = submerged(px, pz, py);
		sink += ((playerState.inWater ? SINK : 0) - sink) * Math.min(1, 8 * delta);

		if (visual) {
			visual.rotation.y = yaw;
			visual.position.y = -sink;
		}

		// walk cycle on the humanoid limbs (legs/arms swing contralaterally while moving)
		gait += (len > 0 ? 9 : 0) * delta;
		const sw = Math.sin(gait) * (len > 0 ? 0.6 : 0);
		if (legL) legL.rotation.x = sw;
		if (legR) legR.rotation.x = -sw;
		if (armL) armL.rotation.x = -sw * 0.7;
		if (armR) armR.rotation.x = sw * 0.7;

		// as you tilt down (low pitch) the camera swings CLOSE & under you → look up at the sky
		const dist = CAM_DIST * (0.28 + 0.72 * ss(-0.15, 0.7, pitch));
		let horiz = dist * Math.cos(pitch);
		let camY = py + dist * Math.sin(pitch);

		// CAMERA COLLISION — only substantial buildings occlude the camera. Small props and ambient trunks are
		// deliberately ignored: letting every bush/rock/tree collapse the orbit camera caused constant zooming
		// in normal play. Pull in promptly for a real wall, but never all the way onto the character.
		const cdx = Math.sin(yaw);
		const cdz = Math.cos(yaw);
		for (const o of world.objects) {
			if (!CAMERA_BLOCKERS.has(o.kind)) continue;
			const ox = o.pos[0] - px;
			const oz = o.pos[2] - pz;
			const proj = ox * cdx + oz * cdz; // distance along the ray to the object's closest point
			if (proj <= 0 || proj > horiz) continue; // behind you, or further than the camera already is
			const rr = kindDef(o.kind).r * Math.max(o.scale?.[0] ?? 1, o.scale?.[2] ?? 1) + 0.4;
			const perp2 = (ox - cdx * proj) ** 2 + (oz - cdz * proj) ** 2;
			if (perp2 >= rr * rr) continue; // the ray misses this footprint
			horiz = Math.max(5, proj - Math.sqrt(rr * rr - perp2)); // keep a stable shoulder-camera distance
		}
		// SMOOTH the collision distance so the camera EASES in/out of obstacles instead of snapping to/from "right
		// behind you" every time a prop/tree flickers in the ray test — that snap was the erratic camera jitter.
		// Pull in quicker than it eases out (so it still ducks a real wall promptly), but a 1-frame false hit now
		// barely moves it. (Frame-rate-independent exponential smoothing.)
		const camRate = horiz < camHoriz ? 16 : 6;
		camHoriz += (horiz - camHoriz) * Math.min(1, delta * camRate);
		// keep the camera above the ground at ITS OWN xz too (not just yours), so it doesn't sink into a hill
		// rising behind you — clamp to the higher of the two terrain heights (camHoriz is final now).
		const camX = px + cdx * camHoriz;
		const camZ = pz + cdz * camHoriz;
		const floorY = Math.max(heightAt(px, pz, world.terrain), heightAt(camX, camZ, world.terrain)) + 0.5;
		if (camY < floorY) camY = floorY; // never dip below the ground (player's OR the camera's)
		// camera shake on a predator strike (decays over ~0.45 s)
		if (shakeT > 0) shakeT -= delta;
		const shk = shakeT > 0 ? shakeT * 1.3 : 0;
		const jx = shk && (Math.random() - 0.5) * shk;
		const jy = shk && (Math.random() - 0.5) * shk;
		const jz = shk && (Math.random() - 0.5) * shk;
		cam.position.set(camX + jx, camY + jy, camZ + jz);
		cam.lookAt(px, py, pz);

		playerState.place([px, 0, pz], yaw);
		playerState.tick(len > 0, grounded, vy);
	});
</script>

<T.PerspectiveCamera makeDefault fov={62} bind:ref={cam} position={[0, 6, 16]} />

<RigidBody type="kinematicPosition" bind:rigidBody={body}>
	<Collider shape="capsule" args={[0.5, 0.4]} bind:collider={collider} />
	<T.Group bind:ref={visual}>
		<!-- humanoid avatar; offset down so the feet sit at the ground (group origin = capsule centre) -->
		<T.Group position={[0, -CAPSULE_HALF, 0]}>
			<T.Mesh position={[0, 1.05, 0]} geometry={NPC.torso} material={creatureMat('#e8794b')} castShadow />
			<T.Mesh position={[0, 1.62, 0]} geometry={NPC.head} material={creatureMat('#f0c293')} castShadow />
			<T.Group bind:ref={armL} position={[0.34, 1.4, 0]}>
				<T.Mesh position={[0, -0.3, 0]} geometry={NPC.arm} material={creatureMat('#e8794b')} castShadow />
			</T.Group>
			<T.Group bind:ref={armR} position={[-0.34, 1.4, 0]}>
				<T.Mesh position={[0, -0.3, 0]} geometry={NPC.arm} material={creatureMat('#e8794b')} castShadow />
			</T.Group>
			<T.Group bind:ref={legL} position={[0.14, 0.7, 0]}>
				<T.Mesh position={[0, -0.35, 0]} geometry={NPC.leg} material={creatureMat('#34507f')} castShadow />
			</T.Group>
			<T.Group bind:ref={legR} position={[-0.14, 0.7, 0]}>
				<T.Mesh position={[0, -0.35, 0]} geometry={NPC.leg} material={creatureMat('#34507f')} castShadow />
			</T.Group>
		</T.Group>
	</T.Group>
</RigidBody>
