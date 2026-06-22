/**
 * THE agent engine adapter — the headless Rust/WASM core (`crates/worldsim`) IS the simulation; JS + three.js is
 * a thin render layer (see the `rust-owns-all-compute` memory). As of the perf foundation (see the
 * `perf-foundation-plan` memory), the wasm + the `Sim` run in a WEB WORKER (worldsim.worker.ts) — OFF the main
 * thread — so stepping 1000 agents no longer steals frame time from render. This module is the main-thread
 * PROXY: it owns the agent roster, drives the worker with one `tick` message per sim tick, and mirrors the
 * snapshot the worker posts back onto the `ManagedAgent`s the renderers read (`m.agent.rx/rz/rh`, `m.dead`, …).
 *
 * Why this is clean despite the async boundary: the main thread owns the stable-slot free-list. A removed
 * agent's slot is retired before a new spawn fills it via `Sim.spawn_at`, so no async round-trip is needed to
 * learn indices and lifetime churn stays bounded. We step exactly once per clock tick; determinism is preserved.
 *
 * Build the bundle first: `pnpm build:wasm` (emits to `static/worldsim/`). If the worker/wasm fails to load,
 * agents stay put and a console error fires — there is NO main-thread fallback, by design.
 *
 * STILL ON THE RUST TODO: ambient-forest tree push-out, the player-pet `companion` follow nuances, the placement
 * search.
 */
import { base } from '$app/paths';
import { agentManager, type ManagedAgent } from './agents.svelte';
import { fishRegistry } from './fish.svelte';
import { playerState } from './playerState.svelte';

// kind string → the stable code the Rust `kind_from_code` expects (enum order: see crates/worldsim/src/eco.rs)
const KIND_CODE: Record<string, number> = { rabbit: 0, cat: 1, kangaroo: 2, person: 3, lion: 4, dinosaur: 5 };
const KIND_NAME = ['rabbit', 'cat', 'kangaroo', 'person', 'lion', 'dinosaur'] as const; // birth kindCode → kind
// behaviour code → the renderer's pose name (must match crates/worldsim Behavior::code order)
const BEHAVIORS = ['wander', 'pause', 'lookAround', 'groom', 'sit', 'pounce'] as const;

// ── worker message shapes (mirror worldsim.worker.ts) ───────────────────────────────────────────────
type Spawn = { slot: number; x: number; z: number; code: number; radius: number; seedId: number; companion: boolean; juvenile: boolean; gene: number; pfamA: number; pfamB: number; genome: number[] | null };
type Snap = {
	type: 'snap';
	seq: number;
	count: number;
	xs: Float32Array;
	zs: Float32Array;
	headings: Float32Array;
	healths: Float32Array;
	flags: Uint32Array;
	behaviors: Uint8Array;
	progress: Float32Array;
	births: Float32Array;
	builds: Float32Array; // [x,z]×n — house-build requests (Scene places them)
	wells: Float32Array; // [x,z]×n — well-dig requests (Scene places a well + feeds it back as a drink source)
	events: Float32Array; // [code,kind,x,z]×n — sim events from the worker (currently unused client-side)
	danger: number;
	ageMeans: Float32Array; // 6 — mean age fraction (0..1) per kind; -1 = none alive (HUD age readout)
};
type OutMsg =
	| { type: 'init'; base: string; obstacles: Float64Array | null }
	| { type: 'obstacles'; flat: Float64Array }
	| { type: 'refuges'; xz: Float64Array }
	| { type: 'water'; xzr: Float64Array }
	| { type: 'aridity'; a: number }
	| { type: 'behaviorMode'; code: number }
	| { type: 'tick'; seq: number; dt: number; px: number; pz: number; night: number; popScale: number; fish: Float64Array; spawns: Spawn[]; despawns: number[] };
type WorkerMsg = { type: 'ready' } | { type: 'failed'; error: string } | Snap;

