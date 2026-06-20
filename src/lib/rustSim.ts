/**
 * THE agent engine — the headless Rust/WASM core (`crates/worldsim`) IS the simulation; JS + three.js is a thin
 * render layer (see the `rust-owns-all-compute` memory + the migration roadmap in the work-queue). There is no
 * JS sim and no toggle: the legacy `agents.svelte.ts` tick is being deleted. The `Sim` keeps its state in WASM
 * linear memory; we read transforms back as zero-copy typed-array VIEWS (never a per-agent JS↔WASM call) and
 * mirror them onto the `ManagedAgent`s, which are now just lightweight render mirrors the renderers read
 * (`m.agent.rx/rz/rh` + `m.dead/m.asleep`).
 *
 * Build the bundle first: `pnpm build:wasm` (emits to `static/worldsim/`, served at `/worldsim/`). If the wasm
 * fails to load, agents stay put and a console error fires — there is NO JS fallback, by design.
 *
 * STILL ON THE RUST TODO (tracked in the work-queue, being ported): ambient-forest tree push-out (`#resolveTrees`),
 * the player-pet `companion` follow, and the placement search (`findFreeSpot`). Until then those behave as noted.
 */
import { base } from '$app/paths';
import { agentManager, type ManagedAgent } from './agents.svelte';
import { fishRegistry } from './fish.svelte';
import { playerState } from './playerState.svelte';

// kind string → the stable code the Rust `kind_from_code` expects (enum order: see crates/worldsim/src/eco.rs)
const KIND_CODE: Record<string, number> = { rabbit: 0, cat: 1, kangaroo: 2, person: 3, lion: 4, dinosaur: 5 };

// minimal self-typed surface of the generated wasm module — so svelte-check needs NO generated pkg files and
// the default (JS) path never imports them. Mirrors the `#[wasm_bindgen] Sim` in crates/worldsim/src/lib.rs.
interface RustSim {
	spawn(x: number, z: number, kindCode: number, radius: number, seedId: number): number;
	set_player(x: number, z: number): void;
	set_companion(i: number): void;
	despawn(i: number): void;
	set_night(n: number): void;
	set_fish(xz: Float64Array): void;
	set_obstacles(flat: Float64Array): void;
	step(dt: number): void;
	count(): number;
	danger(): number;
	xs_ptr(): number;
	zs_ptr(): number;
	headings_ptr(): number;
	healths_ptr(): number;
	flags_ptr(): number;
	behaviors_ptr(): number;
	progress_ptr(): number;
}
interface WasmExports {
	memory: WebAssembly.Memory;
}
interface WasmModule {
	default: (input?: unknown) => Promise<WasmExports>;
	Sim: new () => RustSim;
}

let wasm: WasmExports | null = null;
let sim: RustSim | null = null;
let status: 'off' | 'loading' | 'ready' | 'failed' = 'off';

const slotOf = new WeakMap<ManagedAgent, number>(); // already-spawned agents → their Rust index
const tracked: ManagedAgent[] = []; // index = Rust slot → the agent to write transforms back onto

// typed-array views over WASM memory, rebuilt when the buffer detaches (grows) or the agent count changes
let viewBuf: ArrayBuffer | null = null;
let viewCount = -1;
let xs: Float32Array = new Float32Array();
let zs: Float32Array = new Float32Array();
let headings: Float32Array = new Float32Array();
let healths: Float32Array = new Float32Array();
let flags: Uint32Array = new Uint32Array();
let behaviors: Uint8Array = new Uint8Array();
let progress: Float32Array = new Float32Array();
// behaviour code → the renderer's pose name (must match crates/worldsim Behavior::code order)
const BEHAVIORS = ['wander', 'pause', 'lookAround', 'groom', 'sit', 'pounce'] as const;

// latest obstacle set, kept so it survives the async wasm load (Scene may push obstacles before the Sim exists)
let pendingObstacles: Float64Array | null = null;

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
	if (sim) sim.set_obstacles(flat);
}

