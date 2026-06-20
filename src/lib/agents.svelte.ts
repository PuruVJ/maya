// Central manager for all ambient agents (critters + people). ONE per-frame tick (driven by the
// headless AgentSystem component) rebuilds a spatial-hash grid, runs the FOOD-CHAIN simulation
// (who hunts whom, stamina, catch→death), computes Reynolds flocking, steps each Agent, and assigns
// per-agent perf flags (LOD + shadow budget). Components register/unregister and only READ state to
// drive locomotion + death/sleep poses. Deliberately NOT reactive in the hot path — these objects
// are mutated 60×/s; making them $state would cause render storms. See docs/npc-movement.md.
import { Agent, type Behavior } from './steering';
import { SpatialHashGrid } from './spatialhash';
import { playerState } from './playerState.svelte';
import { forEachTreeNear, treeRadius, onPath } from './scatter';
import { fishRegistry } from './fish.svelte';
import { Rng } from './rng';
import type { Path } from './world';

// Deterministic sim RNG (docs/self-sustaining-world.md §1.6): every per-individual draw is keyed by the
// agent's stable `seedId` + a CHANNEL (so two draws at the same coordinate don't correlate) → spawns are
// reproducible (same obj.id ⇒ same traits, so a shared world agrees). BIRTH-TIME rolls (speed/aggro/slash)
// key by seedId only; per-tick rolls (wander/fight) will also key by the clock tick once the sim is
// clock-driven (that migration step is still pending). Base seed is fixed for now — swap to world.seed later.
const rng = new Rng('worldgen-agents');
const CH = { wander: 1, speed: 2, aggro: 3, slash: 4, breed: 5, mutate: 6, fight: 7 } as const;

// ── Food chain ───────────────────────────────────────────────────────────────────────────────────
// rank = trophic level (higher eats lower). speed = random range (per individual; big hunters are
// generally a touch slower than small critters, so chases can fail). endurance = how long it can
// sprint before tiring (small critters high → outlast a hunter; big hunters low → must sleep).
// hunts: 'lower' = eats anything below its rank · 'humans' = (people) attacks own kind on a coinflip
// · 'none' = pure prey.
// fullAfter (hunters only) = how many kills before a food-coma sleep; sleepSecs = how long that proper
// sleep lasts. Distinct per predator: a lion gorges over several kills then sleeps it off; the t-rex has
// a far bigger appetite and sleeps even longer. Omitted kinds never get a satiation coma (they only ever
// sleep from sheer exhaustion, which still uses sleepSecs for its duration).
export const ECO: Record<
	string,
	{
		rank: number;
		speed: [number, number];
		endurance: number;
		hunts: 'lower' | 'humans' | 'none';
		fullAfter?: number;
		sleepSecs?: number;
		mobToll?: [number, number]; // how many attackers it slashes dead while being mobbed before it falls
	}
> = {
	rabbit: { rank: 1, speed: [3.6, 4.8], endurance: 1.0, hunts: 'none' },
	cat: { rank: 2, speed: [3.0, 3.9], endurance: 0.8, hunts: 'lower', sleepSecs: 10, mobToll: [1, 2] }, // hunts the rabbit
	kangaroo: { rank: 2, speed: [3.4, 4.6], endurance: 0.9, hunts: 'none' },
	person: { rank: 3, speed: [1.8, 2.5], endurance: 0.6, hunts: 'humans' }, // amble; sprint when fleeing
	lion: { rank: 4, speed: [3.0, 3.9], endurance: 0.4, hunts: 'lower', fullAfter: 5, sleepSecs: 16, mobToll: [1, 3] },
	dinosaur: { rank: 5, speed: [4.8, 6.2], endurance: 0.3, hunts: 'lower', fullAfter: 9, sleepSecs: 24, mobToll: [2, 5] } // apex — fastest, biggest appetite, longest sleep, deadliest when cornered
};
const DEFAULT_SLEEP_SECS = 10; // fallback rest length if a kind has no explicit sleepSecs

// How desirable each kind is as PREY — a hunter is drawn more to a big, meaty (and the slow, helpless)
// target than to a small rabbit. Weighed against distance below: a much closer lesser prey can still
// win. (T-rex: prefers a human over a farther lion, but grabs whatever is right next to it.)
const PREY_PRIZE: Record<string, number> = {
	rabbit: 0.7,
	cat: 1.0,
	kangaroo: 1.4,
	lion: 1.8,
	person: 2.0,
	dinosaur: 2.6
};
const prizeOf = (k: string): number => PREY_PRIZE[k] ?? 1;
const sleepSecs = (k: string): number => ECO[k]?.sleepSecs ?? DEFAULT_SLEEP_SECS;
const AGGRO_PROB = 0.2; // share of people that turn aggressive (hunt their own kind)

/** A random max speed in this kind's range (varies every individual). */
export function speedFor(kind: string, seedId: number): number {
	const s = (ECO[kind] ?? ECO.cat).speed;
	return rng.range(s[0], s[1], seedId, CH.speed); // deterministic per individual (was Math.random)
}

export interface ManagedAgent {
	agent: Agent;
	kind: string; // 'rabbit' | 'cat' | 'kangaroo' | 'person' | 'lion' | 'dinosaur'
	radius: number; // body radius → personal space + catch contact
	menu: Behavior[];
	objId?: string; // the world-object id this agent renders (placed animals) → lets the live state be saved
	tint?: string; // the agent's display colour (e.g. a person's hashed shirt) → far impostors match the near look
	companion?: boolean; // the player's pet — follows you (home leash tracks the player) and isn't scared off

