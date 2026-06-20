// Per-agent RENDER STATE + secondary-motion helper for ambient critters (cats) and NPCs (people). The
// actual locomotion sim (the Reynolds steering + behaviour FSM that used to live here) is now the Rust/WASM
// core (crates/worldsim) — see the `rust-owns-all-compute` memory. rustSim writes the Rust read-back
// (x/z/heading, + derived speed/turnRate) onto each Agent every tick; this class just HOLDS that pose and
// interpolates it to the display rate. Pure logic, no Svelte/three — components own the meshes.

const TAU = Math.PI * 2;
const rand = (a = 0, b = 1) => a + Math.random() * (b - a);

/**
 * Frame-rate-independent damped spring — the "secondary motion" workhorse (lean into turns, body
 * bob, tail follow-through, head look). A drop-in upgrade over lerp/tween: eases in, and (with
 * damping < 1) overshoots and settles like real mass. `step` integrates toward `target`.
 */
export class Spring {
	value: number;
	vel = 0;
	target: number;
	stiffness: number; // chase speed (higher = snappier)
	damping: number; // 1 = critical (no overshoot); < 1 = bouncy follow-through

	constructor(value = 0, stiffness = 9, damping = 0.85) {
		this.value = value;
		this.target = value;
		this.stiffness = stiffness;
		this.damping = damping;
	}

	step(dt: number, target = this.target): number {
		this.target = target;
		const k = this.stiffness;
		const c = 2 * Math.sqrt(k) * this.damping; // damping coeff from ratio
		const h = Math.min(dt, 0.05); // clamp for stability on frame spikes
		const a = -k * (this.value - this.target) - c * this.vel;
		this.vel += a * h;
		this.value += this.vel * h;
		return this.value;
	}
}

export type Behavior = 'wander' | 'pause' | 'lookAround' | 'sit' | 'groom' | 'pounce';

export interface AgentOpts {
	maxSpeed?: number;
	homeRadius?: number; // leash distance from home before steering back
	wanderRate?: number; // heading jitter (rad/s)
	accel?: number; // how hard it chases desired velocity (responsiveness)
	turnSpeed?: number; // how fast heading swings toward velocity
	wanderlust?: number; // chance (0..1) this agent is a far-roaming "explorer"
}

/**
 * An agent's pose on the XZ plane, plus render interpolation. The Rust sim owns the motion: rustSim writes
 * `x`/`z`/`heading` from the WASM read-back each tick and derives `speed`/`turnRate` from the per-tick delta;
 * the components read `rx`/`rz`/`rh` (interpolated), `gaitRate()`, `turnRate`, `behavior` and `progress` to
 * drive locomotion. `AgentOpts` is still passed at spawn (maxSpeed scales the gait; the rest mirror the Rust
 * `opts_for` config) but the Rust core, not this class, acts on it.
 */
export class Agent {
	x: number;
	z: number;
	heading: number; // facing angle; model nose is +Z, so group.rotation.y = heading
	speed = 0; // planar speed (m/s) — derived by rustSim each tick → drives gaitRate()
	turnRate = 0; // signed yaw rate (rad/s) → banking / tail lag — derived by rustSim each tick

	// RENDER INTERPOLATION — the sim steps at a fixed 30 Hz, but renderers refresh at the display rate.
	// savePrev() snapshots the pre-step pose; interpolate(alpha) blends prev→current by the clock's sub-tick
	// fraction into rx/rz/rh, so motion is smooth at any FPS (no 30 Hz stutter). No alloc.
	prevX: number;
	prevZ: number;
	prevHeading: number;
	rx = 0;
	rz = 0;
	rh = 0;

	hx: number; // home / leash centre (companion-pet follow target — the Rust port reads this in Phase C)
	hz: number;

	behavior: Behavior = 'wander'; // pose hint for the renderers (idle FSM not yet fed back from Rust)

	readonly maxSpeed: number; // scales gaitRate()

	constructor(x: number, z: number, opts: AgentOpts = {}) {
		this.x = x;
		this.z = z;
		this.hx = x;
		this.hz = z;
		this.heading = rand(0, TAU);
		this.maxSpeed = opts.maxSpeed ?? 2.4;
		this.prevX = x;
		this.prevZ = z;
		this.prevHeading = this.heading;
	}

	/** Snapshot the current pose as the interpolation "previous" — call once per sim step, BEFORE the read-back. */
	savePrev(): void {
		this.prevX = this.x;
		this.prevZ = this.z;
		this.prevHeading = this.heading;
	}

	/** Blend prev→current pose by `alpha` (0..1, the clock's sub-tick fraction) into rx/rz/rh for rendering. */
	interpolate(alpha: number): void {
		this.rx = this.prevX + (this.x - this.prevX) * alpha;
		this.rz = this.prevZ + (this.z - this.prevZ) * alpha;
		let dh = this.heading - this.prevHeading; // shortest-arc so a ±π wrap doesn't spin the model
		while (dh > Math.PI) dh -= TAU;
		while (dh < -Math.PI) dh += TAU;
		this.rh = this.prevHeading + dh * alpha;
	}

	/** 0..1 progress through the current behaviour (drives pounce arcs, groom cycles, …). Static until the Rust
	 *  read-back feeds idle-behaviour timing (Phase C) — kept so the renderers stay source-compatible. */
	get progress(): number {
		return 0;
	}

	/** Move the leash centre (e.g. keep a critter loosely near the player) — the companion-pet follow. */
	setHome(x: number, z: number): void {
		this.hx = x;
		this.hz = z;
	}

	/** Normalised gait drive (0..1) so locomotion swings faster the quicker the agent moves. */
	gaitRate(): number {
		return this.speed / this.maxSpeed;
	}
}