// newborns from the Rust breeding pass (kind + position) → Scene drains them into world.objects each frame
export type Birth = { kind: string; x: number; z: number; gene: number; pfamA: number; pfamB: number; genome: number[] };
let pendingBirths: Birth[] = [];
/** Pull (and clear) the babies bred since the last call — Scene turns each into a world-object. */
export function drainBirths(): Birth[] {
	if (pendingBirths.length === 0) return pendingBirths;
	const out = pendingBirths;
	pendingBirths = [];
	return out;
}

// house-build requests from settlers (x,z) → Scene places houses
export type Build = { x: number; z: number };
let pendingBuilds: Build[] = [];
/** Pull (and clear) the house-build requests since the last call — Scene places each as a house world-object. */
export function drainBuilds(): Build[] {
	if (pendingBuilds.length === 0) return pendingBuilds;
	const out = pendingBuilds;
	pendingBuilds = [];
	return out;
}

// well-dig requests from industrious settlers (x,z) → Scene places a well AND feeds it back as a drink source
export type Well = { x: number; z: number };
let pendingWells: Well[] = [];
/** Pull (and clear) the well-dig requests since the last call — Scene places each as a well + a new water source. */
export function drainWells(): Well[] {
	if (pendingWells.length === 0) return pendingWells;
	const out = pendingWells;
	pendingWells = [];
	return out;
}

// conception events (a pair just bonded) → Scene floats a heart at the spot. [x,z] at the mom (≈ the couple's midpoint).
export type Love = { x: number; z: number };
let pendingLoves: Love[] = [];
/** Pull (and clear) the bonding/conception spots since the last call — Scene pops a floating heart at each. */
export function drainLoves(): Love[] {
	if (pendingLoves.length === 0) return pendingLoves;
	const out = pendingLoves;
	pendingLoves = [];
	return out;
}

let worker: Worker | null = null;
let status: 'off' | 'loading' | 'ready' | 'failed' = 'off';

let nextSlot = 0; // stable-slot high-water mark
const freeSlots: number[] = []; // reaped tombstones, reused deterministically (LIFO)
const slotOf = new WeakMap<ManagedAgent, number>(); // already-spawned agents → their Rust slot
const tracked: ManagedAgent[] = []; // index = Rust slot → the agent to mirror the snapshot onto

let snap: Snap | null = null; // latest snapshot from the worker (not yet applied)
let appliedSeq = -1; // seq of the last snapshot we applied (skip re-applying the same one)
let postSeq = 0; // monotonic id stamped on each tick message (echoed back on its snapshot)
let lastDanger = 0; // most recent danger imminence (0..1) from the worker → the UI vignette
let behindTarget = 0; // 1 while a hunter is in your back hemisphere → eased into playerState.dangerBehind

let pendingObstacles: Float64Array | null = null; // survives the async worker load; flushed on 'ready'
let pendingRefuges: Float64Array | null = null; // house centres (flee-to-safety); survives load, flushed on 'ready'
let pendingWater: Float64Array | null = null; // drinkable water sources [x,z,r]×n (thirst); survives load, flushed on 'ready'
let pendingAridity: number | null = null; // director drought level; survives load, flushed on 'ready'
let popScale = 1; // world-area multiplier for prey caps (Scene computes it from the world's extent), rides the tick msg
let behaviorCode = 1; // which decision brain the Rust world runs: 0 Manual · 1 Emergent (the default, see Sim::new)

/** Switch the agent decision brain at runtime (the design's mode toggle — docs/emergent-behavior.md). `emergent`
 *  true = the needs+primitives+utility scorer (default); false = the hand-coded Manual sim. Survives worker load
 *  (re-sent on 'ready') so a chosen mode sticks. Returns the new mode for the caller's UI state. */