	// written by the manager each frame, read by the owning component:
	lod: 0 | 1 | 2; // 0 near (full), 1 mid, 2 far (freeze articulation)
	castShadow: boolean; // only the nearest few cast (shadow budget)
	dist: number; // distance to the player
	// ecosystem state (seeded by makeManaged, evolved by the manager; read for poses):
	rank: number;
	endurance: number;
	aggressive: boolean; // people only — hunts its own kind
	seedId: number; // stable per-agent uint32 → its own deterministic RNG stream (traits now; offspring later)
	stamina: number; // 0..1; drains while sprinting, recovers at rest
	health: number; // 0..1; mobs & lost scraps chip it down; ≤0 = death; <HURT_AT = injured/limping
	meals: number; // kills since last sleep — reaching ECO.fullAfter triggers a food-coma nap
	spooked: number; // seconds left fleeing a recent attacker (bully) and refusing to sleep
	mobbed: boolean; // LATCHED (hysteresis): being swarmed → flee the mob; holds until it thins out
	dead: boolean; // caught → corpse, lies where it fell
	asleep: boolean; // hunter resting (food-coma after eating its fill, or sheer exhaustion)
	sleepTimer: number; // seconds left in the current sleep → a proper, full rest (not a blink)
	// ENERGY / METABOLISM (carnivores): `stamina` IS the energy unit — a hunter's ebbs constantly (BASAL_DRAIN)
	// and is refilled only by EATING a kill or SLEEPING, so it hunts when hungry and gives up a chase gone too far.
	chaseOX: number; // where the current prey-chase began → give up once chased MAX_CHASE from here (NaN = none)
	chaseOZ: number;
	giveUpCd: number; // seconds it rests (won't re-acquire prey) after abandoning a chase
	// transient per-tick targeting (manager-internal):
	prey?: ManagedAgent | null;
	threat?: ManagedAgent | null;
	bully?: ManagedAgent | null; // who last hurt it → flees this one while spooked
	preyD?: number;
	threatD?: number;
	mobCount?: number; // prey currently fleeing THIS agent → ≥MOB_MIN flips them to swarm it
	mobX?: number; // running sum of those mobbers' positions → their centroid (for break-away)
	mobZ?: number;
	attackers?: number; // mobbers actually pressed into CONTACT this tick → they wound it; it slashes back
	huntedBy?: number; // predators that have CLAIMED this prey this tick → competition (extra hunters fan out)
	rival?: ManagedAgent | null; // nearest same-rank apex predator crowding it (transient, per tick)
	rivalD2?: number; // squared dist to that rival (for picking the nearest)
	rivalTime: number; // seconds it's been crowded by a rival → boils over into a territorial fight
	nearPredator?: boolean; // another hunter is close → stay alert (don't lie down to sleep)
	crowd?: number; // flock neighbours this tick → gates player-hunting (a herd wanders, a loner stalks)
	_i?: number; // per-tick live-list index → stable ordering so a peer-fight pair is resolved exactly once
	slashMax: number; // this individual's ferocity — attackers it can slay in one mob fight (from mobToll)
	slashBudget: number; // attackers it can still slash this fight before it's overwhelmed (refills when free)
	slashCd: number; // cooldown until its next retaliatory slash
}

/** Build a fully-seeded managed agent from its kind (so components don't repeat the eco wiring). */
export function makeManaged(agent: Agent, kind: string, radius: number, menu: Behavior[], objId: string | undefined, seedId: number): ManagedAgent {
	const eco = ECO[kind] ?? ECO.cat;
	const toll = eco.mobToll;
	const slashMax = toll ? toll[0] + rng.int(0, toll[1] - toll[0] + 1, seedId, CH.slash) : 0; // deterministic per individual
	return {
		agent,
		kind,
		radius,
		menu,
		objId,
		lod: 0,
		castShadow: true,
		dist: 0,
		rank: eco.rank,
		endurance: eco.endurance,
		aggressive: kind === 'person' && rng.chance(AGGRO_PROB, seedId, CH.aggro), // deterministic per individual
		seedId,
		stamina: eco.hunts === 'lower' ? 0.45 : 1, // carnivores start a touch hungry so they hunt soon
		health: 1,
		meals: 0,
		spooked: 0,
		mobbed: false,
		dead: false,
		asleep: false,
		sleepTimer: 0,
		chaseOX: NaN,
		chaseOZ: NaN,
		giveUpCd: 0,
		slashMax,
		slashBudget: slashMax,
		slashCd: 0,
		rivalTime: 0
	};
}

/** Does `a` hunt `b` right now? (Sleeping/dead agents neither hunt nor threaten.) */
function preysOn(a: ManagedAgent, b: ManagedAgent): boolean {
	if (a === b || a.dead || b.dead || a.asleep) return false;
	const hunts = ECO[a.kind]?.hunts;
	if (hunts === 'lower') return b.rank < a.rank; // cat/lion/dinosaur → anything below
	if (hunts === 'humans') return a.aggressive && b.kind === 'person'; // aggressive person → people
	return false;
}

const NEIGHBOR_RADIUS = 4; // also the grid cell size (flocking only)
const DENSITY_THRESHOLD = 1; // a lone neighbour is cozy; spread ramps in from the 2nd so small
// groups (e.g. 3 people) settle at a comfortable spacing instead of collapsing into a buzzing knot
const SEP_WEIGHT = 1.6;
const ALI_WEIGHT = 0.4;
const LOD1_DIST = 26;
const LOD2_DIST = 62;
const SHADOW_AGENTS = 12;
// predator/prey ranges + forces
const SEEK = 100; // a predator NOTICES + stalks toward the nearest prey within this radius (m)
const SEEK2 = SEEK * SEEK; // ...squared (the widest food-chain interaction range)
const HUNT2 = 34 * 34; // ...and breaks into a sprint for the kill once this close
// prey bolts at 40m — just OUTSIDE the predator's 34m sprint trigger, so it gets a real head start
// instead of only reacting once the hunter is already at a full sprint on top of it.
const DANGER2 = 40 * 40;
const FLEE_W = 2.6; // strong — overrides wander when running for your life
const AVOID_W = 1.8; // gentle "personal space" — every animal steers AROUND the player's body, not through it
const COMPETE_W = 1.2; // how hard a prey's appeal drops per hunter already on it → surplus predators fan out
const MAX_HUNTERS = 3; // a prey already claimed by this many hunters is "full" → extra predators peel off to search
const CHASE_W = 2.0;
const FLEE_BOOST = 1.7; // panic-run speed multiplier
const CHASE_BOOST = 1.45;
// stamina
const EXERT_DRAIN = 0.22; // /s while sprinting, divided by endurance
const RECOVER = 0.16; // /s at rest, multiplied by endurance
const SLEEP_MULT = 2.4; // recover faster while asleep
// energy / metabolism (carnivores only — `stamina` doubles as the energy unit)
const BASAL_DRAIN = 0.02; // /s a carnivore's energy always ebbs → it must eat or sleep to sustain (no idle recovery)
const HUNGRY = 0.55; // a fed carnivore (energy ≥ this) rests; below it, it goes hunting
const EAT_GAIN = 0.6; // a kill (eating) refuels this much energy
const MAX_CHASE2 = 45 * 45; // give up a chase once this far (m) from where it began — a predator won't pursue forever
const GIVEUP_CD = 5; // seconds it rests / won't re-acquire prey after abandoning a chase
const GIVEUP_ENERGY = 0.06; // ...or it abandons the chase early when this spent
const CONTACT_PAD = 0.4; // extra reach that counts as a catch
// mobbing — when prey heavily outnumber one hunter, the herd turns and swarms it instead of fleeing
const MOB_MIN = 4; // this many prey fleeing ONE hunter flips them flee → swarm
const MOB_RELEASE = 3; // hysteresis: a mobbed hunter stays mobbed until the swarm is whittled BELOW this.
// Without the gap, a count hovering at MOB_MIN flips flee↔chase every tick → predator frozen, jittering.
const MOB_W = 2.2; // converge force as the mob charges the predator
const MOB_KILL_DPS = 0.03; // health/s a hunter loses PER attacker pressed against it (so the threshold to
// down it is a COMBINATION of swarm size and its remaining health — many attackers + a hurt hunter = fast)
const SLASH_CD = 1.2; // seconds between a cornered hunter's retaliatory slashes (each kills one attacker)
// combat / injury — damage ONLY comes from a deliberate slash, never from mere contact. A prey mob just
// INTIMIDATES a hunter into breaking away (no damage); a hunter is only wounded when another predator
// notices and slashes it. A wounded animal limps away slower and can be finished off. Health heals slowly.
const HURT_AT = 0.45; // below this health an animal is injured → moves at HURT_SPEED and flees
const HURT_SPEED = 0.6; // injured locomotion multiplier (a limp, so a healthy hunter can run it down)
const FIGHT_R2 = 3 * 3; // two predators closer than this stay alert to each other
const RIVAL_PATIENCE = 5; // seconds two apex predators tolerate crowding before they turn and fight
const RIVAL_DPS = 0.35; // health/s each loses in a territorial scrap → one breaks off wounded (or goes down)
const LURE_R = 11; // a cat within this range of a lake fish is drawn to the bank (the water obstacle stops it dry)
const HEAL = 0.04; // health/s regained while unharmed (full recovery takes ~25s)

