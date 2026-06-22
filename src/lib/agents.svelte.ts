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
const CH = { speed: 2 } as const; // only the render-gait speed roll is still done JS-side (Rust owns the sim rolls)

// ── Per-kind RENDER trait mirror ────────────────────────────────────────────────────────────────────
// The Rust core (crates/worldsim/src/eco.rs) owns the CANONICAL, full ecosystem table and runs ALL the sim
// (movement / food-chain / combat / metabolism). JS keeps ONLY what RENDERING needs: each kind's `speed` range →
// a per-agent maxSpeed that scales the leg-swing GAIT, and `rank` (the player-stun check in Player.svelte). The
// sim-only fields that used to live here (endurance / hunts / fullAfter / sleepSecs / mobToll, and the per-agent
// stamina/slash/rival/chase state in makeManaged) were JS-SIM-era residue — removed, since Rust is the one source
// of truth now and the duplicate table was pure mirror-sync hazard (every balance tweak had to be edited twice).
export const ECO: Record<string, { rank: number; speed: [number, number] }> = {
	rabbit: { rank: 1, speed: [4.0, 5.2] },
	cat: { rank: 2, speed: [3.5, 4.5] },
	kangaroo: { rank: 2, speed: [3.4, 4.6] },
	person: { rank: 3, speed: [1.8, 2.5] },
	lion: { rank: 4, speed: [3.7, 4.8] },
	dinosaur: { rank: 5, speed: [4.8, 6.2] }
};

/** A random max speed in this kind's range (varies every individual; deterministic per seedId) — for the gait. */
export function speedFor(kind: string, seedId: number): number {
	const s = (ECO[kind] ?? ECO.cat).speed;
	return rng.range(s[0], s[1], seedId, CH.speed);
}

// render-side perf-flag thresholds (the only sim-ish constants left — used by assignPerfFlags / #assignShadows)
// Reveal distances bumped (the AmbientScatter freeze fix freed main-thread budget): full articulated animals now
// draw FARTHER before dropping to the flat impostor (user: "increase the radius of drawing the full animal rather
// than a floating cube"). Tune down if a 1000-strong herd ever hitches again.
const LOD1_DIST = 36;
// LOD2 = "far": the full articulated Critter/Npc mesh is shed and the agent draws via the instanced impostor.
// Exported so those components can init their mesh-mounted state from spawn distance (avoid the mount storm).
export const LOD2_DIST = 95;
const SHADOW_AGENTS = 12; // only the nearest few cast shadows (shadows are the dominant cost at scale)
// MESH BUDGET — cap full articulated meshes to the nearest N agents. Distance-LOD alone fails for a CLUSTER:
// spawn 1000 cats around you and hundreds sit inside LOD2_DIST → hundreds of ~15-node bodies + per-frame tasks
// + shadow work → the main thread hangs (the Worker moved the SIM off-thread, not this render cost). With a
// COUNT cap, surplus near agents fall back to the instanced impostor (2 draw calls), so a dense crowd costs a
// fixed amount no matter how many you spawn — the nearest N stay fully articulated (you only scrutinise those).
const MESH_BUDGET = 150;

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
	// the Rust sim mirrors the live values back each tick (health/dead/asleep/hunting); the rest of the ecosystem
	// state the old JS sim tracked (stamina/slash/rival/chase/mob/…) is gone — Rust owns it now.
	rank: number; // trophic level — the only food-chain field the render side still reads (the player-stun check)
	seedId: number; // stable per-agent uint32 → its deterministic trait RNG (sex/gait; matches the Rust seed_id)
	health: number; // 0..1; ≤0 = death; <HURT_AT = injured/limping (mirrored from Rust)
	dead: boolean; // caught → corpse (mirrored from Rust)
	corpseAge: number; // seconds it's been dead → drives the sink-and-reap decay (0 while alive)
	asleep: boolean; // resting (mirrored from Rust)
	hunting: boolean; // this apex is charging YOU right now (mirrored from Rust) → its eyeshine glares red
	migrating: boolean; // a roamer en route to another settlement (mirrored from Rust) → the HUD tallies these
	pregnant: boolean; // carrying a litter (mirrored from Rust) → the view shows a rounded belly
	guardian: boolean; // his mate is expecting (mirrored from Rust) → the view arms him with a machete
	drinking: boolean; // lapping at a water edge (mirrored from Rust) → the view dips its head
	juvenile?: boolean; // a Rust-bred newborn → rustSim stamps a maturation breed-cooldown when it spawns into the sim
	gene?: number; // inherited VIGOR (≈1.0; scales speed) — ferried from the Rust birth → set on the sim agent at spawn
	pfamA?: number; // mother's lineage id — ferried from the Rust birth → set_lineage at spawn (incest avoidance)
	pfamB?: number; // father's lineage id
	genome?: number[]; // inherited behaviour genome (5 weights) — ferried from the Rust birth → set_genome at spawn
}

/** Build a fully-seeded managed agent from its kind (so components don't repeat the eco wiring). The Rust sim
 *  re-seeds its own copy from the same `seedId`; these fields are the JS-side mirror the renderers read. */
export function makeManaged(agent: Agent, kind: string, radius: number, menu: Behavior[], objId: string | undefined, seedId: number): ManagedAgent {
	// Render-only state. The Rust sim owns + mirrors back the live values (health / dead / asleep / hunting) each
	// tick; everything else the OLD JS sim tracked here (stamina/slash/rival/chase/…) is gone — Rust computes it.
	return {
		agent,
		kind,
		radius,
		menu,
		objId,
		lod: 0,
		castShadow: true,
		dist: 0,
		rank: (ECO[kind] ?? ECO.cat).rank, // the player-stun check reads this; rest of the food-chain is Rust's
		seedId,
		health: 1,
		dead: false,
		corpseAge: 0,
		asleep: false,
		hunting: false,
		migrating: false,
		pregnant: false,
		guardian: false,
		drinking: false
	};
}

class AgentManager {
	#agents = new Set<ManagedAgent>();
	#shadowScratch: ManagedAgent[] = []; // reused nearest-N buffer for the shadow budget (no per-frame alloc)
	#meshScratch: ManagedAgent[] = []; // reused nearest-N buffer for the mesh budget (no per-frame alloc)
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
		this.#assignMeshBudget();
		this.#assignShadows();
	}

	/** Cap full articulated meshes to the nearest MESH_BUDGET — anyone farther than the budget-th nearest is
	 *  forced to the instanced impostor (lod 2), so a dense crowd costs a fixed amount however many you spawn.
	 *  One O(N·K) selection (reused buffer, no alloc) to find the budget-th nearest distance, then one O(N) demote. */
	#assignMeshBudget(): void {
		if (this.#agents.size <= MESH_BUDGET) return; // everyone fits → keep their distance-LOD
		const near = this.#meshScratch;
		near.length = 0;
		let maxI = 0; // index in `near` of the FARTHEST currently-kept candidate
		for (const m of this.#agents) {
			if (near.length < MESH_BUDGET) {
				near.push(m);
				if (m.dist > near[maxI].dist) maxI = near.length - 1;
			} else if (m.dist < near[maxI].dist) {
				near[maxI] = m;
				maxI = 0; // re-find the farthest kept
				for (let i = 1; i < MESH_BUDGET; i++) if (near[i].dist > near[maxI].dist) maxI = i;
			}
		}
		const cutoff = near[maxI].dist; // distance of the MESH_BUDGET-th nearest agent
		for (const m of this.#agents) if (m.dist > cutoff && m.lod < 2) m.lod = 2; // surplus near agents → impostor
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