export function setRustBehaviorMode(emergent: boolean): boolean {
	behaviorCode = emergent ? 1 : 0;
	if (worker && status === 'ready') worker.postMessage({ type: 'behaviorMode', code: behaviorCode } satisfies OutMsg);
	return emergent;
}

/** The decision brain the sim is currently running (true = Emergent, false = Manual) — for the HUD readout. */
export function rustBehaviorIsEmergent(): boolean {
	return behaviorCode === 1;
}

let lastAgeMeans = new Float32Array(6).fill(-1); // mean age fraction per kind from the latest snapshot (HUD)
/** Mean AGE as a fraction of lifespan (0..1) per kind [rabbit,cat,kangaroo,person,lion,dino]; -1 = none alive. */
export function rustAgeMeans(): Float32Array {
	return lastAgeMeans;
}

/** Set the world-AREA population multiplier — bigger/more-built world → higher prey caps (predators follow prey).
 *  Scene feeds this from the static-object extent; it rides the next tick message to the Rust world. */
export function setRustPopScale(s: number): void {
	popScale = s;
}

/** Feed the solid obstacles (props/buildings/ponds) to the Rust world. Accepts the same shape Scene builds for
 *  agentManager.setObstacles; flattened to the packed [x,z,r,hx,hz,cos,sin] layout (circle → hx = NaN). */
export function setRustObstacles(obs: { x: number; z: number; r: number; hx?: number; hz?: number; cos?: number; sin?: number }[]): void {
	const flat = new Float64Array(obs.length * 7);
	for (let i = 0; i < obs.length; i++) {
		const o = obs[i];
		const b = i * 7;
		flat[b] = o.x;
		flat[b + 1] = o.z;
		flat[b + 2] = o.r;
		flat[b + 3] = o.hx ?? NaN; // NaN → circle
		flat[b + 4] = o.hz ?? 0;
		flat[b + 5] = o.cos ?? 0;
		flat[b + 6] = o.sin ?? 0;
	}
	pendingObstacles = flat;
	if (worker && status === 'ready') worker.postMessage({ type: 'obstacles', flat } satisfies OutMsg); // clone (no transfer — keep pendingObstacles intact)
}

/** Feed the REFUGE points (house/settlement centres) to the Rust world — a threatened woman/child flees toward the
 *  nearest one (home is safety, and where the guard men cluster). `pts` is the building centres Scene already has. */
export function setRustRefuges(pts: { x: number; z: number }[]): void {
	const xz = new Float64Array(pts.length * 2);
	for (let i = 0; i < pts.length; i++) {
		xz[i * 2] = pts[i].x;
		xz[i * 2 + 1] = pts[i].z;
	}
	pendingRefuges = xz;
	if (worker && status === 'ready') worker.postMessage({ type: 'refuges', xz } satisfies OutMsg); // clone (keep pendingRefuges intact)
}

/** Feed the DRINKABLE water sources (pond centre + radius) to the sim — every animal must reach a bank to slake
 *  thirst or it dies. `ponds` is the same water-zone set Scene already uses for obstacles. */
export function setRustWater(ponds: { x: number; z: number; r: number }[]): void {
	const xzr = new Float64Array(ponds.length * 3);
	for (let i = 0; i < ponds.length; i++) {
		xzr[i * 3] = ponds[i].x;
		xzr[i * 3 + 1] = ponds[i].z;
		xzr[i * 3 + 2] = ponds[i].r;
	}
	pendingWater = xzr;
	if (worker && status === 'ready') worker.postMessage({ type: 'water', xzr } satisfies OutMsg);
}

/** DROUGHT level from the macro-director (Mother Nature / LLM): 1 = normal, >1 drier, <1 wetter. Scales thirst. */
export function setRustAridity(a: number): void {
	pendingAridity = a;
	if (worker && status === 'ready') worker.postMessage({ type: 'aridity', a } satisfies OutMsg);
}

