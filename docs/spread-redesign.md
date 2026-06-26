# The Spread Redesign — ending the whack-a-mole

Systemic diagnosis + coherent plan for: jitter that survived instancing, settlements merging into mega-cities, and the perpetually-broken fence. Built from a 6-subsystem code audit (sim core, settlement dynamics, fence, streaming, render, data/arch). Evidence is `file:line`; claims are marked **[verified-in-code]** or **[inference]** as the auditors rated them.

## The one-sentence story

**The world has no force that creates *distance*.** Every spatial mechanism either pulls people *toward* existing towns or rewards building *in place*, so 3 seeded towns densify and sprawl until they merge. The fence is fitted to that unbounded, drifting, mergeable town, so it can never be stable; and render cost scales with the resulting density, so jitter is worst exactly where the cities are densest. Underneath, **four subsystems each define "a town" differently with no shared source of truth** — which is why every point-fix desyncs the others. That four-way disagreement *is* the whack-a-mole engine.

## Complaint → root cause

| Complaint | It's actually a symptom of… |
|---|---|
| Settlements merge into mega-cities | **No pioneer drive + centripetal migration + unbounded in-place growth** (roots 1–3) |
| Fence still broken | **Downstream of the merge** — you can't wall an unbounded, drifting, mergeable, lossy-view town (root 5) |
| Jitter even after instancing | **Render cost scales with city density** — proxy scans, per-frame budget churn, whole-world JSON (root 6) |
| Whack-a-mole | **Four incompatible definitions of "a settlement"** (root 4) |

## The root causes

### R1 — There is no pioneer/exploration drive, and migration pulls people *inward* [verified-in-code]
- The `industry` gene is documented as "founder vs drifter" but its **only** consumer is the well-vs-house split (`genome.rs:32`, `world.rs:1969`). It does not affect dispersal, migration, or building. **The trait you want doesn't exist as a phenotype.**
- The utility scorer has no Explore/Pioneer primitive (`utility.rs`); `WANDER_FLOOR=0.16` is the only "do nothing special" option.
- No need grows with crowding — `social` only *pulls together* (`needs.rs:39`). The brain can express "I'm lonely → gather" but never "it's too packed → leave."
- Inter-settlement migration steers roamers toward the **nearest existing under-full town** (`MIGRATE_R=320m`, `MIGRATE_W=0.55`, `world.rs:2319-2361`). Animals get a real outward nomad heading; **people never do.**
- "Wanderlust" is a per-tick RNG coin-flip re-rolled every 30s, not heritable (`world.rs:2327`) — selection can't accumulate it into a pioneer lineage.

### R2 — "A town" has no bound; growth is rewarded in place [verified-in-code]
- `COLONY_MAX=10` is a **per-plot 75m window**, not a per-town cap (`worldgen.rs:452-468`). A house on the sparse frontier always has <10 neighbours → accepted → the rim advances outward. The only real ceiling is the world-wide `HOUSE_CAP=140`.
- Densification feedback: `world_area_scale = 1 + builds/25` (cap 4×, `world.rs:266`, comment "cities are the point"). More builds → more carrying capacity → more people → more builds.
- The **75–350m dead zone**: a house must be infill (<75m of a building) **or** a far new town (≥350m); nothing in between is allowed (`worldgen.rs:468-472`). Founding at ≥350m is unreachable because dispersal force dies within tens of metres.

### R3 — The bond/build chain anchors exactly the agents who build [verified-in-code]
- Conception installs a permanent pair-bond tether (`BOND_W`, ×2.2 while gestating, 50% lifelong — `world.rs:1940,2363`).
- Building requires an existing bond (`partner.is_some()`, `world.rs:1960`); bonded adults are past `DISPERSE_AGE=0.32` → exempt from dispersal.
- **Correction to the original hypothesis:** breeding does **not** require a house. The chain is breed→bond→*later*build, and it's the **tether + migration**, not a house-gate, that anchors people. Patching the imagined "house before pregnant" rule would do nothing.

