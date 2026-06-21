// Registry + per-agent RENDER STATE for all ambient agents (critters + people). The SIMULATION is now the
// headless Rust/WASM core (crates/worldsim), driven via rustSim.ts — this file no longer simulates anything.
// The old JS food-chain + Reynolds-flocking tick (~600 lines) was DELETED in the JS→Rust migration (see the
// `rust-owns-all-compute` memory + the migration roadmap in the work-queue): Rust owns ALL compute; JS keeps
// only the render-side bookkeeping the Rust read-back doesn't cover.
//
// What remains here:
//  • the agent REGISTRY — components register/unregister; the renderers + rustSim iterate it (forEach);
//  • the ManagedAgent DATA SHAPE + its seeded factory `makeManaged` (Rust mirrors transforms/flags onto these,
//    so the renderers read `m.agent.rx/rz/rh` + `m.dead/m.asleep` exactly as before);
//  • the cheap per-frame VIEW passes Rust doesn't produce — LOD tiers + the nearest-N shadow budget
//    (`assignPerfFlags`, called by rustSim after each read-back).
// Deliberately NOT reactive in the hot path — these objects are mutated 60×/s; making them $state would cause
// render storms. See docs/npc-movement.md.
import { Agent, type Behavior } from './steering';
import { Rng } from './rng';

// Deterministic per-individual trait RNG (docs/self-sustaining-world.md §1.6): every birth-time draw is keyed
// by the agent's stable `seedId` + a CHANNEL, so spawns are reproducible (same obj.id ⇒ same traits → a shared
// world agrees). (Per-tick rolls now live in the Rust core's addressed RNG.)
const rng = new Rng('worldgen-agents');
const CH = { wander: 1, speed: 2, aggro: 3, slash: 4, breed: 5, mutate: 6, fight: 7 } as const;

// ── Food chain (per-kind birth config) ─────────────────────────────────────────────────────────────
// rank = trophic level. speed = per-individual random range. endurance = sprint stamina. hunts: 'lower' eats
// anything below its rank · 'humans' (people) attack own kind on a coinflip · 'none' = pure prey. fullAfter =
// kills before a food-coma; sleepSecs = that nap's length; mobToll = attackers it slashes dead while mobbed.
// (The Rust core carries the canonical copy of this table; this stays only to seed `makeManaged`.)
export const ECO: Record<
	string,
	{
		rank: number;
		speed: [number, number];
		endurance: number;
		hunts: 'lower' | 'humans' | 'none';
		fullAfter?: number;
		sleepSecs?: number;
		mobToll?: [number, number];
	}
> = {
	rabbit: { rank: 1, speed: [3.6, 4.8], endurance: 1.0, hunts: 'none' },
	cat: { rank: 2, speed: [3.0, 3.9], endurance: 0.8, hunts: 'lower', sleepSecs: 10, mobToll: [1, 2] },
	kangaroo: { rank: 2, speed: [3.4, 4.6], endurance: 0.9, hunts: 'none' },
	person: { rank: 3, speed: [1.8, 2.5], endurance: 0.6, hunts: 'humans' },
	lion: { rank: 4, speed: [3.0, 3.9], endurance: 0.4, hunts: 'lower', fullAfter: 5, sleepSecs: 16, mobToll: [1, 3] },
	dinosaur: { rank: 5, speed: [4.8, 6.2], endurance: 0.3, hunts: 'lower', fullAfter: 9, sleepSecs: 24, mobToll: [2, 5] }
};
const AGGRO_PROB = 0.2; // share of people that turn aggressive (hunt their own kind)

/** A random max speed in this kind's range (varies every individual; deterministic per seedId). */
export function speedFor(kind: string, seedId: number): number {
	const s = (ECO[kind] ?? ECO.cat).speed;
	return rng.range(s[0], s[1], seedId, CH.speed);
}

// render-side perf-flag thresholds (the only sim-ish constants left — used by assignPerfFlags / #assignShadows)
const LOD1_DIST = 26;
// LOD2 = "far": the full articulated Critter/Npc mesh is shed and the agent draws via the instanced impostor.
// Exported so those components can init their mesh-mounted state from spawn distance (avoid the mount storm).
export const LOD2_DIST = 62;
const SHADOW_AGENTS = 12; // only the nearest few cast shadows (shadows are the dominant cost at scale)

// CORPSE DECAY: now that reproduction makes the world cycle life→death→corpse forever, bodies would pile up
// without bound (scene graph + the saved share-link both grow). A corpse lingers (you wanted to SEE dead
// bodies) then, in its final seconds, sinks into the ground and is reaped — its world-object is removed, which
// unmounts the renderer and despawns it from the Rust sim (see Scene's reaper + rustSim's unregister→despawn).
export const CORPSE_DECAY_SECS = 62; // a body lingers this long before it's gone
export const CORPSE_SINK_SECS = 6; // …sinking into the earth over the last few of those seconds