const smoothstep = (a: number, b: number, x: number) => {
	const t = Math.max(0, Math.min(1, (x - a) / (b - a)));
	return t * t * (3 - 2 * t);
};

/** A solid world prop the steering sim must route around. Round props use the circle `r`; rectangular ones
 *  (buildings) ALSO carry oriented-box half-extents (hx,hz) + the rotation's cos/sin so animals hug walls /
 *  use streets instead of a fat circle — matching the player's collision. `r` stays the bounding radius so
 *  the obstacle grid's broad-phase still finds boxes. */
export interface Obstacle {
	x: number;
	z: number;
	r: number;
	hx?: number; // box half-extent along local X (present → resolve as an oriented box, not a circle)
	hz?: number; // box half-extent along local Z
	cos?: number; // cos/sin of the box's Y-rotation (precomputed for the hot path)
	sin?: number;
}
const OBSTACLE_CELL = 12; // spatial-hash cell for obstacle lookups — must exceed the biggest footprint +
// body radius so the 3×3 sweep can't miss a large scaled prop (a 3× house's circle is still well under this)

class AgentManager {
	#agents = new Set<ManagedAgent>();
	#grid = new SpatialHashGrid<ManagedAgent>(NEIGHBOR_RADIUS);
	#seekGrid = new SpatialHashGrid<ManagedAgent>(SEEK); // coarse grid for food-chain targeting (cell = SEEK
	// so the 3×3 sweep is guaranteed to find every agent within SEEK → O(N·neighbours), not O(N²))
	#obGrid = new SpatialHashGrid<Obstacle>(OBSTACLE_CELL);
	#hasObstacles = false;
	#shadowScratch: ManagedAgent[] = []; // reused nearest-N buffer for the shadow budget (no per-frame alloc)
	#live: ManagedAgent[] = []; // reused per-tick non-corpse list (reset each tick) → no N-length array alloc/frame
	#flockOut = { fx: 0, fz: 0 }; // reused #flock return (consumed immediately by the caller) → no per-agent alloc
	#paths: Path[] = []; // roads/rivers → ambient trees are culled there, so animals must skip those ghost trunks too
	#lastPx = NaN; // last player pos → per-tick player speed (wildlife scatters harder when you RUN at it)
	#lastPz = NaN;
	#night = 0; // 0 day → 1 night (set from the sky): prey jumpier, predators keener after dark

	/** How nocturnal the world is right now (0 day … 1 night) — drives the food chain's day/night mood. */
	setNight(n: number): void {
		this.#night = Math.max(0, Math.min(1, n));
	}

	/** The world's paths (roads/rivers) — ambient trees are culled on them, so animals skip those ghost trunks. */
	setPaths(paths: Path[]): void {
		this.#paths = paths;
	}

	/** Replace the set of solid props agents must not walk through (call when the world's objects change). */
	setObstacles(obs: Obstacle[]): void {
		this.#obGrid.clear();
		for (const o of obs) this.#obGrid.insert(o.x, o.z, o);
		this.#hasObstacles = obs.length > 0;
	}

	register(m: ManagedAgent): void {
		this.#agents.add(m);
	}

	unregister(m: ManagedAgent): void {
		this.#agents.delete(m);
	}

