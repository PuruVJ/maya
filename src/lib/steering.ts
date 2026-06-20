// Reusable "alive movement" core for ambient critters (cat) and NPCs (people). Three layers, per
// Craig Reynolds: action-selection (a tiny behaviour FSM) → steering (vehicle-model forces produce
// life-like paths) → locomotion (the components turn agent state into procedural transforms). Pure
// logic, no Svelte/three — components own the meshes. See docs/npc-movement.md.

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
 * An autonomous agent moving on the XZ plane. `update` runs the behaviour FSM, computes a steering
 * force (desired − current velocity), integrates, and turns the heading to follow velocity. The
 * component reads `speed`, `heading`, `turnRate`, `behavior` and `progress` to drive locomotion.
 */
export class Agent {
	x: number;
	z: number;
	vx = 0;
	vz = 0;
	heading: number; // facing angle; model nose is +Z, so group.rotation.y = heading
	speed = 0; // planar speed (m/s)
	turnRate = 0; // signed yaw rate (rad/s) → banking / tail lag

	hx: number; // home (leash centre)
	hz: number;
	wanderAngle: number;

	behavior: Behavior = 'wander';
	private elapsed = 0;
	private duration = 3;

	readonly maxSpeed: number; // public so the flock manager can scale forces to it
	private readonly homeRadius: number;
	private readonly wanderRate: number;
	private readonly accel: number;
	private readonly turnSpeed: number;
	readonly explorer: boolean; // a small fraction roam off to far-flung places
	personality: number; // 0..1 — varies cadence/speed so agents differ

	constructor(x: number, z: number, opts: AgentOpts = {}) {
		this.x = x;
		this.z = z;
		this.hx = x;
		this.hz = z;
		this.heading = rand(0, TAU);
		this.wanderAngle = this.heading;
		this.maxSpeed = opts.maxSpeed ?? 2.4;
		this.homeRadius = opts.homeRadius ?? 24;
		this.wanderRate = opts.wanderRate ?? 2.4;
		this.accel = opts.accel ?? 7;
		this.turnSpeed = opts.turnSpeed ?? 6;
		this.explorer = rand() < (opts.wanderlust ?? 0.14);
		this.personality = rand(0.3, 0.85);
		this.duration = rand(2, 5);
	}

	/** 0..1 progress through the current behaviour (drives pounce arcs, groom cycles, …). */
	get progress(): number {
		return Math.min(1, this.elapsed / this.duration);
	}

	/** Move the leash centre (e.g. keep a critter loosely near the player). */
	setHome(x: number, z: number): void {
		this.hx = x;
		this.hz = z;
	}

	private pick(menu: Behavior[]): void {
		// explorers occasionally strike out for a far-off place — relocate the home leash there, so
		// they actually journey across the map (over several wander cycles) and settle, then maybe
		// roam on again. Most agents are homebodies and never do this.
		if (this.explorer && Math.random() < 0.22) {
			const ang = rand(0, TAU);
			const far = rand(70, 200);
			this.hx = this.x + Math.sin(ang) * far;
			this.hz = this.z + Math.cos(ang) * far;
			this.behavior = 'wander';
			this.elapsed = 0;
			this.duration = rand(4, 8);
			return;
		}

		// weighted: heavily favour wander, then pauses, then the flavour behaviours
		const weights: Record<Behavior, number> = {
			wander: 5,
			pause: 2,
			lookAround: 1.4,
			groom: 1,
			sit: 1,
			pounce: 0.8
		};
		let total = 0;
		for (const b of menu) total += weights[b];
		let r = rand(0, total);
		let chosen: Behavior = menu[0];
		for (const b of menu) {
			r -= weights[b];
			if (r <= 0) {
				chosen = b;
				break;
			}
		}
		this.behavior = chosen;
		this.elapsed = 0;
		this.duration =
			chosen === 'wander'
				? rand(3, 7)
				: chosen === 'pounce'
					? rand(0.45, 0.7)
					: rand(1.6, 4.2);
	}

	update(dt: number, menu: Behavior[] = ['wander', 'pause'], flock?: { fx: number; fz: number }, boost = 1, forceMove = false): void {
		this.elapsed += dt;
		// while hunting/fleeing the manager forces continuous movement, so a chaser runs instead of
		// dropping into a sit/groom idle mid-pursuit (and its legs animate as running)
		if (forceMove) this.behavior = 'wander';
		else if (this.elapsed >= this.duration) this.pick(menu);

		// --- steering: desired velocity from the current behaviour ---
		let dvx = 0;
		let dvz = 0;
		let cap = this.maxSpeed;
		const moving = this.behavior === 'wander' || this.behavior === 'pounce';
		if (moving) {
			// Reynolds wander: a target on a circle ahead, nudged a small amount each frame
			this.wanderAngle += rand(-1, 1) * this.wanderRate * dt;
			const ahead = 1.5;
			const cx = this.x + Math.sin(this.heading) * ahead;
			const cz = this.z + Math.cos(this.heading) * ahead;
			let tx = cx + Math.sin(this.wanderAngle);
			let tz = cz + Math.cos(this.wanderAngle);
			// containment — Arrive back toward home when past the leash
			const homeDist = Math.hypot(this.x - this.hx, this.z - this.hz);
			if (homeDist > this.homeRadius) {
				tx = this.hx;
				tz = this.hz;
				this.wanderAngle = Math.atan2(this.hx - this.x, this.hz - this.z);
			}
			const tdx = tx - this.x;
			const tdz = tz - this.z;
			const td = Math.hypot(tdx, tdz) || 1;
			const burst = this.behavior === 'pounce' ? 2.3 : 1;
			cap = this.maxSpeed * burst;
			dvx = (tdx / td) * cap;
			dvz = (tdz / td) * cap;
		}
		// non-moving behaviours → desired 0 → Arrive smoothly to a stop (but flock can still nudge)

		// crowd flocking (separation/cohesion/alignment) is blended into the DESIRED velocity — not
		// straight into vx/vz — so it passes through the accel low-pass below and stays smooth.
		cap *= boost; // predator chase / prey panic-run speed multiplier (1 = normal)
		if (flock) {
			dvx += flock.fx;
			dvz += flock.fz;
			const dmag = Math.hypot(dvx, dvz);
			if (dmag > cap && dmag > 0) {
				dvx = (dvx / dmag) * cap;
				dvz = (dvz / dmag) * cap;
			}
		}

		// steering force = desired − current, applied as acceleration (the life-vs-mechanical fix)
		const k = Math.min(1, this.accel * dt);
		this.vx += (dvx - this.vx) * k;
		this.vz += (dvz - this.vz) * k;
		this.x += this.vx * dt;
		this.z += this.vz * dt;
		this.speed = Math.hypot(this.vx, this.vz);

		// --- heading follows velocity; record signed turn rate for banking / tail lag ---
		if (this.speed > 0.06) {
			const desired = Math.atan2(this.vx, this.vz);
			let dh = desired - this.heading;
			while (dh > Math.PI) dh -= TAU;
			while (dh < -Math.PI) dh += TAU;
			const turn = dh * Math.min(1, this.turnSpeed * dt);
			this.heading += turn;
			this.turnRate = turn / Math.max(dt, 1e-3);
		} else {
			this.turnRate *= Math.max(0, 1 - 4 * dt);
		}
	}

	/** Normalised gait drive (0..1) so locomotion swings faster the quicker the agent moves. */
	gaitRate(): number {
		return this.speed / this.maxSpeed;
	}
}