/** Lifecycle status — `AgentSystem` only ticks the world once this is `'ready'` (agents idle while loading;
 *  `'failed'` means the worker/wasm didn't load → agents stay put, no main-thread fallback). */
export function rustStatus(): typeof status {
	return status;
}

/** Monotonic sim tick (the applied-snapshot seq) — a cheap clock for region streaming's dormant span + telemetry. */
export function rustTick(): number {
	return appliedSeq < 0 ? 0 : appliedSeq;
}

/** Lazy-spawn the sim worker (it loads the wasm + constructs the `Sim`). Idempotent; resolves true once ready. */
export async function initRustSim(): Promise<boolean> {
	if (status === 'ready') return true;
	if (status === 'loading') return false;
	if (typeof Worker === 'undefined') {
		status = 'failed';
		return false; // not in a browser (SSR / no worker support) — the app is client-only so this shouldn't hit
	}
	status = 'loading';
	return new Promise<boolean>((resolve) => {
		try {
			worker = new Worker(new URL('./worldsim.worker.ts', import.meta.url), { type: 'module' });
			worker.onmessage = (e: MessageEvent<WorkerMsg>) => {
				const d = e.data;
				if (d.type === 'ready') {
					status = 'ready';
					if (pendingObstacles) worker!.postMessage({ type: 'obstacles', flat: pendingObstacles } satisfies OutMsg);
					if (pendingRefuges) worker!.postMessage({ type: 'refuges', xz: pendingRefuges } satisfies OutMsg);
					if (pendingWater) worker!.postMessage({ type: 'water', xzr: pendingWater } satisfies OutMsg);
					if (pendingAridity != null) worker!.postMessage({ type: 'aridity', a: pendingAridity } satisfies OutMsg);
					if (behaviorCode !== 1) worker!.postMessage({ type: 'behaviorMode', code: behaviorCode } satisfies OutMsg); // re-assert a non-default (Manual) choice across loads
					console.info('[rustSim] engine=rust ready (worker)');
					resolve(true);
				} else if (d.type === 'failed') {
					status = 'failed';
					console.error('[rustSim] worker init failed — agents will not move (no fallback). Did you run `pnpm build:wasm`?', d.error);
					resolve(false);
				} else if (d.type === 'snap') {
					snap = d; // newest snapshot — applied on the next tickRust
				}
			};
			worker.onerror = (err) => {
				status = 'failed';
				console.error('[rustSim] worker error', err.message);
				resolve(false);
			};
			worker.postMessage({ type: 'init', base, obstacles: pendingObstacles } satisfies OutMsg);
		} catch (e) {
			status = 'failed';
			console.error('[rustSim] could not spawn the sim worker', e);
			resolve(false);
		}
	});
}

/** Current danger imminence (0..1), for the UI vignette in rust mode (0 until ready). */
export function rustDanger(): number {
	return lastDanger;
}

/** Live fish positions → a fresh transferable buffer (the worker uses them to lure an idle cat to the bank). */
function collectFish(): Float64Array {
	const out = new Float64Array(fishRegistry.count * 2);
	let i = 0;
	fishRegistry.forEach((f) => {
		out[i++] = f.x;
		out[i++] = f.z;
	});
	return out;
}

/**
 * One fixed-DT tick. Mirrors the old synchronous contract from the caller's view (AgentSystem still calls this
 * once per emitted clock tick), but the heavy lifting now happens in the worker: here we (1) APPLY the latest
 * snapshot onto the ManagedAgents — savePrev → set pose → the renderers interpolate; (2) diff the roster into
 * spawn/despawn commands; (3) post the next step to the worker. The snapshot for THIS step arrives before the
 * next frame, so we run ~1 tick behind — invisible at ambient speeds.
 */
