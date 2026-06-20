/**
 * OPTIONAL Rust/WASM sim backend, behind `?engine=rust` (default OFF → the JS `agentManager` drives).
 *
 * This is step 2 of the engine port (docs/self-sustaining-world.md §6.6 / §7, and the work-queue): a thin JS
 * adapter that lets the headless Rust core (`crates/worldsim`) drive the ambient agents IN-BROWSER so the user
 * can A/B it against the live JS sim. The Rust `Sim` keeps its state in WASM linear memory; we read transforms
 * back as zero-copy typed-array VIEWS (never a per-agent JS↔WASM call) and mirror them onto the existing
 * `ManagedAgent`s, so every renderer (Critter / Npc / AgentImpostors) works UNCHANGED — they still read
 * `m.agent.rx/rz/rh` + `m.dead/m.asleep` exactly as before.
 *
 * Build the bundle first: `pnpm build:wasm` (emits to `static/worldsim/`, served at `/worldsim/`). Then open
 * the app with `?engine=rust`. Until the wasm finishes loading — and if it fails — the JS sim drives, seamless.
 *
 * KNOWN GAPS in this first cut (it's a SIM-BEHAVIOUR A/B toggle, not a full swap yet — all tracked for the
 * shakeout iteration): the Rust core has no world obstacles, so rust-mode agents ignore walls/water (no
 * pond/house collision, so the fish-lure has no bank to stop at); per-agent LOD/`dist`/`lod` flags aren't fed
 * yet (the impostor tier is a later Rust chunk); the player-pet `companion` isn't modelled. Behaviour is NOT
 * byte-identical to the JS sim by design (the Rust core double-buffers + is seeded by the addressed RNG).
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

/** Which sim engine drives the agents. Rust is the DEFAULT now; `?engine=js` forces the legacy JS sim (for
 *  A/B). (The app is client-only three.js — no SSR to guard.) */
export function engineIsRust(): boolean {
	return new URLSearchParams(location.search).get('engine') !== 'js';
}

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

/** Lifecycle status — `AgentSystem` falls back to the JS sim unless this is `'ready'`. */
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
		console.error('[rustSim] init failed — staying on the JS sim', e);
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
}

/** Spawn any newly-registered agents into the Rust world (at their current pose → continuity on hot-swap). */
function syncRoster(): void {
	if (!sim) return;
	agentManager.forEach((m) => {
		if (slotOf.has(m)) return;
		const code = KIND_CODE[m.kind] ?? 0;
		const i = sim!.spawn(m.agent.x, m.agent.z, code, m.radius, m.seedId);
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
	for (let i = 0; i < tracked.length; i++) {
		const m = tracked[i];
		if (!m) continue;
		m.agent.x = xs[i];
		m.agent.z = zs[i];
		m.agent.heading = headings[i];
		m.health = healths[i];
		const f = flags[i];
		m.dead = (f & 1) !== 0;
		m.asleep = (f & 2) !== 0;
	}
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