export interface ManagedAgent {
	agent: Agent;
	kind: string; // 'rabbit' | 'cat' | 'kangaroo' | 'person' | 'lion' | 'dinosaur'
	radius: number; // body radius → personal space + catch contact
	menu: Behavior[];
	objId?: string; // the world-object id this agent renders (placed animals) → lets the live state be saved
	tint?: string; // the agent's display colour (e.g. a person's hashed shirt) → far impostors match the near look
	companion?: boolean; // the player's pet — follows you (home leash tracks the player) and isn't scared off

	// written each frame, read by the owning component:
	lod: 0 | 1 | 2; // 0 near (full), 1 mid, 2 far (freeze articulation)
	castShadow: boolean; // only the nearest few cast (shadow budget)
	dist: number; // distance to the player
	// ecosystem state (seeded by makeManaged; the Rust sim mirrors the live values — health/dead/asleep — back):
	rank: number;
	endurance: number;
	aggressive: boolean; // people only — hunts its own kind
	seedId: number; // stable per-agent uint32 → its own deterministic trait RNG (matches the Rust seed_id)
	stamina: number; // 0..1 energy
	health: number; // 0..1; ≤0 = death; <HURT_AT = injured/limping (mirrored from Rust)
	meals: number; // kills since last sleep
	spooked: number; // seconds left fleeing a recent attacker
	mobbed: boolean; // being swarmed
	dead: boolean; // caught → corpse (mirrored from Rust)
	corpseAge: number; // seconds it's been dead → drives the sink-and-reap decay (0 while alive)
	asleep: boolean; // resting (mirrored from Rust)
	hunting: boolean; // this apex is charging YOU right now (mirrored from Rust) → its eyeshine glares red
	juvenile?: boolean; // a Rust-bred newborn → rustSim stamps a maturation breed-cooldown when it spawns into the sim
	sleepTimer: number; // seconds left in the current sleep
	chaseOX: number; // where the current prey-chase began (NaN = none)
	chaseOZ: number;
	giveUpCd: number; // seconds it rests after abandoning a chase
	rivalTime: number; // seconds crowded by a rival → a territorial fight
	slashMax: number; // attackers it can slay in one mob fight
	slashBudget: number; // attackers it can still slash this fight
	slashCd: number; // cooldown until its next retaliatory slash
}

/** Build a fully-seeded managed agent from its kind (so components don't repeat the eco wiring). The Rust sim
 *  re-seeds its own copy from the same `seedId`; these fields are the JS-side mirror the renderers read. */
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
		corpseAge: 0,
		asleep: false,
		hunting: false,
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

class AgentManager {
	#agents = new Set<ManagedAgent>();
	#shadowScratch: ManagedAgent[] = []; // reused nearest-N buffer for the shadow budget (no per-frame alloc)
	#night = 0; // 0 day → 1 night (set from the sky) → fed to the Rust sim

	/** How nocturnal the world is right now (0 day … 1 night) — drives the food chain's day/night mood. */
	setNight(n: number): void {
		this.#night = Math.max(0, Math.min(1, n));
	}

	/** Current night value (0..1) — read by the Rust sim backend to mirror the day/night mood. */
	get nightValue(): number {
		return this.#night;
	}

	/** Recompute per-agent LOD tier + the nearest-N shadow budget from the current positions. The Rust sim
	 *  produces transforms but not these view flags, so rustSim calls this after each read-back (cheap, no alloc
	 *  — reuses #shadowScratch) to keep impostor/shadow perf right. */
	assignPerfFlags(px: number, pz: number): void {
		for (const m of this.#agents) {
			m.dist = Math.hypot(m.agent.x - px, m.agent.z - pz);
			m.lod = m.dist > LOD2_DIST ? 2 : m.dist > LOD1_DIST ? 1 : 0;
		}
		this.#assignShadows();
	}

	register(m: ManagedAgent): void {
		this.#agents.add(m);
	}

	unregister(m: ManagedAgent): void {
		this.#agents.delete(m);
	}

	/** Is this agent still registered? rustSim reconciles its WASM roster against this — an agent that
	 *  unregistered (its object was removed / the world cleared) is despawned from the Rust sim, not left a ghost. */
	has(m: ManagedAgent): boolean {
		return this.#agents.has(m);
	}

	/** Iterate all managed agents (used by the renderers + the Rust sim adapter). */
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

	#assignShadows(): void {
		if (this.#agents.size <= SHADOW_AGENTS) {
			for (const m of this.#agents) m.castShadow = true;
			return;
		}
		// Pick the SHADOW_AGENTS nearest (by this tick's `m.dist`) in ONE O(N·K) pass into a reused buffer —
		// avoids the GC churn of `[...agents].sort()` every frame at herd scale when we only need the top 12.
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