### R4 — FOUR incompatible definitions of "a settlement" — the structural root [verified-in-code]
- build_ops colony = **75m** + 350m gap (`worldgen.rs:427`); wall union-find = **60m** (`worldgen.rs:227`); sim avoidance/migration = **refuges-centroid** within `SETTLEMENT_GATHER_R` (`world.rs:2415`); streaming = **200m region tiles** (`streaming.ts:20`).
- No single source of truth for "where is town X and how big." A fix to one radius desyncs the other three. **This is why the bugs respawn.**

### R5 — The fence is fitted to an unbounded, drifting, mergeable, lossy-view town [verified-in-code + inference]
- Ring radius grows every refit (clamp 7–400m); **centroid-keyed jitter** reshuffles the *entire* ring on any home add → mass remove+re-add + visible "jump" (`worldgen.rs:292`).
- Union-find `GATHER=60m` **merges two sprawling towns into one giant ring** once their homes get within 60m.
- `settlement_ops` reads a **filtered + streaming-lossy** world → walls a *phantom/partial* town (`Scene.svelte:652`).
- Streaming splits one ring across 200m tiles; `enforceLiveBudget` (STRUCT_BUDGET=220, which a mature city exceeds) evicts the **far arc** → half-open ring.
- Three writers (live / fastForward / on-load), three id prefixes, three world snapshots.
- **The fence is ~entirely a symptom of R2 + R4. Patching it directly is the canonical whack-a-mole.**

### R6 — Render cost scales with density and self-invalidates [verified-in-code]
- `world.objects` is a deep `$state` proxy holding **every** element (creatures + every fence panel/grave/flower). The **7 instanced renderers each scan all of it every frame** through proxy traps; every Scene mutation invalidates them + the obstacle `$effect` + PlacedShadows.
- `enforceLiveBudget` runs every frame; once over STRUCT_BUDGET (always, in a mature demo) it allocates+sorts+**reassigns `world.objects` every frame** → re-fires the whole reactive cascade.
- Whole-world `JSON.stringify(world)` (a proxy!) on every build/death/veg/grave frame → main-thread O(N) hitch (`Scene.svelte:380,424,499`).
- visible-set rebuilds on every `objLen` change (every birth/death), not just movement.
- Buildings/trees/creatures still mount as keyed components (shader compile on mount).
- DRS strobes resolution in response to all the above → visible flashing.
- **Instancing the props touched none of this — it's all in the data/reactivity path. Complaint #2 (density) directly drives complaint #1 (jitter).**

### R7 — The JS-owned half is non-deterministic + double-bookkeeps population [verified-in-code]
- `fastForward`/`wakeRegion`/wildcards/graves use `Math.random()` → the world is **not** a pure function of (seed, tick), breaking the time-travel contract the Rust core was built for.
- Worker roster vs `world.objects` can diverge across reset/stream (ghosts / frozen agents).

## The fix — three layers, on your three principles

The cure is to **delete the fighting gates and let the behaviour emerge**, built in this dependency order:

### Layer 1 — Robust emergent core (makes spreading the default)
1. **One settlement source of truth, owned by Rust.** A `Settlement{ id, seed, centroid, members, capacity }` record that build/wall/avoid/stream all read. Kills R4 → ends respawning bugs.
2. **Add the missing primitive: heritable land-pressure / pioneer drive.** A need that *grows* with local built-density/crowd, weighted by a real genome trait (repurpose the inert `industry`). When a settlement passes its soft capacity, surplus agents get a **strong outward drive to virgin frontier** in a random direction (longitudinal **and** toward the curve) — *not* toward existing towns. Spreading becomes emergent, selectable, and the thing you wanted.
3. **Bound each ecosystem small.** A **per-settlement** capacity (replacing the per-plot `COLONY_MAX` and the densification feedback). Small + distinct by construction.

### Layer 2 — Fence follows a stable settlement
4. Fence ring keyed on the settlement's **stable seed/identity**, not the drifting centroid (no reshuffle). Treat a ring as **one unit** for streaming/budget (all-live-or-all-dormant). With small bounded towns that never merge, the ring is a stable closed loop. R5 dissolves.

