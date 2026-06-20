# Crowd aversion ("living organism") + many-agent performance — plan

Subagent design output. Goal: when many people/cats pile up they ease apart on their own (Reynolds
**separation**, density-gated so a little crowding is fine), with light cohesion/alignment so groups
read as a living flock — AND it stays smooth at 100+ agents (the user saw jank at ~120). The two are
coupled: separation needs neighbour queries, and the cheap neighbour structure (spatial hash) is also
what makes many agents affordable. Build order below.

## Phase 1 — Central manager + spatial hash (substrate; highest leverage)
- **`src/lib/agents.svelte.ts`** — module singleton `agentManager` (mirror `playerState` pattern,
  deliberately NON-reactive in the hot path). Owns a `Set<ManagedAgent>` registry + the grid + ONE
  `tick(dt)`: rebuild grid → for each agent compute flock force → `agent.update(dt, menu, flock)`.
  `ManagedAgent = { agent, kind, radius, menu, lod, castShadow, distToPlayer }`.
- **`src/lib/spatialhash.ts`** — `SpatialHashGrid`, cell size = neighbour radius (~4m). `Map<number,
  ManagedAgent[]>` keyed `(cx*73856093)^(cz*19349663)`. Rebuilt once/frame. `forEachNeighbor(x,z,r,cb)`
  is **callback form, no per-agent array alloc** (the key GC decision). Invariant: cell size === neighbour radius.
- **`AgentSystem.svelte`** — headless component, single `useTask(dt => agentManager.tick(dt))`, mounted
  once in `Scene.svelte`. Manager runs before components read transforms (or accept 1-frame latency).
- **Components register, don't self-tick:** in Cat/Npc, `$effect(() => { agentManager.register(managed);
  return () => agentManager.unregister(managed) })`. The per-component `useTask` keeps ONLY render
  (read `agent.x/z/heading/speed/turnRate/behavior/progress` → meshes/springs); `agent.update` moves
  to the manager. Keep `managed`/`agent` plain (NOT `$state`) — #1 jank trap is reactivity in the hot path.

## Phase 2 — Separation / flocking (the headline feature)
- `Agent.update(dt, menu, flock?)` blends `flock` into the **desired velocity** (not vx/vz directly),
  then truncates to maxSpeed before the existing accel low-pass — that smoothing is what kills jitter.
- In `tick`, ONE `forEachNeighbor` per agent accumulates: **separation** (within SEPARATION_RADIUS
  ~1.6–1.9m, inverse-distance repulsion), light **cohesion** (toward avg pos), light **alignment**
  (toward avg velocity). Weights: SEP 1.6, COH 0.25 (cats ~0.1), ALI 0.4.
- **Density gate = "a little crowding is OK":** separation scaled by `smoothstep(0, 2, nClose -
  DENSITY_THRESHOLD)` (threshold ~2) — a smooth ramp, NEVER a hard `if` (hard switch = boundary jiggle).
- Home-leash still wins when an agent is past its leash. Add the **player as a separation-only
  neighbour** so crowds part around you (cheap, great "living" touch).
- Failure modes: clump-to-point (raise SEP / lower COH), never-settle (lower SEP, ensure gate hits 0),
  shoving through props (agent-vs-agent only; optional obstacle repulsion later).

## Phase 3 — Rendering perf (ranked cheapest→highest impact)
1. **Shadow cap (biggest, trivial):** only nearest ~12 agents cast shadows; manager flags
   `managed.castShadow` by distance; components set `mesh.castShadow` only on change. (Today EVERY part
   of every agent casts → ~850 shadow meshes.) Optionally drop shadow map to 1024.
2. **Locomotion LOD:** manager sets `managed.lod` by distance — LOD0 <25m full; LOD1 25–60m every 2nd/3rd
   frame, skip micro-springs; LOD2 >60m/in fog freeze legs, only move root. Throttle far steering too
   (but keep grid insertion so they still count as neighbours).
3. **Shared geo/materials:** hoist Cat/Npc geometries + materials to module-level (`critterAssets.ts`),
   cache person materials by color. (AmbientScatter already proves the pattern.)
4. **Instanced impostors (last, only if still janky):** LOD2 agents hide their group and render as a
   single silhouette in two shared InstancedMeshes (cat/person), matrices set by the manager — collapses
   ~100 far agents from ~700 draw calls to 2. Hide the LOD pop in fog.
5. Re-raise the 120 clamp once the above land; document the new safe max.

## Cross-cutting
Keep `$state` out of the per-frame path · grid cell size === neighbour radius · no alloc in neighbour
query · register/unregister via `$effect` cleanup (handles `{#each (obj.id)}` + HMR) · ambient movement
stays non-deterministic (not serialized) so no save concerns.