	/** Iterate all managed agents (used by the far-agent impostor renderer). */
	forEach(cb: (m: ManagedAgent) => void): void {
		for (const m of this.#agents) cb(m);
	}

	/** Current LIVE state of every PLACED animal, keyed by its world-object id → merged into the share link
	 *  so a creature that wandered off / died reopens exactly there. (Ambient ones with no objId are skipped.) */
	liveSnapshot(): Map<string, { x: number; z: number; dead: boolean; asleep: boolean }> {
		const out = new Map<string, { x: number; z: number; dead: boolean; asleep: boolean }>();
		for (const m of this.#agents) {
			if (!m.objId) continue;
			out.set(m.objId, { x: m.agent.x, z: m.agent.z, dead: m.dead, asleep: m.asleep });
		}
		return out;
	}

	tick(dt: number): void {
		const h = Math.min(dt, 0.05); // clamp so a frame spike can't teleport agents
		const px = playerState.pos[0];
		const pz = playerState.pos[2];
		// player speed this tick → a running player scares wildlife from farther away
		const pSpeed = Number.isNaN(this.#lastPx) ? 0 : Math.hypot(px - this.#lastPx, pz - this.#lastPz) / h;
		this.#lastPx = px;
		this.#lastPz = pz;
		// day/night mood: after dark prey are jumpier (flee/notice predators from farther) and predators
		// break into the kill-sprint sooner. Both stay within SEEK2 so the coarse seek grid still covers them.
		const night = this.#night;
		const danger2 = DANGER2 * (1 + 0.5 * night);
		const hunt2 = HUNT2 * (1 + 0.4 * night);
		let dangerNow = 0; // peak imminence of any predator hunting the player this tick → danger vignette

		// 1. neighbour grid (flocking) + coarse seek grid (targeting) + the live (non-corpse) list,
		// resetting per-tick targeting
		this.#grid.clear();
		this.#seekGrid.clear();
		const live = this.#live; // reused buffer (reset, not reallocated) → no per-frame N-length array churn at scale
		live.length = 0;
		for (const m of this.#agents) {
			if (m.dead) continue; // corpses don't move, flock, hunt or flee
			this.#grid.insert(m.agent.x, m.agent.z, m);
			this.#seekGrid.insert(m.agent.x, m.agent.z, m);
			m._i = live.length; // stable index for the once-per-pair peer-fight gate
			m.prey = null;
			m.threat = null;
			m.preyD = 0; // now a SCORE (prize/dist²) — pick the MAX, not the nearest
			m.threatD = danger2;
			m.mobCount = 0;
			m.mobX = 0;
			m.mobZ = 0;
			m.attackers = 0;
			m.huntedBy = 0; // competition tally — rebuilt this tick by sequential prey-claiming below
			m.rival = null;
			m.rivalD2 = Infinity;
			m.nearPredator = false;
			live.push(m);
		}

		// 2. food-chain targeting via the coarse seek grid — each agent scans only its ~3×3-cell neighbours
		// (cell = SEEK, so nothing in range is missed) instead of all N. Each agent handles itself AS THE
		// PREDATOR (picks its best prey + marks its prey's threat), so every directed predator→prey relation
		// is resolved exactly once — b's own pass covers b-as-predator. The symmetric peer-fight is gated by
		// `_i` so a pair clashes once, not twice. Scales to the "scatter 1000 animals" case (was O(N²) → 1M
		// pairs/frame); worst case is still quadratic only if everything piles into one SEEK cell.
		for (const a of live) {
			const aHunts = ECO[a.kind]?.hunts === 'lower';
			// metabolism gate: a fed carnivore (energy ≥ HUNGRY), or one cooling off from a chase it just gave up,
			// doesn't go looking for prey (and so isn't a threat that scares prey). Aggressive people ignore this.
			const aSeeks = aHunts ? a.stamina < HUNGRY && a.giveUpCd <= 0 : true;
			this.#seekGrid.forEachNeighbor(a.agent.x, a.agent.z, (b) => {
				if (b === a) return;
				const dx = a.agent.x - b.agent.x;
				const dz = a.agent.z - b.agent.z;
				const d2 = dx * dx + dz * dz;
				if (d2 > SEEK2) return; // outside the notice range (hash-collision false neighbour)
				if (aSeeks && preysOn(a, b)) {
					// size- & proximity-weighted score, DISCOUNTED by how many hunters already claimed b this
					// tick (sequential claiming below) → a crowded prey looks worse, so later predators prefer a
					// freer target and the pack naturally spreads across prey instead of all dogpiling one.
					const s = prizeOf(b.kind) / (Math.max(d2, 1) * (1 + COMPETE_W * (b.huntedBy ?? 0)));
					if (s > a.preyD!) ((a.prey = b), (a.preyD = s));
					if (d2 < danger2 && d2 < b.threatD!) ((b.threat = a), (b.threatD = d2)); // b fears its nearest hunter
				}
				// two peer predators close together just stay ALERT (so they don't doze off in a crowd). They
				// do NOT fight to the death — packing 100 same-rank predators used to be a bloodbath; now they
				// simply mill and wander. (Lethal peer-infight removed deliberately.)
				if (aHunts && d2 < FIGHT_R2 && ECO[b.kind]?.hunts === 'lower') {
					a.nearPredator = b.nearPredator = true;
					// same-rank apex predators (dino↔dino, lion↔lion) are RIVALS — track the nearest so prolonged
					// crowding can boil over into a territorial fight (they don't pack). Cross-rank is predation.
					if (a.rank >= 4 && a.rank === b.rank && d2 < (a.rivalD2 ?? Infinity)) ((a.rival = b), (a.rivalD2 = d2));
				}
			});
			// CLAIM the chosen prey — but if it already has enough hunters, this predator gives up and FANS OUT
			// (prey=null → it wanders off to search for its own kill) instead of joining a pointless pile. Earlier
			// predators in the list claim first, so the surplus consistently peels away. A few hunters can still
			// gang a tough target; the cap just stops 20 dinos chasing one rabbit.
			if (a.prey) {
				if ((a.prey.huntedBy ?? 0) >= MAX_HUNTERS) ((a.prey = null), (a.preyD = 0));
				else a.prey.huntedBy = (a.prey.huntedBy ?? 0) + 1;
			}
			// CHASE THRESHOLD — a carnivore abandons prey it's chased too far from where the chase began (or that
			// it's now too spent to catch), then cools off (GIVEUP_CD) before hunting again → never pursues forever.
			if (aHunts) {
				if (a.prey) {
					if (Number.isNaN(a.chaseOX)) ((a.chaseOX = a.agent.x), (a.chaseOZ = a.agent.z)); // chase begins here
					const far = (a.agent.x - a.chaseOX) ** 2 + (a.agent.z - a.chaseOZ) ** 2 > MAX_CHASE2;
					if (far || a.stamina < GIVEUP_ENERGY) ((a.prey = null), (a.preyD = 0), (a.giveUpCd = GIVEUP_CD), (a.chaseOX = NaN));
				} else {
					a.chaseOX = NaN; // not chasing → reset the origin for the next hunt
				}
			}
		}

		// 2b. mobbing tally — how many prey flee each hunter + the sum of their positions, so a hunter
		// knows when it's outnumbered (≥MOB_MIN) and the herd can converge on the mob's centroid.
		for (const m of live) {
			const t = m.threat;
			if (!t) continue;
			t.mobCount = (t.mobCount ?? 0) + 1;
			t.mobX = (t.mobX ?? 0) + m.agent.x;
			t.mobZ = (t.mobZ ?? 0) + m.agent.z;
			// a mobber pressed into contact is actively attacking → it both wounds the hunter and is a
			// candidate to be slashed back
			const dx = m.agent.x - t.agent.x;
			const dz = m.agent.z - t.agent.z;
			const reach = m.radius + t.radius + CONTACT_PAD + 0.4;
			if (dx * dx + dz * dz < reach * reach) t.attackers = (t.attackers ?? 0) + 1;
		}

		// 2c. latch the mobbed state with hysteresis — set at MOB_MIN, released only below MOB_RELEASE — so a
		// count sitting at the boundary can't flip the hunter flee↔chase every tick (which froze it in place).
		for (const m of live) {
			const c = m.mobCount ?? 0;
			if (c >= MOB_MIN) m.mobbed = true;
			else if (c < MOB_RELEASE) m.mobbed = false;
		}

		// 3. per-agent: death → sleep → behaviour (mob break-away / spook-flee / flee / hunt) → step → LOD
		for (const m of live) {
			if (m.dead) continue; // may have just been caught this tick
			const a = m.agent;

			// a slash that emptied the health bar is fatal; otherwise wounds knit back slowly
			if (m.health <= 0) {
				m.dead = true;
				m.asleep = false;
				a.vx = a.vz = 0;
				m.dist = Math.hypot(a.x - px, a.z - pz);
				m.lod = m.dist > LOD2_DIST ? 2 : m.dist > LOD1_DIST ? 1 : 0;
				continue;
			}
			m.health = Math.min(1, m.health + HEAL * h);
			if (m.spooked > 0) m.spooked -= h;
			if (m.giveUpCd > 0) m.giveUpCd -= h; // cooling off after abandoning a chase

			// asleep: run the rest timer down (recovering stamina); wake after a PROPER sleep — or instantly
			// if a peer wanders up, a hunter closes in, it gets spooked, or the PLAYER blunders too close (then
			// it stays awake and bolts). The wake radius scales with player speed: you can tiptoe within ~1.5 m
			// of a sleeper, but SPRINT at it and it startles from farther → sneaking past finally means something.
			if (m.asleep) {
				const wakeR = Math.min(7, 1.5 + Math.max(0, pSpeed - 3) * 0.45);
				const pdx = a.x - px;
				const pdz = a.z - pz;
				const playerWoke = !m.companion && pdx * pdx + pdz * pdz < wakeR * wakeR;
				const disturbed = m.nearPredator || !!m.threat || m.spooked > 0 || playerWoke;
				m.sleepTimer -= h;
				m.stamina = Math.min(1, m.stamina + RECOVER * m.endurance * SLEEP_MULT * h);
				a.vx *= Math.max(0, 1 - 3 * h);
				a.vz *= Math.max(0, 1 - 3 * h);
				a.x += a.vx * h;
				a.z += a.vz * h;
				a.speed = Math.hypot(a.vx, a.vz);
				if (m.sleepTimer <= 0 || disturbed) {
					m.asleep = false;
					m.meals = 0;
					if (playerWoke && m.rank < 4) m.spooked = Math.max(m.spooked, 1.0); // prey startles awake → bolts
				}
				this.#resolveObstacles(m);
				this.#resolveTrees(m);
				m.dist = Math.hypot(a.x - px, a.z - pz);
				m.lod = m.dist > LOD2_DIST ? 2 : m.dist > LOD1_DIST ? 1 : 0;
				continue;
			}

			const flock = this.#flock(m, px, pz);
			let boost = 1;
			let pursuing = false; // forces continuous movement (running, not idling) while in a chase
			const canSprint = m.stamina > 0.03;
			const mobbed = m.mobbed; // LATCHED with hysteresis (set ≥MOB_MIN, clears <MOB_RELEASE)
			if (!mobbed) ((m.slashBudget = m.slashMax), (m.slashCd = 0)); // fresh ferocity for the next fight

			// big predators (lion/dino) HUNT the player — non-lethal: they pressure + bump you, burn stamina
			// on the sprint, then tire out and give up. Keener + farther-reaching at night. Only when not
			// otherwise busy and the player is clearly closer than any animal prey.
			let huntPlayer = false;
			// a LONE big predator stalks you; one in a pack (crowd) just wanders — so spawning 100 dinos
			// doesn't turn into a swarm piling onto the player
			if (m.rank >= 4 && (m.crowd ?? 0) < 3 && ECO[m.kind]?.hunts === 'lower' && !mobbed && m.spooked <= 0 && !m.threat && canSprint && m.stamina < HUNGRY) {
				const dP2 = (px - a.x) ** 2 + (pz - a.z) ** 2;
				const reach = 15 * (1 + 0.6 * night); // night → hunts you from farther
				const preyD2 = m.prey ? (m.prey.agent.x - a.x) ** 2 + (m.prey.agent.z - a.z) ** 2 : Infinity;
				if (dP2 < reach * reach && dP2 < preyD2 * 0.81) {
					huntPlayer = true; // only if clearly closer (0.9²)
					dangerNow = Math.max(dangerNow, 1 - Math.sqrt(dP2) / reach); // closer hunter → louder alarm
				}
			}

			// territorial timer: apex predators don't pack — accumulate time spent crowded by a rival, cooling
			// off quickly once apart. Once it boils over they pick a fight (handled in the chain below).
			if (m.rival && !m.rival.dead) m.rivalTime = Math.min(RIVAL_PATIENCE + 0.5, m.rivalTime + h);
			else m.rivalTime = Math.max(0, m.rivalTime - h * 1.5);
			const fightingRival = !!m.rival && !m.rival.dead && m.rivalTime >= RIVAL_PATIENCE && !mobbed && !m.threat;

			// cats are drawn to LAKE FISH: a cat with no better business and a fish within range pads to the
			// water's edge after it. It never catches one — the pond is an obstacle, so the cat stops at the bank
			// (the resolve-obstacles step below holds it on dry land) and just stalks the shallows. Emergent fishing.
			let fishX = 0;
			let fishZ = 0;
			let fishLuring = false;
			if (m.kind === 'cat' && !mobbed && !m.threat && !fightingRival && m.spooked <= 0 && !m.prey) {
				const f = fishRegistry.nearest(a.x, a.z, LURE_R);
				if (f) ((fishLuring = true), (fishX = f.x), (fishZ = f.z));
			}

			if (mobbed) {
				// abandon the hunt and BREAK AWAY from the swarm's centre — a fast hunter shakes them off
				const dx = a.x - m.mobX! / m.mobCount!;
				const dz = a.z - m.mobZ! / m.mobCount!;
				const d = Math.hypot(dx, dz) || 0.1;
				flock.fx += (dx / d) * a.maxSpeed * FLEE_W;
				flock.fz += (dz / d) * a.maxSpeed * FLEE_W;
				boost = canSprint ? FLEE_BOOST : 1;
				pursuing = true;
				// ...but if it CAN'T shake the attackers pressed against it, they wound it (faster the more
				// of them, and the weaker it already is) while it SLASHES back — thinning the mob in real
				// time until its ferocity is spent and the survivors drag it down.
				const attackers = m.attackers ?? 0;
				if (attackers >= MOB_MIN) {
					m.health = Math.max(0, m.health - MOB_KILL_DPS * attackers * h);
					m.slashCd -= h;
					if (m.slashCd <= 0 && m.slashBudget > 0) {
						const victim = this.#nearestAttacker(m);
						if (victim) {
							victim.dead = true;
							victim.asleep = false;
							victim.agent.vx = 0;
							victim.agent.vz = 0;
							m.slashBudget--;
							m.slashCd = SLASH_CD;
						}
					}
				}
			} else if (m.spooked > 0 && m.bully && !m.bully.dead) {
				// freshly slashed → keep bolting from the attacker for a few seconds (and stay awake)
				const dx = a.x - m.bully.agent.x;
				const dz = a.z - m.bully.agent.z;
				const d = Math.hypot(dx, dz) || 0.1;
				flock.fx += (dx / d) * a.maxSpeed * FLEE_W;
				flock.fz += (dz / d) * a.maxSpeed * FLEE_W;
				boost = canSprint ? FLEE_BOOST : 1;
				pursuing = true;
			} else if (m.threat) {
				if (m.threat.mobbed) {
					// the herd has the numbers → turn and CHARGE the hunter, driving it off (no slash of
					// their own, so the hunter is unharmed — it just gets chased away)
					const dx = m.threat.agent.x - a.x;
					const dz = m.threat.agent.z - a.z;
					const d = Math.hypot(dx, dz) || 0.1;
					flock.fx += (dx / d) * a.maxSpeed * MOB_W;
					flock.fz += (dz / d) * a.maxSpeed * MOB_W;
					boost = canSprint ? FLEE_BOOST : 1;
					pursuing = true;
				} else {
					// flee the nearest hunter — sprint if there's stamina, else stumble away at a walk
					const dx = a.x - m.threat.agent.x;
					const dz = a.z - m.threat.agent.z;
					const d = Math.hypot(dx, dz) || 0.1;
					flock.fx += (dx / d) * a.maxSpeed * FLEE_W;
					flock.fz += (dz / d) * a.maxSpeed * FLEE_W;
					boost = canSprint ? FLEE_BOOST : 1;
					pursuing = true;
				}
			} else if (fightingRival && m.rival) {
				// TERRITORIAL: two apex predators that have crowded each other too long stop tolerating it and
				// fight — charge the rival; on contact both bleed, so one breaks off wounded (then flees its
				// bully via the spooked branch) or is dragged down. Apex hunters spread out instead of banding.
				const r = m.rival;
				const dx = r.agent.x - a.x;
				const dz = r.agent.z - a.z;
				const d = Math.hypot(dx, dz) || 0.1;
				flock.fx += (dx / d) * a.maxSpeed * CHASE_W;
				flock.fz += (dz / d) * a.maxSpeed * CHASE_W;
				boost = canSprint ? CHASE_BOOST : 1;
				pursuing = true;
				if (d < m.radius + r.radius + CONTACT_PAD) {
					m.health = Math.max(0, m.health - RIVAL_DPS * h); // takes blows in the scrap (death handled up top)
					if (m.health < HURT_AT) ((m.spooked = Math.max(m.spooked, 2.5)), (m.bully = r)); // wounded → break off & flee
				}
			} else if (huntPlayer) {
				// charge the player; sprint when close. Never catches (you're uncatchable — Player.svelte
				// push-out stops overlap), it just bumps + pressures you and burns stamina, then tires and
				// lies down to rest. Outrun it (sprint) or outlast it.
				const dx = px - a.x;
				const dz = pz - a.z;
				const d = Math.hypot(dx, dz) || 0.1;
				const close = d * d < hunt2;
				flock.fx += (dx / d) * a.maxSpeed * CHASE_W;
				flock.fz += (dz / d) * a.maxSpeed * CHASE_W;
				boost = close && canSprint ? CHASE_BOOST : 1;
				pursuing = true;
			} else if (m.prey) {
				// STALK toward the best prey at a walk; SPRINT in once close enough for the kill
				const dx = m.prey.agent.x - a.x;
				const dz = m.prey.agent.z - a.z;
				const d = Math.hypot(dx, dz) || 0.1;
				const close = d * d < hunt2;
				flock.fx += (dx / d) * a.maxSpeed * CHASE_W;
				flock.fz += (dz / d) * a.maxSpeed * CHASE_W;
				boost = close && canSprint ? CHASE_BOOST : 1; // only the close sprint costs stamina
				pursuing = true;
				if (close && d < m.radius + m.prey.radius + CONTACT_PAD) {
					// the hunter SLASHES — its prey is killed outright
					m.prey.dead = true;
					m.prey.asleep = false;
					m.prey.agent.vx = 0;
					m.prey.agent.vz = 0;
					// eat — a kill REFUELS energy; keep hunting until full, then drop into a food coma
					m.meals++;
					m.chaseOX = NaN; // the chase ended in a kill → reset the origin
					const fullAfter = ECO[m.kind]?.fullAfter;
					if (fullAfter && m.meals >= fullAfter) {
						m.stamina = Math.min(m.stamina, 0.15); // gorged → lie down and digest
						m.asleep = true;
						m.sleepTimer = sleepSecs(m.kind);
					} else {
						m.stamina = Math.min(1, m.stamina + EAT_GAIN); // a meal restores energy
					}
				}
			} else if (fishLuring) {
				// pad toward the fish at a curious walk (no sprint) — the pond obstacle halts the cat at the bank
				const dx = fishX - a.x;
				const dz = fishZ - a.z;
				const d = Math.hypot(dx, dz) || 0.1;
				flock.fx += (dx / d) * a.maxSpeed * CHASE_W * 0.6;
				flock.fz += (dz / d) * a.maxSpeed * CHASE_W * 0.6;
				pursuing = true;
			}

			// WILDLIFE REACTS TO THE PLAYER — animals scatter when you get close, and from farther away when
			// you RUN at them. Skittishness falls with trophic rank: rabbits bolt, an apex dinosaur ignores you.
			const skittish = Math.max(0, (5 - m.rank) / 4); // rabbit 1 → 1.0 … dinosaur 5 → 0
			if (skittish > 0 && !huntPlayer && !m.companion) {
				const dx = a.x - px;
				const dz = a.z - pz;
				const d = Math.hypot(dx, dz);
				// stand still → they tolerate you close (~3 m); walk → mild spread; sprint → real scatter.
				// Jumpier after dark.
				const scareR = (2.5 + Math.max(0, pSpeed - 3) * 0.5) * (0.6 + 0.7 * skittish) * (1 + 0.4 * night);
				if (d < scareR && d > 0.01) {
					const w = skittish * (1 - d / scareR); // stronger the closer / more skittish
					flock.fx += (dx / d) * a.maxSpeed * FLEE_W * w;
					flock.fz += (dz / d) * a.maxSpeed * FLEE_W * w;
					pursuing = true;
					if (canSprint && w > 0.25) boost = Math.max(boost, FLEE_BOOST); // real bolt when truly spooked
				}
			}

			// PERSONAL SPACE: every animal gives the player's body a berth and steers AROUND it — so a lone
			// frolicking or fleeing animal swerves past you instead of barging through. Distinct from the fear
			// scatter above (this applies even to bold/non-skittish animals). Skipped for a hunter deliberately
			// coming for you (huntPlayer) and for your own pet. The hard push-out in Player.svelte then only
			// SHOVES you under crowd pressure (a stampede) — a single animal yields rather than moving you.
			if (!huntPlayer && !m.companion) {
				const adx = a.x - px;
				const adz = a.z - pz;
				const ad = Math.hypot(adx, adz);
				const avoidR = m.radius + 1.5; // player body (~0.5) + a margin to round the corner early
				if (ad < avoidR && ad > 0.01) {
					const w = 1 - ad / avoidR; // firmer the closer
					flock.fx += (adx / ad) * a.maxSpeed * AVOID_W * w;
					flock.fz += (adz / ad) * a.maxSpeed * AVOID_W * w;
				}
			}

			// a wound makes it LIMP — caps every gait (walk, flee, charge) so a healthy hunter runs it down
			if (m.health < HURT_AT) boost *= HURT_SPEED;

			// energy: sprinting burns it fast; a CARNIVORE also has a constant metabolism (BASAL_DRAIN) and NO
			// idle recovery — it refuels only by EATING or SLEEPING, so it gets hungry and must hunt. Prey/people
			// keep the simple rest-recovers-stamina model.
			if (boost > 1) m.stamina = Math.max(0, m.stamina - (EXERT_DRAIN / m.endurance) * h);
			if (ECO[m.kind]?.hunts === 'lower') m.stamina = Math.max(0, m.stamina - BASAL_DRAIN * h);
			else if (boost <= 1) m.stamina = Math.min(1, m.stamina + RECOVER * m.endurance * h);
			// an exhausted hunter lies down to sleep it off — but never while a threat, a nearby peer, a
			// fresh scare or a mob keeps it on edge (then it stays awake and keeps moving)
			if (
				m.stamina <= 0 &&
				ECO[m.kind]?.hunts === 'lower' &&
				!m.threat &&
				!m.nearPredator &&
				m.spooked <= 0 &&
				!mobbed
			) {
				m.asleep = true;
				m.sleepTimer = sleepSecs(m.kind);
			}

			a.update(h, m.menu, flock, boost, pursuing);
			this.#resolveObstacles(m); // keep it out of solid props (no tunnelling, slides around)
			this.#resolveTrees(m); // ...and out of ambient-forest trunks
			m.dist = Math.hypot(a.x - px, a.z - pz);
			m.lod = m.dist > LOD2_DIST ? 2 : m.dist > LOD1_DIST ? 1 : 0;
		}

		// danger level for the UI vignette — ease toward this tick's peak so the alarm swells/fades smoothly
		playerState.danger += (dangerNow - playerState.danger) * Math.min(1, 6 * h);

		// 4. shadow budget — only the nearest few cast (shadows are the dominant cost at scale)
		this.#assignShadows();
	}