export function tickRust(dt: number): void {
	if (!worker || status !== 'ready') return;
	const s = snap;
	const hasSnap = s !== null && s.seq !== appliedSeq;
	if (hasSnap) {
		appliedSeq = s!.seq;
		lastDanger = s!.danger;
		if (s!.ageMeans) lastAgeMeans = new Float32Array(s!.ageMeans); // mean age fraction per kind → HUD age readout
		// drain this snapshot's NEWBORNS (Rust bred them) → Scene turns each into a world-object (which mounts +
		// spawns back into the sim as a juvenile). Flat [kindCode,x,z,…].
		const nb = s!.births.length / 11; // [kc, x, z, gene, motherFam, fatherFam, g0..g4] per birth
		for (let k = 0; k < nb; k++) {
			const o = k * 11;
			pendingBirths.push({ kind: KIND_NAME[s!.births[o]] ?? 'rabbit', x: s!.births[o + 1], z: s!.births[o + 2], gene: s!.births[o + 3], pfamA: s!.births[o + 4], pfamB: s!.births[o + 5], genome: [s!.births[o + 6], s!.births[o + 7], s!.births[o + 8], s!.births[o + 9], s!.births[o + 10]] });
		}
		// drain HOUSE-BUILD requests (settlers) → Scene places each as a house world-object. Flat [x,z,…].
		const nbd = s!.builds.length / 2;
		for (let k = 0; k < nbd; k++) pendingBuilds.push({ x: s!.builds[k * 2], z: s!.builds[k * 2 + 1] });
		const nw = s!.wells.length / 2;
		for (let k = 0; k < nw; k++) pendingWells.push({ x: s!.wells[k * 2], z: s!.wells[k * 2 + 1] });
		// drain CONCEIVE events (a pair just bonded) → Scene floats a heart at the spot. Events are [code,kind,x,z]×n.
		const ne = s!.events.length / 4;
		for (let k = 0; k < ne; k++) {
			const o = k * 4;
			if (s!.events[o] === 6 /* EV_CONCEIVE */) pendingLoves.push({ x: s!.events[o + 2], z: s!.events[o + 3] });
		}
	}

	const px = playerState.pos[0];
	const pz = playerState.pos[2];
	const TAU = Math.PI * 2;
	let huntX = 0;
	let huntZ = 0;
	let huntD2 = Infinity; // nearest active player-hunter → the "it's behind you" dread cue

	const spawns: Spawn[] = [];
	const despawns: number[] = [];

	// Retire unregistered agents first. The worker applies despawns before spawns, so these tombstones can be
	// safely reused by new registrations in this same roster diff.
	for (let i = 0; i < tracked.length; i++) {
		const m = tracked[i];
		if (!m) continue;
		if (!agentManager.has(m)) {
			// its component unmounted (object removed / world cleared) → tell the worker to drop it from the sim so
			// it doesn't linger as an invisible ghost still steering the food chain.
			despawns.push(i);
			slotOf.delete(m);
			tracked[i] = undefined as unknown as ManagedAgent;
			freeSlots.push(i);
			continue;
		}
	}

	// Newly registered agents reuse a reaped slot before extending the stable-slot high-water mark.
	agentManager.forEach((m) => {
		if (slotOf.has(m)) return;
		const slot = freeSlots.pop() ?? nextSlot++;
		slotOf.set(m, slot);
		tracked[slot] = m;
		spawns.push({ slot, x: m.agent.x, z: m.agent.z, code: KIND_CODE[m.kind] ?? 0, radius: m.radius, seedId: m.seedId, companion: !!m.companion, juvenile: !!m.juvenile, gene: m.gene ?? 1, pfamA: m.pfamA ?? 0, pfamB: m.pfamB ?? 0, genome: m.genome ? Array.from(m.genome) : null }); // plain copy — m.genome may be a $state Proxy (not cloneable to the worker)
	});

	// Mirror a fresh snapshot onto the live roster.
	for (let i = 0; i < tracked.length; i++) {
		const m = tracked[i];
		if (!m) continue;
		if (!hasSnap || i >= s!.count) continue;
		const a = m.agent;
		a.savePrev(); // prev = last applied snapshot pose; current ← this snapshot → renderers interpolate prev→new
		const nx = s!.xs[i];
		const nz = s!.zs[i];
		const nh = s!.headings[i];
		if (!a.appeared) {
			// first snapshot for a freshly-spawned agent → zero the delta (its ctor heading was random) so it
			// doesn't show a bogus speed/turn spike and topple over on spawn; it eases into real motion next tick.
			a.prevX = nx;
			a.prevZ = nz;
			a.prevHeading = nh;
			a.appeared = true;
		}
		// derive speed + turnRate from the per-tick delta so the gait (leg swing) + banking animate
		a.speed = Math.hypot(nx - a.prevX, nz - a.prevZ) / dt;
		let dh = nh - a.prevHeading;
		while (dh > Math.PI) dh -= TAU;
		while (dh < -Math.PI) dh += TAU;
		a.turnRate = dh / dt;
		a.x = nx;
		a.z = nz;
		a.heading = nh;
		a.behavior = BEHAVIORS[s!.behaviors[i]] ?? 'wander';
		a.progress = s!.progress[i];
		m.health = s!.healths[i];
		const f = s!.flags[i];
		m.dead = (f & 1) !== 0;
		m.corpseAge = m.dead ? m.corpseAge + dt : 0; // age corpses → Scene's reaper sinks + removes the old ones
		m.asleep = (f & 2) !== 0;
		m.hunting = (f & 8) !== 0; // bit3 → this apex is charging the player → the view glares its eyes
		m.migrating = (f & 16) !== 0; // bit4 → roamer en route to another settlement (HUD)
		m.pregnant = (f & 32) !== 0; // bit5 → carrying a litter → the view shows a belly
		m.guardian = (f & 64) !== 0; // bit6 → his mate is expecting → the view arms him with a machete
		m.drinking = (f & 128) !== 0; // bit7 → lapping at a water edge → the view dips its head (watering hole)
		if (m.hunting) {
			const dx = nx - px;
			const dz = nz - pz;
			const d2 = dx * dx + dz * dz;
			if (d2 < huntD2) ((huntD2 = d2), (huntX = nx), (huntZ = nz));
		}
	}

	// post the step to the worker (transfer the fish buffer — the worker takes ownership)
	const fish = collectFish();
	worker.postMessage({ type: 'tick', seq: ++postSeq, dt, px, pz, night: agentManager.nightValue, popScale, fish, spawns, despawns } satisfies OutMsg, [fish.buffer]);

	// IS THE HUNTER BEHIND YOU? (recompute the target only when a fresh snapshot gave us hunter positions; ease
	// every tick for smoothness). Player forward = (-sin yaw, -cos yaw); dot with the dir to the nearest hunter
	// < 0 → it's in your back hemisphere. The dread of an unseen pursuer.
	if (hasSnap) {
		behindTarget = 0;
		if (huntD2 < Infinity) {
			const yaw = playerState.yaw;
			const fx = -Math.sin(yaw);
			const fz = -Math.cos(yaw);
			let tx = huntX - px;
			let tz = huntZ - pz;
			const tl = Math.hypot(tx, tz) || 1;
			tx /= tl;
			tz /= tl;
			if (fx * tx + fz * tz < -0.15) behindTarget = 1; // small deadzone so a side-on hunter doesn't flicker it
		}
	}
	playerState.dangerBehind += (behindTarget - playerState.dangerBehind) * Math.min(1, 4 * dt); // eased

	// the worker produced positions but not the per-agent perf flags — recompute LOD + shadow budget so the
	// impostor/shadow culling (and thus FPS) is unchanged. Cheap, no alloc; runs every tick (player moves).
	agentManager.assignPerfFlags(px, pz);
	playerState.danger = lastDanger; // mirror the eased Rust danger onto playerState so the UI vignette swells/fades
}