/** Lifecycle status — `AgentSystem` only ticks the world once this is `'ready'` (agents idle while loading;
 *  `'failed'` means the wasm didn't load → agents stay put, no JS fallback). */
export function rustStatus(): typeof status {
	return status;
}

/** Lazy-load the wasm bundle + construct the `Sim`. Idempotent; resolves true once the engine is live. */
export async function initRustSim(): Promise<boolean> {
	if (status === 'ready') return true;
	if (status === 'loading') return false;
	status = 'loading';
	try {
		// runtime URL (served from static/) → kept opaque to Vite so the wasm glue resolves its own .wasm via
		// import.meta.url. @vite-ignore: do not try to bundle/transform this dynamic import.
		const mod = (await import(/* @vite-ignore */ `${base}/worldsim/worldsim.js`)) as unknown as WasmModule;
		wasm = await mod.default();
		sim = new mod.Sim();
		if (pendingObstacles) sim.set_obstacles(pendingObstacles); // apply anything Scene pushed before we loaded
		status = 'ready';
		console.info('[rustSim] engine=rust ready');
		return true;
	} catch (e) {
		status = 'failed';
		console.error('[rustSim] init failed — agents will not move (no JS fallback). Did you run `pnpm build:wasm`?', e);
		return false;
	}
}

/** Rebuild the memory views if the WASM buffer grew (detached) or the agent count changed. */
function refreshViews(): void {
	if (!sim || !wasm) return;
	const c = sim.count();
	if (viewBuf === wasm.memory.buffer && viewCount === c) return;
	viewBuf = wasm.memory.buffer;
	viewCount = c;
	xs = new Float32Array(viewBuf, sim.xs_ptr(), c);
	zs = new Float32Array(viewBuf, sim.zs_ptr(), c);
	headings = new Float32Array(viewBuf, sim.headings_ptr(), c);
	healths = new Float32Array(viewBuf, sim.healths_ptr(), c);
	flags = new Uint32Array(viewBuf, sim.flags_ptr(), c);
	behaviors = new Uint8Array(viewBuf, sim.behaviors_ptr(), c);
	progress = new Float32Array(viewBuf, sim.progress_ptr(), c);
}

/** Spawn any newly-registered agents into the Rust world (at their current pose → continuity on hot-swap). */
function syncRoster(): void {
	if (!sim) return;
	agentManager.forEach((m) => {
		if (slotOf.has(m)) return;
		const code = KIND_CODE[m.kind] ?? 0;
		const i = sim!.spawn(m.agent.x, m.agent.z, code, m.radius, m.seedId);
		if (m.companion) sim!.set_companion(i); // the player's pet → follows you, won't flee you
		slotOf.set(m, i);
		tracked[i] = m;
	});
}

/** Current danger imminence (0..1) from the Rust core, for the UI vignette in rust mode (0 until ready). */
export function rustDanger(): number {
	return sim ? sim.danger() : 0;
}

/**
 * One fixed-DT tick driven by the Rust core. Mirrors the JS tick's contract: snapshot each agent's pose for
 * render interpolation (as agents.svelte.ts does before moving), advance the sim, then copy transforms + the
 * dead/asleep flags back onto the `ManagedAgent`s. Call once per emitted clock tick (like `agentManager.tick`).
 */
