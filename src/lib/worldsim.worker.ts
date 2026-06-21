/// <reference lib="webworker" />
/**
 * The Rust/WASM world-sim, running OFF the main thread (perf foundation 2/3 — see the `perf-foundation-plan`
 * memory). The engine is unchanged; we just moved WHERE it runs: the wasm + the `Sim` live here, in a dedicated
 * worker, so stepping 1000 agents no longer steals frame time from render. The main thread (rustSim.ts) drives
 * this with one `tick` message per sim tick and renders the snapshot we post back.
 *
 * The contract that makes this clean: Rust `spawn` is APPEND-ONLY and `despawn` only TOMBSTONES (the slot is
 * never reused — read-back is index-stable). So the main thread can PREDICT each new agent's slot with a plain
 * monotonic counter and tell us that slot up-front; we just trust `sim.spawn()` returns the same index (asserted
 * below). No async round-trip to learn indices. Determinism is preserved: we step exactly once per `tick`
 * message, and the main clock emits exactly one message per clock tick (pause → no messages → frozen).
 *
 * Snapshot transport: each step we COPY the read-back arrays (slice → owned buffers) and TRANSFER them to the
 * main thread (zero-copy handoff; ~28 KB for 1000 agents). No SharedArrayBuffer → no cross-origin-isolation
 * headers needed (which would risk the same-origin model loading). The ~1-tick (33 ms) pipeline latency is
 * invisible for ambient agents and smoothed by the renderer's sub-tick interpolation.
 */

// minimal shape of the generated wasm module (mirrors the `Sim` in crates/worldsim/src/lib.rs)
interface RustSim {
	spawn(x: number, z: number, kindCode: number, radius: number, seedId: number): number;
	set_player(x: number, z: number): void;
	set_companion(i: number): void;
	despawn(i: number): void;
	set_breed_cooldown(i: number, cd: number): void;
	juvenile_cd(): number;
	birth_count(): number;
	births_ptr(): number;
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

// the main thread sends one of these; we reply with 'ready' / 'failed' / 'snap'
type Spawn = { slot: number; x: number; z: number; code: number; radius: number; seedId: number; companion: boolean; juvenile: boolean };
type InMsg =
	| { type: 'init'; base: string; obstacles: Float64Array | null }
	| { type: 'obstacles'; flat: Float64Array }
	| { type: 'tick'; seq: number; dt: number; px: number; pz: number; night: number; fish: Float64Array; spawns: Spawn[]; despawns: number[] };

let wasm: WasmExports | null = null;
let sim: RustSim | null = null;

// typed-array views over wasm memory, rebuilt when the buffer detaches (grows) or the agent count changes
let viewBuf: ArrayBuffer | null = null;
let viewCount = -1;
let xs = new Float32Array();
let zs = new Float32Array();
let headings = new Float32Array();
let healths = new Float32Array();
let flags = new Uint32Array();
let behaviors = new Uint8Array();
let progress = new Float32Array();

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

const ctx = self as unknown as DedicatedWorkerGlobalScope;

ctx.onmessage = async (e: MessageEvent<InMsg>) => {
	const d = e.data;

	if (d.type === 'init') {
		try {
			// load the wasm glue from the static path (same module the main thread used to import). @vite-ignore:
			// it's a runtime URL served from static/, not something to bundle.
			const mod = (await import(/* @vite-ignore */ `${d.base}/worldsim/worldsim.js`)) as unknown as WasmModule;
			wasm = await mod.default();
			sim = new mod.Sim();
			if (d.obstacles) sim.set_obstacles(d.obstacles); // anything Scene pushed before we finished loading
			ctx.postMessage({ type: 'ready' });
		} catch (err) {
			ctx.postMessage({ type: 'failed', error: String(err) });
		}
		return;
	}

	if (!sim || !wasm) return;

	if (d.type === 'obstacles') {
		sim.set_obstacles(d.flat);
		return;
	}

	// d.type === 'tick' — apply roster changes + inputs, advance one fixed step, post the snapshot back
	for (const s of d.spawns) {
		const idx = sim.spawn(s.x, s.z, s.code, s.radius, s.seedId);
		if (idx !== s.slot) console.warn('[worldsim.worker] slot desync: rust', idx, '!= predicted', s.slot);
		if (s.companion) sim.set_companion(idx);
		if (s.juvenile) sim.set_breed_cooldown(idx, sim.juvenile_cd()); // a newborn → must mature before it breeds
	}
	for (const slot of d.despawns) sim.despawn(slot);

	sim.set_player(d.px, d.pz);
	sim.set_night(d.night);
	sim.set_fish(d.fish);
	sim.step(d.dt);

	// copy the read-back into owned buffers (slice → detached from wasm memory) so we can TRANSFER them
	refreshViews();
	const sx = xs.slice();
	const sz = zs.slice();
	const sh = headings.slice();
	const shp = healths.slice();
	const sf = flags.slice();
	const sb = behaviors.slice();
	const sp = progress.slice();
	const nb = sim.birth_count();
	const births = nb > 0 ? new Float32Array(wasm.memory.buffer, sim.births_ptr(), nb * 3).slice() : new Float32Array(0);

	ctx.postMessage(
		{ type: 'snap', seq: d.seq, count: viewCount, xs: sx, zs: sz, headings: sh, healths: shp, flags: sf, behaviors: sb, progress: sp, births, danger: sim.danger() },
		[sx.buffer, sz.buffer, sh.buffer, shp.buffer, sf.buffer, sb.buffer, sp.buffer, births.buffer]
	);
};