	// Hard-resolve an agent out of any solid prop it overlaps — a position correction (not a soft force)
	// so a fast sprinter can never tunnel through a wall. The inward velocity is cancelled but tangential
	// motion is kept, so a chaser SLIDES around the obstacle instead of sticking to it.
	#resolveObstacles(m: ManagedAgent): void {
		if (!this.#hasObstacles) return;
		const a = m.agent;
		this.#obGrid.forEachNeighbor(a.x, a.z, (o) => {
			const dx = a.x - o.x;
			const dz = a.z - o.z;
			let nx: number;
			let nz: number;
			if (o.hx !== undefined) {
				// ORIENTED BOX (buildings) — rotate into local frame, clamp, eject along least-penetration axis
				const cs = o.cos!;
				const sn = o.sin!;
				const lx = dx * cs - dz * sn;
				const lz = dx * sn + dz * cs;
				const hx = o.hx + m.radius;
				const hz = o.hz! + m.radius;
				if (Math.abs(lx) >= hx || Math.abs(lz) >= hz) return; // outside the inflated box
				let nlx = lx;
				let nlz = lz;
				let lnx: number; // local-space push normal
				let lnz: number;
				if (hx - Math.abs(lx) < hz - Math.abs(lz)) {
					nlx = lx >= 0 ? hx : -hx;
					lnx = lx >= 0 ? 1 : -1;
					lnz = 0;
				} else {
					nlz = lz >= 0 ? hz : -hz;
					lnx = 0;
					lnz = lz >= 0 ? 1 : -1;
				}
				a.x = o.x + (nlx * cs + nlz * sn); // local → world
				a.z = o.z + (-nlx * sn + nlz * cs);
				nx = lnx * cs + lnz * sn; // push normal back to world
				nz = -lnx * sn + lnz * cs;
			} else {
				const min = o.r + m.radius;
				const d2 = dx * dx + dz * dz;
				if (d2 >= min * min || d2 === 0) return;
				const d = Math.sqrt(d2);
				nx = dx / d;
				nz = dz / d;
				a.x = o.x + nx * min; // shove back out to the footprint edge
				a.z = o.z + nz * min;
			}
			const vn = a.vx * nx + a.vz * nz; // kill only the component driving INTO the prop → SLIDE along it
			if (vn < 0) {
				a.vx -= vn * nx;
				a.vz -= vn * nz;
				a.speed = Math.hypot(a.vx, a.vz);
			}
		});
	}

	// Same hard push-out as #resolveObstacles, but against the deterministic ambient-forest trees (no grid
	// needed — their placement is a pure function of the world cell, shared with the renderer + the player).
	#resolveTrees(m: ManagedAgent): void {
		const a = m.agent;
		forEachTreeNear(a.x, a.z, m.radius + 1, (tr) => {
			if (onPath(this.#paths, tr.x, tr.z)) return; // AmbientScatter culls trees on roads → don't bump the ghost
			const dx = a.x - tr.x;
			const dz = a.z - tr.z;
			const min = m.radius + treeRadius(tr.scale);
			const d2 = dx * dx + dz * dz;
			if (d2 >= min * min || d2 === 0) return;
			const d = Math.sqrt(d2);
			const nx = dx / d;
			const nz = dz / d;
			a.x = tr.x + nx * min; // shove back out to the trunk edge
			a.z = tr.z + nz * min;
			const vn = a.vx * nx + a.vz * nz; // cancel only the inward component → it slides past the trunk
			if (vn < 0) {
				a.vx -= vn * nx;
				a.vz -= vn * nz;
				a.speed = Math.hypot(a.vx, a.vz);
			}
		});
	}

	/** The closest living mobber pressed against hunter `m` (one of the prey currently fleeing it). */
	#nearestAttacker(m: ManagedAgent): ManagedAgent | null {
		let best: ManagedAgent | null = null;
		let bestD = Infinity;
		this.#grid.forEachNeighbor(m.agent.x, m.agent.z, (o) => {
			if (o.dead || o.threat !== m) return; // must be one of THIS hunter's mobbers
			const dx = o.agent.x - m.agent.x;
			const dz = o.agent.z - m.agent.z;
			const d2 = dx * dx + dz * dz;
			const c = m.radius + o.radius + CONTACT_PAD + 0.4;
			if (d2 < c * c && d2 < bestD) ((bestD = d2), (best = o));
		});
		return best;
	}

	// Reynolds flocking, summed over grid neighbours: an always-on anti-overlap push (bodies never
	// interpenetrate), a density-GATED comfort-spread (a little crowding is fine; a pile-up eases
	// apart), and light cohesion + alignment so groups read as a living flock.
	#flock(m: ManagedAgent, px: number, pz: number): { fx: number; fz: number } {
		const a = m.agent;
		const sepR = m.radius + (m.kind === 'person' ? 1.5 : 1.2); // comfort / personal space
		const hardR = m.radius + (m.kind === 'person' ? 0.4 : 0.3); // bodies (nearly) touching
		const sepR2 = sepR * sepR;
		const nr2 = NEIGHBOR_RADIUS * NEIGHBOR_RADIUS;

		let sepX = 0;
		let sepZ = 0;
		let hardX = 0;
		let hardZ = 0;
		let nClose = 0;
		let cohX = 0;
		let cohZ = 0;
		let aliX = 0;
		let aliZ = 0;
		let nNear = 0;

		const repel = (dx: number, dz: number, d2: number) => {
			const d = Math.max(Math.sqrt(d2), 0.2);
			if (d2 < sepR2) {
				const w = (sepR - d) / sepR / d; // stronger when closer; /d normalises direction
				sepX += dx * w;
				sepZ += dz * w;
				nClose++;
			}
			if (d < hardR) {
				const hw = (hardR - d) / hardR / d;
				hardX += dx * hw;
				hardZ += dz * hw;
			}
		};

		this.#grid.forEachNeighbor(a.x, a.z, (o) => {
			if (o === m || o.dead) return;
			const dx = a.x - o.agent.x;
			const dz = a.z - o.agent.z;
			const d2 = dx * dx + dz * dz;
			if (d2 > nr2) return; // out of range (or a hash-collision false neighbour)
			cohX += o.agent.x;
			cohZ += o.agent.z;
			aliX += o.agent.vx;
			aliZ += o.agent.vz;
			nNear++;
			repel(dx, dz, d2);
		});

		// the player is a separation-only neighbour → crowds part around you
		const pdx = a.x - px;
		const pdz = a.z - pz;
		const pd2 = pdx * pdx + pdz * pdz;
		if (pd2 < sepR2) repel(pdx, pdz, pd2);

		let fx = 0;
		let fz = 0;

		// ANTI-OVERLAP — always on so two agents never stand inside each other
		if (hardX !== 0 || hardZ !== 0) {
			const hl = Math.hypot(hardX, hardZ) || 1;
			const s = (a.maxSpeed * 1.3) / hl;
			fx += hardX * s;
			fz += hardZ * s;
		}

		// COMFORT-SPREAD — density-gated (smoothstep, never a hard switch → no boundary jitter)
		const sepGain = smoothstep(0, 2, nClose - DENSITY_THRESHOLD);
		if (sepGain > 0 && (sepX !== 0 || sepZ !== 0)) {
			const sl = Math.hypot(sepX, sepZ) || 1;
			const s = (a.maxSpeed * SEP_WEIGHT * sepGain) / sl;
			fx += sepX * s;
			fz += sepZ * s;
		}

		// COHESION + ALIGNMENT — gentle, so groups read as a flock without collapsing to a point
		if (nNear > 0) {
			const cdx = cohX / nNear - a.x;
			const cdz = cohZ / nNear - a.z;
			const cl = Math.hypot(cdx, cdz) || 1;
			// people barely cohere (0.06 < animals' 0.1) — they're wandering EXPLORERS, not a flock; the old 0.25
			// glued them into the "huge groups" complaint even after the wide-leash/wanderlust retune. Animals
			// still flock loosely.
			const cohW = m.kind === 'person' ? 0.06 : 0.1;
			const c = (a.maxSpeed * cohW) / cl;
			fx += cdx * c;
			fz += cdz * c;
			fx += (aliX / nNear - a.vx) * ALI_WEIGHT;
			fz += (aliZ / nNear - a.vz) * ALI_WEIGHT;
		}

		m.crowd = nNear; // how many neighbours it has → a lone hunter stalks you; a herd just wanders
		this.#flockOut.fx = fx; // reuse the buffer (caller reads it synchronously before the next #flock call)
		this.#flockOut.fz = fz;
		return this.#flockOut;
	}

	#assignShadows(): void {
		if (this.#agents.size <= SHADOW_AGENTS) {
			for (const m of this.#agents) m.castShadow = true;
			return;
		}
		// Pick the SHADOW_AGENTS nearest (by this tick's `m.dist`) in ONE O(N·K) pass into a reused buffer —
		// the old `[...agents].sort()` allocated a fresh full array AND sorted all N every frame (60×/s), which
		// at herd scale (hundreds–1000s of agents) is pure GC churn + wasted work when we only need the top 12.
		const near = this.#shadowScratch;
		near.length = 0;
		let maxI = 0; // index in `near` of the FARTHEST currently-kept candidate (the next to evict)
		for (const m of this.#agents) {
			m.castShadow = false;
			if (near.length < SHADOW_AGENTS) {
				near.push(m);
				if (m.dist > near[maxI].dist) maxI = near.length - 1;
			} else if (m.dist < near[maxI].dist) {
				near[maxI] = m;
				maxI = 0; // re-find the farthest kept
				for (let i = 1; i < SHADOW_AGENTS; i++) if (near[i].dist > near[maxI].dist) maxI = i;
			}
		}
		for (let i = 0; i < near.length; i++) near[i].castShadow = true;
	}
}

export const agentManager = new AgentManager();