export function tickRust(dt: number): void {
	if (!sim || !wasm) return;
	syncRoster();
	sim.set_player(playerState.pos[0], playerState.pos[2]);
	sim.set_night(agentManager.nightValue);
	feedFish();
	// snapshot prev pose BEFORE the step so interpolate(alpha) blends prev→new (mirrors agents.svelte.ts:428)
	for (let i = 0; i < tracked.length; i++) tracked[i]?.agent.savePrev();
	sim.step(dt);
	refreshViews();
	const TAU = Math.PI * 2;
	let huntX = 0;
	let huntZ = 0;
	let huntD2 = Infinity; // nearest active player-hunter → for the "it's behind you" dread cue
	for (let i = 0; i < tracked.length; i++) {
		const m = tracked[i];
		if (!m) continue;
		if (!agentManager.has(m)) {
			// its component unmounted (object removed / world cleared) → drop it from the Rust sim so it doesn't
			// linger as an invisible ghost that still steers the food chain. Slot is retired (read-back is by index).
			sim.despawn(i);
			slotOf.delete(m);
			tracked[i] = undefined as unknown as ManagedAgent;
			continue;
		}
		const a = m.agent;
		const nx = xs[i];
		const nz = zs[i];
		const nh = headings[i];
		// derive speed + turnRate from the per-tick delta (prev pose was snapshot by savePrev above) so the
		// renderers' gait (leg swing) + banking animate — the Rust read-back gives pose, not velocity.
		a.speed = Math.hypot(nx - a.prevX, nz - a.prevZ) / dt;
		let dh = nh - a.prevHeading;
		while (dh > Math.PI) dh -= TAU;
		while (dh < -Math.PI) dh += TAU;
		a.turnRate = dh / dt;
		a.x = nx;
		a.z = nz;
		a.heading = nh;
		a.behavior = BEHAVIORS[behaviors[i]] ?? 'wander'; // idle pose (sit/groom/pounce/…) for the near renderers
		a.progress = progress[i]; // 0..1 through that behaviour → groom cycle / pounce arc / lookAround sweep
		m.health = healths[i];
		const f = flags[i];
		m.dead = (f & 1) !== 0;
		m.asleep = (f & 2) !== 0;
		m.hunting = (f & 8) !== 0; // bit3 → this apex is charging the player → the view glares its eyes
		if (m.hunting) {
			const dx = nx - playerState.pos[0];
			const dz = nz - playerState.pos[2];
			const d2 = dx * dx + dz * dz;
			if (d2 < huntD2) ((huntD2 = d2), (huntX = nx), (huntZ = nz));
		}
	}
	// IS THE HUNTER BEHIND YOU? The dread of an unseen pursuer. Player forward = (-sin yaw, -cos yaw) (matches
	// Player.svelte movement); dot with the direction to the nearest hunter < 0 → it's in your back hemisphere.
	let behindTarget = 0;
	if (huntD2 < Infinity) {
		const yaw = playerState.yaw;
		const fx = -Math.sin(yaw);
		const fz = -Math.cos(yaw);
		let tx = huntX - playerState.pos[0];
		let tz = huntZ - playerState.pos[2];
		const tl = Math.hypot(tx, tz) || 1;
		tx /= tl;
		tz /= tl;
		if (fx * tx + fz * tz < -0.15) behindTarget = 1; // small deadzone so a side-on hunter doesn't flicker it
	}
	playerState.dangerBehind += (behindTarget - playerState.dangerBehind) * Math.min(1, 4 * dt); // eased
	// the Rust read-back has positions but not the per-agent perf flags — recompute LOD + shadow budget so the
	// impostor/shadow culling (and thus FPS) is identical to the JS path.
	agentManager.assignPerfFlags(playerState.pos[0], playerState.pos[2]);
	// mirror the eased Rust danger onto playerState so the UI vignette swells/fades (the JS tick — which would
	// otherwise write this — doesn't run in rust mode).
	playerState.danger = sim.danger();
}

// reused buffer for the lure points fed to the Rust sim each tick (fish move every frame, so re-feed)
let fishScratch = new Float64Array(0);

/** Push the live lake-fish positions into the Rust sim so an idle cat is lured to the bank (the pond obstacle
 *  then stops it dry). Reuses a scratch buffer; feeds an empty set to clear when the last school unregisters. */
function feedFish(): void {
	if (!sim) return;
	const fc = fishRegistry.count;
	if (fc === 0) {
		if (fishScratch.length > 0) {
			fishScratch = new Float64Array(0);
			sim.set_fish(fishScratch);
		}
		return;
	}
	if (fishScratch.length !== fc * 2) fishScratch = new Float64Array(fc * 2);
	let i = 0;
	fishRegistry.forEach((f) => {
		fishScratch[i++] = f.x;
		fishScratch[i++] = f.z;
	});
	sim.set_fish(fishScratch);
}