### Layer 3 — Render decoupled from world size (kills jitter + delivers the wow)
5. **One kind-bucketed structure index** updated on mutation, not 7 per-frame proxy scans. Move `world.objects` out of the deep-proxy hot path (non-reactive store + explicit change signals). Stop whole-world `JSON.stringify` (pass deltas / the active region only).
6. **Far settlements = cheap glows, not geometry.** Full meshes only in the live span; the horizon carries `SettlementGlows`/`BuildingGlow` lights. Low local density by design → GPU happy → no jitter → **"lights in every direction."**

### Layer 4 (later) — Determinism / architecture
7. Move the JS-owned non-deterministic half (fastForward, wake, wildcards, graves) into Rust / seed it. Restores (seed,tick)→world for time-travel and removes the second source of truth (R7).

## What to DELETE (simplification is the cure)
- The 75–350m **dead zone** and the per-plot `COLONY_MAX`.
- The **centripetal-only** people-migration (toward existing refuges as the sole outflow).
- The **centroid-keyed** fence jitter.
- Any JS settlement notions in `city.ts` / `settlementPlanner.ts` that duplicate Rust.
- The per-frame **whole-world `JSON.stringify`** round-trips.
- The 4-way disagreement on "a town" (→ one record).

## Success metric
Stand still, spin the camera, **count distinct settlement lights around the horizon.** That number going *up* over time is the win — and it is only achievable once spreading is emergent and render cost is decoupled from world size.

## Build order (LOCKED: full consolidation · keep 3 retuned seeds · emergent/varied capacity)

Built in verifiable increments; each phase has Rust scenario tests + a Playwright/dumpState check before the next.

- **P0 — Settlement = ONE Rust record (the keystone).** Introduce `Settlement{ id, seed, centroid, member homes, pop, capacity }` in Rust, maintained incrementally as homes add/remove. Build-placement, wall-fit, sim avoidance/migration, and streaming all read THIS one record — replacing the four ad-hoc radii (75/60/centroid/200m). Nothing else is consistent until this exists. *Verify:* a scenario asserting one settlement-id per cluster, stable across adds.
- **P1 — Emergent pioneer drive + per-settlement capacity (the core; your "robust primitives").** A `crowding`/land-pressure NEED that grows with local built-density, weighted by a real genome trait (repurpose the inert `industry` into a heritable pioneer phenotype). When a settlement passes its **emergent capacity** (varies by genome + local land), surplus agents get a strong outward drive to EMPTY frontier in a seeded random direction (longitudinal + curve). **Delete:** the 75–350m dead-zone, centripetal-only migration, the per-plot `COLONY_MAX`, the build→capacity densification feedback. *Verify:* a seeded town spawns DISTINCT daughter colonies at distance; the 3 seeds stay distinct & spread over 10k+ ticks (scenario harness measuring inter-settlement distance + count over time).
- **P2 — Fence follows the stable settlement.** One ring per `Settlement`, keyed on its stable seed (no centroid reshuffle); the ring is ONE unit for streaming/budget (all-live-or-all-dormant). Small bounded non-merging towns ⇒ stable closed loop. **Delete** centroid-jitter + the union-find merge path. *Verify:* wall stable across a build (no mass remove/add); intact after a stream round-trip.
- **P3 — Render decoupled from world size.** One kind-bucketed structure index updated on mutation (not 7 per-frame proxy scans); `world.objects` off the deep-proxy hot path; stop whole-world `JSON.stringify` (deltas/active-region only); far settlements = cheap glows, geometry only in the live span. *Verify:* frame-time flat walking into a dense area; horizon shows distant settlement lights.
- **P4 — Determinism / architecture.** Move the JS-owned non-deterministic half (fastForward, wake, wildcards, graves) into Rust / seed it ⇒ (seed,tick)→world for time-travel, one source of truth for population. *Verify:* same seed+ticks ⇒ identical dumpState.

## Honesty note
This synthesis is mine, from the auditors' code-cited findings (the workflow's automated synthesis/verify step kept exceeding the harness output cap). The big causal claims are rated **verified-in-code** by the readers; the cross-subsystem links (e.g. budget-eviction breaking the ring) are **strong-inference**. Before building, the load-bearing inferences should each get a quick confirm against live `dumpState`.
