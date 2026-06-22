/// <reference lib="webworker" />
/**
 * The Rust/WASM world-sim, running OFF the main thread (perf foundation 2/3 — see the `perf-foundation-plan`
 * memory). The engine is unchanged; we just moved WHERE it runs: the wasm + the `Sim` live here, in a dedicated
 * worker, so stepping 1000 agents no longer steals frame time from render. The main thread (rustSim.ts) drives
 * this with one `tick` message per sim tick and renders the snapshot we post back.
 *
 * The main thread owns a stable-slot free-list. It tells us which slot to retire and which slot a new spawn
 * should occupy; Rust's `spawn_at` fills that tombstone (or appends at the high-water mark). No async round-trip
 * is needed to learn indices, and endless birth/death churn no longer grows the WASM vectors forever.
 *
 * Snapshot transport: each step we COPY the read-back arrays (slice → owned buffers) and TRANSFER them to the
 * main thread (zero-copy handoff; ~28 KB for 1000 agents). No SharedArrayBuffer → no cross-origin-isolation
 * headers needed (which would risk the same-origin model loading). The ~1-tick (33 ms) pipeline latency is
 * invisible for ambient agents and smoothed by the renderer's sub-tick interpolation.
 */

// minimal shape of the generated wasm module (mirrors the `Sim` in crates/worldsim/src/lib.rs)
interface RustSim {
	spawn(x: number, z: number, kindCode: number, radius: number, seedId: number): number;
	spawn_at(slot: number, x: number, z: number, kindCode: number, radius: number, seedId: number): number;
	set_player(x: number, z: number): void;
	set_companion(i: number): void;
	despawn(i: number): void;
	set_breed_cooldown(i: number, cd: number): void;
	set_gene(i: number, gene: number): void;
	juvenile_cd(): number;
	birth_count(): number;
	births_ptr(): number;
	build_count(): number;
	builds_ptr(): number;
	well_count(): number;
	wells_ptr(): number;
	event_count(): number;
	events_ptr(): number;
	set_night(n: number): void;
	set_pop_scale(s: number): void;
	set_fish(xz: Float64Array): void;
	set_refuges(xz: Float64Array): void;
	set_water(xzr: Float64Array): void;
	set_aridity(a: number): void;
	set_obstacles(flat: Float64Array): void;
	set_behavior_mode(code: number): void; // 0 = Manual (hand-coded) · 1 = Emergent (needs+utility, the default)
	age_means(): Float32Array; // mean age fraction (0..1) per kind → the HUD age readout
	set_player_immune(immune: number): void;
	set_lineage(i: number, pfamA: number, pfamB: number): void; // newborn's parent lineage ids → incest avoidance
	set_genome(i: number, food: number, safety: number, social: number, rest: number, industry: number): void; // inherited behaviour genome
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
type Spawn = { slot: number; x: number; z: number; code: number; radius: number; seedId: number; companion: boolean; juvenile: boolean; gene: number; pfamA: number; pfamB: number; genome: number[] | null };
type InMsg =
	| { type: 'init'; base: string; obstacles: Float64Array | null }
	| { type: 'obstacles'; flat: Float64Array }
	| { type: 'refuges'; xz: Float64Array }
	| { type: 'water'; xzr: Float64Array }
	| { type: 'aridity'; a: number }
	| { type: 'behaviorMode'; code: number }
	| { type: 'tick'; seq: number; dt: number; px: number; pz: number; night: number; popScale: number; fish: Float64Array; spawns: Spawn[]; despawns: number[] };

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

	if (d.type === 'refuges') {
		sim.set_refuges(d.xz);
		return;
	}

	if (d.type === 'water') {
		sim.set_water(d.xzr);
		return;
	}

	if (d.type === 'aridity') {
		sim.set_aridity(d.a);
		return;
	}

	if (d.type === 'behaviorMode') {
		sim.set_behavior_mode(d.code); // 0 Manual · 1 Emergent — live A/B of the decision brain
		return;
	}

	// d.type === 'tick' — apply roster changes + inputs, advance one fixed step, post the snapshot back
	// Retire old occupants before filling slots recycled in this same roster diff.
	for (const slot of d.despawns) sim.despawn(slot);
	for (const s of d.spawns) {
		const idx = sim.spawn_at(s.slot, s.x, s.z, s.code, s.radius, s.seedId);
		if (idx !== s.slot) console.warn('[worldsim.worker] slot desync: rust', idx, '!= predicted', s.slot);
		if (s.companion) sim.set_companion(idx);
		if (s.juvenile) sim.set_breed_cooldown(idx, sim.juvenile_cd()); // a newborn → must mature before it breeds
		if (s.gene !== 1) sim.set_gene(idx, s.gene); // a bred baby's inherited vigor (genetics)
		if (s.pfamA || s.pfamB) sim.set_lineage(idx, s.pfamA, s.pfamB); // a bred baby's parentage → incest avoidance
		if (s.genome) sim.set_genome(idx, s.genome[0], s.genome[1], s.genome[2], s.genome[3], s.genome[4]); // inherited genome
	}

	sim.set_player(d.px, d.pz);
	sim.set_night(d.night);
	sim.set_pop_scale(d.popScale);
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
	const births = nb > 0 ? new Float32Array(wasm.memory.buffer, sim.births_ptr(), nb * 11).slice() : new Float32Array(0); // [kc,x,z,gene,momFam,dadFam,g0..g4]×nb
	const nbd = sim.build_count();
	const builds = nbd > 0 ? new Float32Array(wasm.memory.buffer, sim.builds_ptr(), nbd * 2).slice() : new Float32Array(0); // [x,z]×nbd
	const nw = sim.well_count();
	const wells = nw > 0 ? new Float32Array(wasm.memory.buffer, sim.wells_ptr(), nw * 2).slice() : new Float32Array(0); // [x,z]×nw
	const ne = sim.event_count();
	const events = ne > 0 ? new Float32Array(wasm.memory.buffer, sim.events_ptr(), ne * 4).slice() : new Float32Array(0); // [code,kind,x,z]×ne

	const ageMeans = sim.age_means(); // 6 floats — mean age fraction per kind (HUD age readout); tiny, not transferred
	ctx.postMessage(
		{ type: 'snap', seq: d.seq, count: viewCount, xs: sx, zs: sz, headings: sh, healths: shp, flags: sf, behaviors: sb, progress: sp, births, builds, wells, events, danger: sim.danger(), ageMeans },
		[sx.buffer, sz.buffer, sh.buffer, shp.buffer, sf.buffer, sb.buffer, sp.buffer, births.buffer, builds.buffer, wells.buffer, events.buffer]
	);
};
