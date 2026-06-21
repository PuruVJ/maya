# Self-sustaining world — design spec

> A world that **lives on its own** (populations rise and fall, creatures breed, compete, evolve) and
> **keeps surprising you** (events, migrations, new species, a running story) — built on the systems we
> already have, with the fine-tuned LLM as the slow "director" on top.
>
> Status: **DESIGN ONLY** (2026-06-20). No code yet. This doc is the contract so the work can be split
> across the two chats — see [Ownership & rollout](#ownership--rollout). Grounded in the real code:
> `agents.svelte.ts` (sim), `engine.ts` / `llm-prompt.ts` (ops), `world.ts` (state), `share.ts` (links).
>
> **Role split:** this is a **decisions/spec** doc. The **game/build chat implements**; the design chat owns
> these docs. Don't take code in this doc as written — it's the contract to build against.

---

## ⭐ NORTH STAR (hard rule) — NO TRUE RANDOMNESS, ever

The whole game must be a **pure function of `(seed, clock-tick)`** so it is fully **replayable** and lets us
**fast-forward, peek into the future, and time-travel**. Therefore, in *all* game logic:

- **NO `Math.random()`. NO `Date.now()` / `new Date()` / `performance.now()`. No unseeded entropy.** Every
  random draw goes through the seeded hash-RNG ([`rng.ts`](../src/lib/rng.ts) `Rng`), keyed by
  `(seed, tick, entityId, channel)` — see [§1.5](#15-foundations--clock--deterministic-rng-built-2026-06-20)
  / [§1.6](#16-wiring-the-existing-sim-to-the-clock--rng-game-chat--required).
- **Time is the integer `clock.tick`** ([`clock.ts`](../src/lib/clock.ts)), not wall-clock.
- **Every stray `Math.random()` / wall-clock read in game logic is a BUG.** (Purely cosmetic, non-saved
  visual fuzz that never affects state is the only tolerated exception — and even then, prefer `rng`.)

Everything below — the living sim, Mother Nature, time travel, the shared [Big World](./big-world.md) and its
deterministic base — depends on this rule holding without exception.

---

## 1. The core idea: two layers at two clock speeds

Self-sustenance does **not** come from intelligence — it comes from *rules*, like Conway's Game of Life.
The LLM is far too slow (~seconds/reply) to be a per-frame brain. So we split the world cleanly:

| | **Layer 1 — the Metabolism** | **Layer 2 — the Director** |
|---|---|---|
| What | Energy, grazing, hunting, breeding, death, mutation | Events, migrations, new species, balancing, narration |
| Runs | **Every frame** (60 Hz), in `agents.svelte.ts` tick | **Every ~15–60 s**, a scheduler calls the LLM |
| Cost | Cheap arithmetic per agent | One grammar-constrained generation per cycle |
| Determinism | Deterministic from a seed (shareable) | Occasional, logged as ops (shareable) |
| Who builds | game chat (owns `agents.svelte.ts`) | **this chat** (new files, no collision) |

The metabolism is the heartbeat; the director is the evolutionary pressure and the storyteller. Together
they're a world that builds itself out over time. Each is useful **alone** — ship Layer 1 first.

---

## 1.5 Foundations — clock + deterministic RNG (BUILT 2026-06-20)

Two primitives underpin everything; both are plain `.ts` (read in the hot path, not reactive), pure, and
unit-tested (`rng.test.ts`, `clock.test.ts` — 24 cases). They make the world a **pure function of
(seed, tick)**: feed a point on the clock and get a deterministic-but-erratic outcome you can reproduce,
trace, and predict — which is exactly what time travel and shareable evolving worlds require.

- **[`clock.ts`](../src/lib/clock.ts) — `SimClock`** (singleton `clock`). A controllable simulation clock
  decoupled from wall-clock: accumulates real frame dt × `rate` into whole fixed-size ticks (`DT = 1/30`).
  Canonical time IS the integer `tick`. API: `advance(realDt)`, `step(n)`, `seek(tick)` (time travel —
  fires `onSeek`), `pause/play/toggle/setRate/reset`, `onTick`/`onSeek` subscriptions. Spiral-of-death
  capped; UI mirrors `tick`/`time` into its own `$state`.
- **[`rng.ts`](../src/lib/rng.ts) — `Rng`** (`new Rng(seed)`). A **stateless, hash-based**
  RNG (Squirrel Eiserloh noise). `rand(...keys)` is a pure function of `(seed, ...keys)` — no stream to
  advance, so you can sample any coordinate directly, in any order. Helpers: `range/int/chance/pick/
  signed/normal/stream`, plus `seedFrom(string|number)`. Key the draws by `(tick, entityId, channel)` and
  the sim becomes replayable.

**Determinism contract** (tested): identical **tick** ⇒ identical world, because all randomness is
`rng.rand(tick, …)` and each tick's update is a pure function of the previous. Time travel = `seek` to a
target, then the sim restores the nearest checkpoint ≤ target and **replays forward** (the `clock.test.ts`
"replays identically after seeking backward" case proves the pattern end-to-end). Frame pacing may put two
machines at different ticks at the same wall-clock instant, but seeking both to tick T shows the same world
— the right guarantee for a shared, real-time, time-travelable sim.

> Backward time travel still needs a sim-side **checkpoint ring buffer** (snapshot every N ticks, restore +
> replay on seek) — sketched in [§5.5](#55-time-travel--checkpoint-ring-buffer--replay-sim-side-deferred);
> deferred to Phase 1+. The clock provides the `seek` signal and `step` replay primitive; the RNG
> guarantees the replay is exact.

---

## 1.6 Wiring the EXISTING sim to the clock + RNG (game chat — required)

For `f(seed, tick)` to hold, **all** nondeterminism in the sim must flow through `clock` + `Rng`. Today it
doesn't (`agents.svelte.ts`/`steering.ts` use `Math.random()` and a variable-dt tick). Two changes, both in
game-owned files:

**(a) Fixed-timestep stepping.** `agentManager.tick(dt)` runs on Threlte's frame loop with a variable
(clamped) `dt` — inherently non-reproducible (frame pacing differs per machine/run). Drive it from the clock
instead, so the sim only ever advances in whole `DT` steps:
```ts
// in the headless AgentSystem (wherever the tick is pumped today):
useTask((dt) => {
  const n = clock.advance(dt);                 // real dt → whole fixed ticks (rate-scaled)
  for (let i = 0; i < n; i++) agentManager.step(clock.tick - (n - 1 - i));  // step each tick in order
});
clock.onSeek(() => simHistory.restoreAndReplay(clock.tick));   // time travel (see §5.5)
```
`tick(dt)` becomes `step(tick: number)` that integrates exactly one `DT`. Rendering keeps reading positions
every real frame (optionally interpolating), so motion stays smooth even though the sim is 30 Hz.

**(b) Seed every draw by (tick, agentId, channel).** Replace `Math.random()` with a module `const rng =
new Rng(world.seed)` and a small channel constant per call-site (so two draws at the same (tick, agent)
don't correlate):
```ts
const CH = { wander: 1, speed: 2, aggro: 3, slash: 4, breed: 5, mutate: 6, fight: 7 } as const;
rng.range(s0, s1, m.seedId, CH.speed)           // was speedFor()'s Math.random()  — birth-time (no tick)
rng.chance(AGGRO_PROB, m.seedId, CH.aggro)       // was Math.random() < AGGRO_PROB   — birth-time
rng.chance(FIGHT_CHANCE, tick, m.seedId, CH.fight)   // per-tick roll keys by tick too
// wander jitter in steering.ts: rng.signed(tick, seedId, CH.wander) instead of Math.random()
```
This requires a **stable per-agent `seedId: number`** (uint32) on `ManagedAgent`, assigned at spawn —
`hash(parentSeedId, birthTick)` for offspring (see [`rng.ts`](../src/lib/rng.ts) `hash`), or a deterministic
counter for the founders. `seedId` is what makes each agent's stream independently addressable + replayable.

> Rule of thumb: **birth-time/per-individual** rolls (speed, aggression, slashMax) key by `seedId` only →
> fixed for that individual's life. **Per-tick** rolls (wander, fight checks) also key by `tick`. Anything
> that touches agent positions or saved/shared state MUST go through `rng` (positions feed the live snapshot
> + share link). Purely cosmetic effects that never touch saved state may stay on `Math.random()` — but when
> unsure, use `rng`; it's the same cost.

Migrate the known `Math.random()` sites: `speedFor`, `makeManaged` (`aggressive`, `slashMax`) in
`agents.svelte.ts`, plus any wander/jitter in `steering.ts`.

---

## 2. Layer 1 — the Metabolism (pure simulation)

We already have ~70% of this in [`agents.svelte.ts`](../src/lib/agents.svelte.ts): trophic ranks
(`ECO`), hunt/flee steering, stamina, `catch = death`, food-coma sleep, mobbing, rivalry. What's missing
are the **loop-closers** that turn a static cast into a cycling population.

### 2.1 Energy economy (closes the thermodynamic loop)
Today nothing is gained or lost — animals just exist. Add a single scalar per agent and a regrowing food
field so energy actually flows herbivore ← plants and carnivore ← herbivore.

- **`energy: number`** on `ManagedAgent` (0..1, seeded ~0.6). Drains at a per-kind **`metabolism`** rate
  every tick (bigger/faster bodies cost more); `energy ≤ 0` → death (starvation), same path as `health ≤ 0`.
- **Food field** — a coarse 2D grid (e.g. 4 m cells over the active area) of `grassEnergy[cell]` in 0..1,
  regrowing at `REGROW/s` capped at 1. A herbivore standing on a cell **grazes**: transfers grass→energy,
  depletes the cell. This is cheap (one array, integer cell math) and reuses the world-stable cell hashing
  the grass/terrain already use (follow-the-player pattern, see `game-work-queue` memory).
  - Only meaningful where `world.ground === 'grass'`; deserts/snow starve grazers (emergent biomes).
- **Carnivores** already "eat" on a kill (`m.meals++`). Convert that to an **energy gain** (`+= prizeOf(prey)`)
  so a fed predator stops hunting and a starving one takes risks. Keeps the existing food-coma.

Result: populations now have a *reason* to grow (food surplus) and shrink (starvation) instead of flatlining.

### 2.2 Reproduction — sexual, gendered, EVERY kind incl. humans (births balance deaths)
**Decision (2026-06-20):** metabolism + reproduction apply to **every** agent — ambient AND hand-placed,
animals AND people ("animals have their own mind, apply it to all"). Nothing is a static exhibit: a placed
lion can starve, a placed couple can have kids. Reproduction is **sexual + gendered**.

- **Gender.** Every agent gets a `sex: 0 | 1` set deterministically at spawn (`rng.chance(0.5, seedId,
  CH.sex)`). For people it's visible (§5 — small build/proportion tweak); for animals it's just a breeding tag.
- **Age + growth.** Offspring are born **small** and mature over `MATURE_TICKS` (render size = `gene.size ×
  growth(age)`, ramping ~0.5 → 1). Only **adults** breed. "Small humans" literally render as scaled-down
  people who grow up. (Optional later: old-age frailty/death; leave out of v1.)
- **Pairing.** An **adult** with `energy > BIRTH_THRESHOLD`, off cooldown (`breedCd`), and an **opposite-sex
  adult of the same kind** within mating range (reuse the flocking grid — neighbours are already iterated in
  `#flock`) **spawns one offspring**; both parents pay energy (no free lunch). Per-kind `breedCd`/`litter` →
  rabbits multiply fast, apex predators slowly.
- Offspring registers via `agentManager.register(makeManaged(… seedId = hash(parentSeedId, birthTick) …))`,
  inheriting a blend + mutation of the parents' **genome** (§2.4), spawned adjacent.
- This produces real **Lotka–Volterra cycles** (rabbits boom → cats/lions boom → rabbits crash → predators
  starve → recover) AND, for people, visible **generations** — the clearest possible proof the world evolves.

### 2.3 Contention (caps runaway growth)
- **Food is finite** (the grazing field) → herbivores compete; a dense patch is eaten bare → carrying
  capacity emerges naturally. No explicit cap needed for prey.
- **Space/territory**: the existing separation + apex-rivalry (`RIVAL_PATIENCE`) already model crowding;
  reproduction should be **suppressed when local density is high** (read `m.crowd`, already computed).
- **Hard perf cap**: keep a global agent ceiling (the manager already targets ≤~120 near / 1000s far). When
  at cap, births fail (resource-limited) — and the **Director** is told "at capacity" so it can intervene.

### 2.4 Genome + mutation (the "evolves itself" part)
Make the per-kind `ECO` entry the **species baseline**, and give each individual a small **genome** of
multipliers that drift on birth:

```ts
interface Genome {
  speed: number;       // ×ECO.speed range      (faster prey escape; faster hunters catch)
  size: number;        // ×render scale + radius (big = more energy stored, costs more metabolism)
  metabolism: number;  // ×energy drain          (thrifty vs greedy)
  aggression: number;  // people: P(hunt); prey: boldness near predators
  fertility: number;   // ×breed rate
}
```

On birth: `child.gene[k] = clamp(parent.gene[k] * (1 + N(0, MUT_RATE)))`. Selection is automatic — genes
that survive predation/starvation/competition propagate. Over minutes you can *watch* a prey population get
faster, or a predator lineage get thriftier. Render `size` by scaling the existing `{#snippet}` critter
meshes; everything else is just arithmetic in the tick.

### 2.5 Stability — don't let it go extinct or explode
Emergent ecologies are fragile. Mitigations (cheapest first):
- **Soft floors**: when a prey kind drops below `MIN_VIABLE` (e.g. 2), boost its breed rate / pause its
  predators' interest briefly — prevents a death spiral from a single unlucky run.
- **Food regrowth rate** is the master dial for the whole system's tempo — expose it as a tunable.
- The **Director** (Layer 2) is the real safety net: it sees a crash/explosion in the population summary
  and reseeds or rebalances (§3.4). This is the satisfying answer — *the AI keeps the world alive*.

### 2.6 New `ManagedAgent` fields (Layer-1 surface, additive)
```ts
energy: number;        // 0..1, starvation at 0
metabolism: number;    // baseline drain /s (from kind × genome)
breedCd: number;       // s until it can reproduce again
sex: 0 | 1;            // gender — set at spawn; gates opposite-sex pairing (§2.2)
age: number;           // ticks since birth → growth(size) + adult/breeding gate
gene: Genome;          // heritable multipliers (§2.4)
familyId: number;      // lineage group (couple + kids) → home anchor + city formation (§2.7)
homeId?: string;       // the world-object id of this family's built home, once they've built one (§2.7)
```
All seeded in `makeManaged()`, evolved in `tick()`. No change to the *reactive* surface — these are hot-path
fields like the existing eco state, read by `Critter.svelte` only for `size`/poses.

### 2.7 Emergent construction — families build homes, clusters self-assemble into cities
**Decision (2026-06-20):** people aren't just mouths — they're **builders**. A family that banks surplus
energy spends it to build, and enough families in one place grow into a town **with no player input** ("type
nothing, watch a city rise"). Reuses the EXISTING deterministic generator in
[`city.ts`](../src/lib/city.ts) (`cityOps` — concentric rings of collision-resolved, shareable Ops), so it's
coherent layout, not box-spam.

- **Family = lineage.** `familyId` (shared by a breeding pair + their kids; offspring inherit it) groups
  agents; a family pools energy.
- **Build = an energy SINK.** When an adult family has **no `homeId`** and pooled `energy > BUILD_COST` and an
  open spot is near, they **spend the energy** and place a home — emit an `add` op (`cabin`/`house`) through
  `applyOps` (deterministic `findFreeSpot`, rng-keyed by `(familyId, tick)`). The new object's id becomes the
  family's `homeId` + their NPC home-leash anchor (ties into the existing `homeRadius` wander in `Npc.svelte`
  — a housed family stays near home; nomads keep roaming). The cost gate means cities rise only where food is
  rich enough to bank a surplus → **emergent economics**.
- **City self-assembly.** A cheap periodic pass detects **clusters of homes** (≥ `TOWN_MIN` within radius R,
  via the existing spatial hash) and grows shared infrastructure by calling **`cityOps`-style ring logic**
  anchored at the cluster centroid: streets (`addPath`) linking homes, a central `well` + `plaza` (`addZone`),
  `lamp`s, eventually a `tower`/`fence` wall. Same look as the `🏙️ make city` command — but grown organically
  by the population.
- **These ARE world objects** (saved + shareable + tick-stamped in the intervention log, §4) — UNLIKE ambient
  offspring. The object list genuinely grows as civilization builds; that's the point.
- **Who runs it?** The *structure* is a deterministic SIM rule (always-on, reproducible, no LLM) so a city
  rises even with the director off. The **Director** (Layer 2) only layers flavour on top — narrating ("a
  village has taken root by the lake"), naming, or nudging growth — never gating the building.
- **Caps (must-have or it explodes).** `BUILD_CD` per family, a global `MAX_STRUCTURES`, build cost that RISES
  with local density (towns plateau), homes only on clear/dry/sensible ground (engine placement enforces it).
  Without caps: object explosion → frame death + a giant share link.

### 2.8 Death + decay at every scale (the full cycle of life)
Decay isn't decoration — it's what keeps the world **bounded** (state churns instead of growing forever) and
it completes the cycle. Birth AND death at every level:
- **Organisms:** predation + starvation (have them) + **old age** — `age > lifespan(kind × gene)` → die.
- **Settlements:** when a town's families die out or migrate, the population near its structures drops; an
  **abandonment timer** runs them through dilapidate → ruins → removed (the inverse of §2.7 construction).
  Real ghost towns that fade — and the [Big World](./big-world.md) reuses this as its Death-Stranding
  "unused structures crumble" mechanic + delta garbage-collector.
- **Vegetation:** treat forests as a slow population — seed/spread when dense + healthy, **recede when they go
  sparse** (too few neighbours to reseed) or the biome turns. Forests advance and retreat over time.
- Net effect: **growth + decay = a churning steady state**, not monotonic accumulation — bounded memory AND a
  world that visibly rises and falls.

---

## 3. Layer 2 — Mother Nature (the LLM director)

**In code this is `MotherNature`** — the evolutionary engine / the slow hand that nudges the world "in a
certain direction every once in a while." (The fine-tuned LLM is its brain.) She observes the world on a slow
cadence and emits **the exact ops the model already speaks** ([`engine.ts` `Op`](../src/lib/engine.ts)) to
shape it — never touching the per-frame sim, only nudging the macro state. In the shared
[Big World](./big-world.md), Mother Nature and other players blur into one felt presence — "ghostly forces."

### 3.1 Cadence, authority & control — BAKED IN, not a separate mode
**Decision (2026-06-20):** the living world is the DEFAULT single world — not a mode you toggle into. The
metabolism (§2) is just physics and runs **always**; the player and the AI **co-direct the same world**. The
"watch it evolve" experience is just a **time-lapse lens** (detach camera, raise `clock.rate`) on that one
world, not a separate save. What stops co-direction from feeling like the AI vandalises your build: separate
the three actors by **AUTHORITY, not by mode.**

| Actor | May touch | Cadence |
|---|---|---|
| **Metabolism** (sim) | ambient life + family-built homes/towns (**add-only**, §2.7) — never moves/deletes *your* objects | every tick, always on |
| **You** (player-director) | everything — build / move / delete / paint | whenever you act |
| **AI director ("god")** | **ADD-ONLY**: scatter life, events, season, narration. NEVER removes/moves/paints your objects | **debounced to your activity** |

That add-only restriction is the trust contract — the AI can stir the world but can't wreck your build.

- **Debounced "god".** The director's intensity keys off player idle time (track last build + last input):
  actively building → suppressed (narration-only); idle ~30–60 s → gentle nudges; idle long / tab hidden /
  explicit "let it run" → full god mode (migrations, new species, droughts, seasons) + optional time-lapse.
- **Cadence = wake / act / sleep (NOT a fixed interval).** Mother Nature *wakes, does her thing, then sleeps
  a **bounded-random** duration*, then wakes again — an organic rhythm, not a metronome. The nap length is
  `rng.int(MIN_SLEEP, MAX_SLEEP, tick, CH.nature)` ticks → erratic but fully deterministic + reproducible.
  **This is the clock + RNG's first real consumer** (it answers "the clock isn't used yet"): she subscribes to
  `clock.onTick` and acts once `tick ≥ nextWakeTick`, then rolls her next nap. Player activity *stretches* the
  nap (long sleeps + gentle acts while you build; shorter + bolder when you're idle → "god takes over"); she
  also pauses while the tab's hidden or the LLM is mid user-build (single-flight). In the
  [Big World](./big-world.md) this maps **1:1 onto a Durable Object Alarm**: set an alarm for the next wake,
  evolve the region, schedule the next nap — wake/act/sleep is literally how DO alarms work.
- **Legibility.** On return from idle, surface a short *"while you were away…"* chronicle so changes read as
  authored, not as glitches — this one line turns "the AI moved my stuff" into "whoa, look what happened."
- Reuses the existing `WorldLLM` engine (`llm.svelte.ts`) — **no second model load**; the director is just
  another `generate()`-style caller with its own prompt, in the same Web Worker.

### 3.2 The observation (cheap, structured — NOT the whole world)
The Director gets a compact **ecology summary**, not the full object list. Crucially, this is **read-only**
and can be computed from already-public APIs, so it needs **no edit to `agents.svelte.ts`**:

- Population counts per kind, alive/asleep/dead — from `agentManager.forEach()` (already public).
- Deltas since last cycle (births, deaths, who ate whom) — Director keeps its own previous snapshot and diffs.
- Scarcity signals — avg `energy` per kind, total agents vs cap. (Reading `energy` needs it to exist → Layer 1
  first, or the Director runs in "counts only" mode until then.)
- World facts from `world` directly: `ground`, `sky`, zone/water count, object count.

```ts
interface EcologySummary {
  tick: number;
  ground: string; sky: string; objects: number; water: number;
  pops: { kind: string; alive: number; asleep: number; avgEnergy?: number }[];
  since: { births: number; deaths: number; starved: number; topPredator?: string };
  atCapacity: boolean;
}
```

A `worldSummary(world): EcologySummary` builder (new file) produces this in a few lines.

### 3.3 The director prompt & output
A **new system prompt** (separate from `buildWorldState`) frames the model as a world director, gives it the
`EcologySummary`, and asks for ops + a one-line chronicle. Output stays in the **existing ops grammar** so it
runs through the same `SCHEMA`/`isValidOp`/`applyOps` path — zero new engine code for events. Example shape:

```
You are the ecology director of a living world. Given the population report, optionally intervene with a
FEW ops to keep it alive and interesting, and narrate what's happening in one sentence.
Reply ONLY {"ops":[...], "story":"..."}.
```

What it emits, mapped to **existing ops** (no new vocabulary needed for v1):
- **Migration / reseed** a crashed prey kind → `scatter` (kind, count, area).
- **Drought / bloom** → `remove` water / `addZone water` / `setGround`.
- **Season / mood** → `setSky` (day↔night↔fog) — and the sim already reacts to night via `setNight()`.
- **Narration** → the `story` field → a **Chronicle** ticker in the HUD (and/or a `note` op, which already
  surfaces as the 💡 banner — see `llm-prompt.ts`).

> The Director should emit **0 ops most cycles** (just a story line). It intervenes only on a real signal
> (crash, explosion, stagnation). Over-intervention makes the world feel puppeted, not alive.

### 3.4 Balancing behaviours (the AI safety net for §2.5)
Encode these as director intents in the prompt (it picks via the summary):
- prey crashed → `scatter` a small founder group of that kind.
- predators starving / extinct → `scatter` a couple, or a prey bloom to feed them.
- everything stagnant (no births/deaths for N cycles) → introduce a new kind or flip the sky/season.
- `atCapacity` → do **not** add; narrate the pressure ("the valley is overrun with rabbits").

### 3.5 Possible new ops (only if v1 proves it needs them — keep the surface small)
- `setSeason: spring|summer|autumn|winter` → a real seasonal modifier on food regrowth + breed rates +
  sky. Today "seasons/weather" is a hard-refused `note`; the Director could make it *real* and cheap.
- `spawnSpecies` with a starting genome bias (e.g. "a faster strain of rabbit migrates in") — sugar over
  `scatter` + gene seeding. Defer until §2.4 lands.

Anything new must be added in lockstep to `engine.ts` (`Op` + `applyOps`) **and** `llm-prompt.ts`
(`SCHEMA` + `isValidOp`) — the single source of truth both the app and the eval battery import.

---

## 4. Determinism & share links (the hard constraint)

The world is **shareable text** (`share.ts` gzips `objects/zones/paths/terrain`). A self-evolving population
must not bloat that link or break reopen-fidelity. Today `liveSnapshot()` already folds *placed* animals'
live position/dead/asleep back into their world objects; *ambient* scatter critters (no `objId`) are not saved.

**Decision for the breeding population: treat it as AMBIENT + SEEDED, not per-individual serialized.**
- The share link stores: the **initial** placed/scattered population (as today), a **`sim` block**
  `{ enabled, seed, elapsed, season? }`, and the Director's **op history is already captured** because its
  events go through `applyOps` (they mutate `world` like any user build).
- On reopen, the sim **re-derives** the current population by running forward from the seed + elapsed time
  (or simply restarts the ecology from the seeded founders — a shared world is a *starting condition*, not a
  frozen frame). Offspring are ambient agents; they are **never** added to `world.objects` (which would grow
  the link unbounded and re-trigger the staggered-mount path in `Scene.svelte`).
- This keeps links tiny and makes "share a world" mean "share a *world that will live*", which is on-brand
  (world = shareable text). The RNG is the seeded `Rng` (§1.5) — **no `Math.random()` in the breeding/
  mutation/build path** if we want two viewers to see the same evolution; otherwise accept per-session
  divergence (cheaper, and arguably fine for an ambient sim — a product call to make).
- **Mortal placed agents + built structures.** Since metabolism applies to everyone (§2.2), hand-placed
  animals/people can now die or breed too — their `objId` still anchors live state in the link
  (`liveSnapshot` already captures pos/dead/asleep), and their offspring stay ambient (unsaved). Family-
  **built** homes/towns (§2.7), by contrast, ARE real `world.objects` → saved + shared, so the link **grows
  as a city rises** (intended — it's content; bound it with `MAX_STRUCTURES`).

Open decision: **deterministic shared evolution** (seeded RNG everywhere, heavier) vs **same start, divergent
life** (per-session RNG, simpler). Recommend starting **divergent** — it's simpler and the wow-factor is
identical for a first viewer.

---

## 5. Rendering the population (the one real engineering lift)

Today a creature is rendered by a component (`Critter.svelte` / `Npc.svelte`) that mounts per `world.objects`
entry and registers a managed agent. Sim-spawned offspring have **no world object**, so they need a renderer
driven directly by the manager — exactly like `Birds`/`Grass`/`AgentImpostors` (ambient, manager-driven):

- **Near ambient critters**: a pool component that asks the manager for the nearest-N ambient agents and
  renders them with the existing per-species `{#snippet}` meshes, scaled by `gene.size`. Cap the pool (e.g.
  40) for perf; everything beyond falls to impostors.
- **Far**: `AgentImpostors.svelte` already instances far agents and skips corpses — it should just work for
  ambient ones too (it iterates `agentManager.forEach`).
- Reuse `sharedAssets.ts` `PRIM` geometries (no new allocations per birth).

This is the piece that genuinely spans both layers; it lives on the game-chat side (it's `Critter`-adjacent).

---

## 5.5 Time travel — checkpoint ring buffer + replay (sim-side, deferred)

`clock.seek(target)` fires `onSeek`; the sim must reconstruct its state for `target`. Forward by stepping is
trivial; **backward** needs state we no longer hold. Because the step is deterministic given the seeded RNG
(§1.6), the recipe is the classic lockstep one: **snapshot periodically, replay the short gap.**

```ts
interface SimSnapshot {
  tick: number;
  agents: AgentRecord[];   // VALUE copy: kind, seedId, x, z, vx, vz, energy, health, genome, flags…
  food: Float32Array;      // the grazing field (copied)
  nextSeedId: number;      // so offspring ids stay deterministic after a restore
}
// plain numbers/strings ONLY — never a reference into a live ManagedAgent (would corrupt on next step)
interface AgentRecord { kind: string; seedId: number; x: number; z: number; vx: number; vz: number;
                        energy: number; health: number; gene: Genome; flags: number; }

class SimHistory {
  #ring: SimSnapshot[] = [];                 // fixed-capacity ring (newest evicts oldest)
  #cap = 300; #every = 60;                   // 1 snapshot / 60 ticks (2 s) × 300 ≈ 10 min scrub window
  #log: { tick: number; ops: Op[] }[] = [];  // interventions (user builds + director events), tick-stamped

  maybeSnapshot(tick: number) { if (tick % this.#every === 0) this.#push(captureSim(tick)); }
  record(tick: number, ops: Op[]) { this.#log.push({ tick, ops }); }   // call wherever applyOps runs

  restoreAndReplay(target: number) {
    const snap = this.#latestAtOrBefore(target) ?? captureInitialFounders();
    restoreSim(snap);                                  // rebuild live population + food field from the copy
    for (let t = snap.tick + 1; t <= target; t++) {
      for (const e of this.#log) if (e.tick === t) applyOps(world, e.ops /*, player */);  // faithful re-apply
      agentManager.step(t);                            // deterministic forward step — same rng(seed, t, …)
    }
  }
}
```

Constraints + decisions (for whoever builds it):
- **Value-copy snapshots** — no aliasing into live agents. ≤~120 agents × ~12 numbers ≈ a few KB/snapshot;
  the whole ring is low single-digit MB. `dlog` the ring size to watch it.
- **Log + replay interventions.** Replay is faithful only if every *external* change (user build, director
  event) is re-applied at the tick it happened — else the timeline diverges from what the user saw. Stamp
  each `applyOps` call with `clock.tick`. Births/deaths/mutations are **not** logged — `step` reproduces them.
- **Bounded replay cost.** A backward scrub replays ≤ `#every` ticks (≤2 s) from the nearest snapshot → fast.
  A jump older than the ring (or a fresh world) replays from the initial founders.
- **Forward seek** beyond `current` has no shortcut (state is path-dependent) — same path: nearest snapshot ≤
  target, then step. Far future-skips just cost the steps.
- **Restore ↔ renderer.** Ambient critters are manager-driven (no world objects, §5), so restore swaps the
  manager's agent set and the pool follows. Placed animals' world objects are unchanged — only their live
  pos/dead/asleep reset (the inverse of the existing `liveSnapshot()` in `agents.svelte.ts`). Director world
  ops (objects/zones) ride along in `world` via the intervention replay.
- **Scope = the session.** The ring + log live in memory; time travel works within a session. Share links
  stay tiny (seed + final world + elapsed, §4) and reopen as "same start, life continues" — they do **not**
  serialize history. Persisting the intervention log for cross-session replay is a possible later add.
- **Prereqs are DONE.** The clock (`seek`/`step`) and the seeded RNG (exact replay) are built + tested; this
  buffer is the only remaining piece, and it can't be built until the `SimState`/`Genome` shape exists (so it
  lands with or just after Phase 1–3).

---

## 6. Performance budget

- Energy/grazing: one scalar add per agent + one grid-cell read = negligible vs the existing O(N·neighbours)
  steering. Food field is one `Float32Array`, regrown by amortizing a fraction of cells per frame.
- Reproduction: gated by cooldown + density → rare per frame. Spawns reuse `makeManaged` (no heavy alloc).
- Director: one LLM call / nap. Runs in the existing Web Worker → never blocks the render loop.
- Keep the global agent cap; surface counts in the debug pipe (`dlog`) to watch frame cost as populations swing.

### 6.5 Concurrency — what runs off the main thread (DECIDED 2026-06-21)
> **Platform target (DECIDED): latest Chrome only — last 2–3 versions — gated** ("works in Chrome" check on
> load). So WebGPU, WASM threads, SharedArrayBuffer, OffscreenCanvas and bleeding-edge APIs are all freely
> usable with **zero cross-browser fallback code**. Caveat that Chrome-only does NOT remove: `COOP`/`COEP` is a
> security *header* (set on Cloudflare), not a browser feature — still required to enable SAB/threads, and it
> still blocks cross-origin resources → keep model weights same-origin (R2). Tighten the existing WebGPU gate
> in `llm.svelte.ts` to say "Chrome only."

Principle: **move pure *compute* (work that produces data) to Web Workers; keep *rendering*, *input*, and
*latency-critical control* on the main thread.** Not "move everything" — moving the wrong things hurts.

- ✅ **LLM — already in a worker** (`llm-worker.ts`). Done. Mother Nature reuses it.
- ✅ **The sim (`agentManager` tick) → a worker** — the big win at scale. Steering / food-chain
  (O(N·neighbours)) / energy / breeding / spatial-hash is pure number-crunching. Run it in a worker, write
  agent transforms into a **SharedArrayBuffer**; the render thread reads that buffer each frame and
  **interpolates via `clock.alpha`** (the sub-tick getter — exactly its purpose). Determinism (`f(seed,tick)`)
  makes this clean: the worker owns the truth; the main thread only feeds `dt`/`seek` and reads back. Do it
  when the population grows (Phase 2+).
- ✅ **Procedural generation → a worker** — region/city/forest/terrain primitives + meshing produce
  geometry/instance buffers; generate in a worker, transfer the buffers in → **hitch-free region streaming**
  (no frame spike realizing a town). Pairs with the deterministic base world ([big-world §4](./big-world.md)).
- ✅ **Big World: a worker owns backend delta sync** (fetch/reconcile region deltas off the render loop).
- ⚠️ **OffscreenCanvas for the main 3D render → SKIP.** Threlte builds the scene graph from **Svelte
  components reacting to `$state` on the main thread** — that can't live in a worker. Full OffscreenCanvas
  rendering ≈ abandoning Threlte (hand-managed scene + forwarded input + cross-boundary raycasting). Renderer
  rewrite, not a perf tweak — not worth it for the POC.
- ⚠️ **Player physics (Rapier) → SKIP** (keep main-thread): tied to input + camera per frame; offloading adds
  a frame of latency → mushy control. It's one light character controller, not the bottleneck.
- **Gotcha — SharedArrayBuffer needs cross-origin isolation (COOP/COEP).** Trivial to set on Cloudflare, BUT
  COEP can block cross-origin fetches (e.g. model weights from a Hugging Face CDN) unless they send
  `Cross-Origin-Resource-Policy`. Resolution: serve models from **our own R2** (same-origin → CORP is ours, so
  COEP is fine — already the plan), OR skip SAB and use **transferable double-buffered `ArrayBuffer`s** via
  `postMessage` (zero-copy, no COEP requirement, a bit more plumbing). R2 path → SAB is the clean choice.

### 6.6 Rust/WASM sim core (URGENT — the FIRST thing built; see §7 Phase 0)
The agent manager's hot loop (spatial hash, steering/flocking, O(N·neighbours) food-chain, energy/breed math)
is exactly the kind of numeric code Rust/WASM eats for breakfast. **Decision: do it FIRST** — port the existing
stable sim to Rust/WASM, verify parity, *then* build the loop-closers (§2) on top.

> **Threading reality (don't be fooled): WASM does NOT run on its own thread.** A WASM export executes
> *synchronously on whatever thread calls it* — call `tick()` from the main thread and it runs on (and blocks)
> the main thread, just faster than JS. Only *compilation* (`instantiateStreaming`) is async/off-thread, not
> execution; WASM threads need explicit Workers + SharedArrayBuffer. So to get the per-frame tick off the
> render thread you still instantiate the `.wasm` **inside a Web Worker** (§6.5). (`engine.ts`'s `applyOps` is
> occasional — a build / Mother Nature nudge, not 60 fps — so it's fine on the main thread regardless; it's the
> per-frame TICK that needs the worker, and only at scale.)

**Realistic perf gain (don't over-promise):** the current JS sim is *already* tuned (typed buffers, spatial
hash, alloc-free, GC-conscious), so the raw single-threaded speedup is a modest **~2–4×** (naive JS would see
5–10×; we picked that fruit already). The bigger wins: **no GC → frame-time *consistency*** (no stutter — for
feel, this beats higher average FPS), **threading ~4–6×** more on the data-parallel pass (§6.8, sub-linear),
**SIMD ~2–4×** on inner loops → stacked, realistically **~5–20× on the sim's CPU budget** at scale. **Amdahl
caveat:** Rust speeds up only the sim, NOT rendering (Three.js draws) — so at POC scale (~120 agents) you're
render-bound and see ≈no change; the win appears at *thousands* of agents. Net effect: raises the **full-agent
ceiling** from ~hundreds to ~thousands–tens-of-thousands; **millions stay aggregate (§6.9) regardless.**
**Measure, don't guess:** once `worldsim` ticks, benchmark `ms/tick` JS vs Rust at N=1k and 10k before banking
a number. (And remember determinism + the unified entity model are guaranteed wins independent of the multiplier.)

**Why it's unusually compelling here** (beyond raw speed):
- **Deterministic across clients.** The sim runs **only in the browser** (NOT on Cloudflare — CF is storage/
  API only, [big-world §3](./big-world.md)). WASM's bit-exact float semantics + the addressed RNG mean any
  visitor fast-forwarding a dormant region from the same `{seed, state, lastTick}` computes the **identical**
  world (client↔client + client-now↔client-later consistency). That's what makes a shared browser-run world
  coherent — and it's why replay / time-travel / lazy fast-forward all hold.
- **No GC** → kills the exact class of jank the JS sim keeps fighting (reused buffers, alloc-free flock force…).
- **Deterministic floats** — WASM IEEE-754 is portable by spec (more reproducible than native); the squirrel
  RNG is pure integer ops → ports to Rust trivially, determinism intact.
- Struct-of-arrays over linear memory → cache-friendly, SIMD-able; slots into §6.5 (WASM-in-worker → SAB →
  render reads + `clock.alpha` interpolation).

**The boundary cost (why it's a real rewrite):** the win only lands if **all agent state lives in WASM linear
memory** and `tick(dt)` is **one call per step** — crossing JS↔WASM per-agent kills it. Agents stop being rich
JS objects; JS becomes thin glue reading typed-array views (transforms + `dead/asleep/lod` flags by index).
Rendering, registration, Mother Nature, and the LLM stay JS. Clean split: **Rust owns sim state + tick; JS
owns everything visible.**

> **"Isn't hitting the boundary every tick slow?" — No, if you cross ONCE per tick, not once per agent.** A
> single JS→WASM call is ~nanoseconds (V8 optimizes it near a normal JS call); one `tick(dt)`/frame is free.
> The "WASM boundary is slow" reputation is about (a) *chatty per-entity* calls — a million crossings/frame —
> and (b) *marshalling* objects/strings. We do neither: state stays resident in WASM linear memory; `tick(dt)`
> passes 2–3 numbers; the renderer reads results **zero-copy** via a typed-array view over the WASM buffer
> (`new Float32Array(wasm.memory.buffer, ptr, n*stride)`) — or, in a worker, over a **SharedArrayBuffer** the
> worker's WASM writes (no `postMessage`/copy per tick). Numbers + shared memory cross for free; only objects/
> strings cost, and the only complex things that cross (a build's `ops`, a Mother Nature nudge) are
> *occasional*, not per-tick. **Gotcha:** if WASM memory **grows** (resize), the old `ArrayBuffer` detaches →
> recreate the views after growth, or pre-allocate entity capacity up front to avoid mid-run growth.

**Timing — DECIDED 2026-06-21: Rust-FIRST** (user call, overriding the earlier "do it later" lean). Rationale
that flips the sequencing: we're **not porting a moving target** — the *current* sim core (spatial hash,
steering/flocking, food-chain, combat, stamina) is already mature + stable, so porting it now is porting a
KNOWN quantity. The new loop-closers (energy/breeding/genome/construction) then get built **directly in Rust
on top** → avoids the build-in-JS-then-re-port double-work, and everything inherits worker/WASM + the shared
CF server tick + bit-exact determinism from day one. Cheap AI authoring defuses the old "Rust iteration is
slow" worry.

**Two conditions (one is a real risk):**
- **Coordinate with the game chat — the thing that bites.** Rust-first moves the engine's HOME to a Rust/WASM
  core + a thin JS binding; new sim features are authored IN Rust. The game chat must **stop parallel JS
  feature-work on `agents.svelte.ts` during the port** (or own the port). Two chats editing the sim in
  parallel — JS in one, Rust in the other — is the one scenario that makes a mess. Make it an explicit handoff.
- **Tunables stay data-driven** — the `ECO` table + steering weights live in a JS/JSON config the Rust reads,
  so balancing stays instant (no recompile).

**Port plan:** do it AS the §6.7 boundary refactor. Rust owns world + sim + clock + rng (the already-pure
modules port cleanly; the squirrel RNG is pure int ops → trivial; floats stay deterministic per WASM spec),
state lives in linear memory, exposed via wasm-bindgen as a `tick(dt)` call + typed-array buffer reads. JS
keeps rendering / registration / Mother Nature / the LLM. THEN build the loop-closers (§2.1–2.8) in Rust and
run it in the §6.5 worker. (Sim is **browser-only** — CF never runs it; it stores the deltas a client commits.)

### 6.7 The headless engine core (the KEYSTONE — enables §6.5, §6.6 + the server tick)
**Decision (2026-06-21): formally split a headless ENGINE CORE (pure, non-reactive, framework-free) that the
Svelte/Threlte VIEW calls.** Rationale: Svelte runes (`$state`) are a **view concern** — they can't run in a
worker (correct observation), so reactivity must NOT be in the compute layer. The clean split makes all the
perf moves above possible at once (you can't move `$state`/Threlte anywhere; a clean engine core you can).

- **Already ~halfway there:** `engine.ts` (`applyOps`/world), `clock.ts`, `rng.ts`, `steering.ts`,
  `spatialhash.ts`, `terrain.ts`, `water.ts`, `scatter.ts`, `world.ts`, `kinds.ts` are pure; **`agents.svelte.ts`
  is already deliberately non-reactive** (plain objects, "NOT `$state` in the hot path") — engine-shaped despite
  the filename. So this is consolidation + walling-off, **not a rewrite**.
- **Reactive ring stays in the VIEW:** the Svelte components (`Scene`/`Critter`/`Npc`/`BuildBar`/HUD) + the
  `$state` singletons (`playerState`, `editor`, `history`, `llm`).
- **Boundary / "call it":** engine owns world + sim + clock + rng + region/terrain gen and exposes `step(dt)`,
  `applyOps(ops)` (player builds AND Mother Nature), input/intent commands, and a **read surface** (direct
  reads main-thread; a transferable/shared buffer when worker-ized). The view reads transforms/state each
  frame → renders + interpolates (`clock.alpha`) + mirrors only HUD scalars into `$state`. Engine = `f(seed,
  tick)`; view = a pure projection.
- **This is the prerequisite for §6.5 (engine→worker) and §6.6 (hot loop→Rust/WASM + SIMD + threads).** One
  enabling refactor. Also just clarifies the codebase: engine = "what is true," view = "how it looks." (Sim is
  browser-only — CF runs nothing; big-world §3.)
- **Caveat:** keep the **player controller low-latency** (view-side prediction / main-thread) even if the
  ambient sim moves to a worker → the split may be "ambient engine in worker / player in view," not all-or-
  nothing.
- **Timing:** this boundary is established **by the Rust port itself** (§6.6 / §7 Phase 0) — it's the FIRST
  thing, not a someday-refactor. The port carves engine-from-view as it goes.

### 6.8 Multithreading the WASM sim (YES — at scale, LATER, thread-count-invariant)
**Does it work?** Yes — WASM threads = shared `WebAssembly.Memory` + Web Workers + atomics; in Rust the easy
path is **`wasm-bindgen-rayon`** (`par_iter`). Needs **COOP/COEP** — the *same* requirement as the SAB plan
(§6.5), so once that's on, threads come ~free. Broad modern-browser support.

**Does it make sense?** Yes, but **only at scale** — the per-tick force pass is data-parallel (build the
neighbour grid, then each agent computes its next state from a *read-only* grid view → `par_iter`s cleanly).
At POC scale (~120) single-threaded WASM is already fast and thread overhead would *lose*; the win is
thousands-of-agents / big-world.

**The hard constraint (north star): parallel must be BIT-IDENTICAL to single-threaded** — else replay /
time-travel / client-server consistency break. Golden rule: **the result is invariant to thread count** (1
thread ≡ 8 threads). Requires:
- **Double-buffer** state (read prev tick, write next) → no same-tick cross-reads, no races, order-independent.
- **Addressed RNG** keyed by `(tick, agentId, channel)` — *already the design*; each draw is addressed (not a
  shared stream), so agents run in any order on any thread and get the same numbers. **This is the enabler.**
- **Thread-count-invariant reductions** — per-agent neighbour sums (fixed grid order) are fine; avoid
  order-dependent global float accumulation (fixed order or integers).

**Server caveat:** Cloudflare's runtime is **single-threaded** — WASM threads/SAB aren't available there like
in the browser. So the *same* `.wasm` runs **single-threaded on the CF server tick** (fine — coarse + low-freq).
Threads are a **client-only scale knob**, and the "invariant to thread count" rule is exactly what guarantees
the 8-thread browser and the single-thread server compute the identical world.

**Sequencing + COMMITMENT (user call 2026-06-21): SIMD + multithreading WILL be unlocked by the end** — a
definite deliverable, not "maybe later." Order: **single-threaded parity FIRST** (Phase 0) → then **SIMD**
(cheaper — build `-C target-feature=+simd128`, autovectorize / `core::simd` the hot inner loops) → then
**threads** (`wasm-bindgen-rayon`, build `-C target-feature=+atomics,+bulk-memory` + shared memory; COOP/COEP
is free since Chrome-only). **Determinism gate — MUST pass before shipping either:** a test asserting output
is **bit-identical scalar-vs-SIMD AND 1-thread-vs-N-thread.** The one risk is order-dependent FLOAT reductions
(SIMD horizontal sums / parallel accumulation reorder rounding) → keep reductions fixed-order or integer;
element-wise SIMD + index-partitioned threads are otherwise bit-exact. Still don't do it *before* parity (no
determinism-bug tax before the core is correct), but it's a tracked end-goal — see §7 Phase 0c.

### 6.9 Scaling to "millions" — unified entities + tiered (LOD) simulation, NOT brute force
**Decision (2026-06-21):** *everything is a mini-agent* — organisms, trees, AND houses share one lifecycle
(age/energy/growth/decay/flags + a `kind` discriminant) in a single struct-of-arrays entity buffer. Uniform +
clean in the Rust core.

**But you cannot full-sim millions per tick — not threaded, not in any browser, not on one server.** A million
neighbour-querying agents at 30 Hz is nobody's budget. Massive worlds (Dwarf Fortress, RimWorld, SimCity, MMOs)
all use **tiered / LOD simulation**:
- **Near (active region): full per-agent sim** — hundreds–low-thousands of real entities. This is what the
  threaded WASM core (§6.8) is for.
- **Far: entities collapse to AGGREGATE fields** — a distant forest = a *coverage/density* number that grows +
  decays; a distant city = a *population stat*; NOT a million individual ticks. The "millions" live as cheap
  per-region aggregates that a **visiting client fast-forwards** on arrival (lazy, deterministic — never a
  server tick; [big-world §3](./big-world.md)).
- **Realize individuals on demand** — approaching a region spawns real entities deterministically from `seed` +
  the fast-forwarded aggregate (the §4 "silhouette → real on arrival" streaming). Leave → collapse back to stats.

**Fidelity split (what persists vs what regenerates):**
- **Durable** things (player/family-built structures, named settlements) persist EXACTLY as DB deltas.
- **Ambient** individuals (the millions of rabbits/trees) are *statistically regenerated* from the seed — you
  don't need the same individual rabbit to survive your absence, only the population trend. Keeps state bounded
  AND makes "millions" tractable + deterministic.

So: unified entity model ✓ + threaded WASM for the active thousands ✓ — the millions are **aggregate, with
individuals realized on demand.** This is the same region/streaming model already chosen, applied to the sim.

### 6.10 Renderer — stay Three.js (the bottleneck is the SIM, not the render) (DECIDED 2026-06-21)
Recurring question: is Three.js the right fit, or could a leaner/"Decima-like" engine do better? **Decision:
stay Three.js.** Reasoning:
- **The renderer is NOT where "millions" lives.** With LOD + aggregation (§6.9) you only ever *draw* a bounded
  near-set (hundreds–low-thousands), never millions. The scaling wall is the **sim** (handled by Rust/WASM +
  LOD + workers), orthogonal to the renderer. Switching renderers would not unlock millions.
- **"Decima-like" is technique, not a library.** Decima's frame-cheap-yet-gorgeous = bespoke clustered/
  GPU-driven rendering + streaming + custom LODs + a decade of AAA pipeline (it also powers Death Stranding —
  but it's native). No off-the-shelf "web Decima." On web you approximate those techniques yourself, and most
  — **instancing, LOD, impostors, few materials, culling** — are renderer-agnostic + already in use here.
- **The real web lever: WebGL2 → WebGPU.** Lower CPU draw overhead + compute shaders (agent transforms/
  particles on GPU). **Three.js already has a WebGPU renderer (TSL)** → this is a path *inside* Three (keep
  Threlte + Svelte components + the shader work), not an engine switch. WebGPU is Chrome-gated anyway (§6.5) →
  fully available. Biggest bang, least disruption — pursue when render cost is *measured* to matter.
- **Decision (2026-06-21): PARKED after a working spike.** A dedicated `webgpu` branch successfully booted
  `WebGPURenderer` and proved TSL terrain/curvature, but visual parity across the project's many hand-tuned
  GLSL patches was too fragile to develop reliably by LLM iteration (valid shaders repeatedly produced wrong
  world-space lighting, fog, LOD grounding, and curvature). **WebGL2 remains the production renderer.** Keep
  improving renderer-agnostic wins — instancing, budgets, culling, LOD, workers, SIMD — and revisit WebGPU only
  when Three/TSL migration tooling is mature enough for systematic visual-regression testing. The spike remains
  isolated in its worktree/branch as research; do not merge it piecemeal into `main`.
- **Not locked in:** the §6.7 engine/view split means the renderer is **not load-bearing** — swap Three.js for
  a unified-Rust engine (**Bevy**: ECS + WebGPU, one language for sim+render, no JS↔WASM render boundary) or
  raw WebGPU **later** without touching the sim, IF render becomes the real bottleneck. But that's a near-total
  rewrite (lose Threlte/Svelte/shaders/app-shell), Bevy-web is still maturing, and it's premature mid-sim-port.
  Flagged long-term convergence, **not a now-move.**

---

## 7. Ownership & rollout

Phased, cheapest-highest-impact first. **Each phase is shippable alone.**

| # | Phase | Owner | Touches | Depends on |
|---|---|---|---|---|
| **0** | **🚨 URGENT — Rust/WASM engine port (§6.6/§6.7)** — port the EXISTING sim to a headless Rust core, verify parity, run it in a worker. Everything else is built ON it. | game chat / coding chat | replaces `agents.svelte.ts` hot loop w/ a Rust/WASM core + thin JS binding; engine-from-view split | §1.6 done first (clock+rng-keyed, fixed-step) |
| 0c | **🔒 COMMITTED — unlock SIMD + multithreading (§6.8)** "by the end" (user). SIMD first (`+simd128`, autovectorize hot loops), then threads (`wasm-bindgen-rayon`, `+atomics,+bulk-memory`, COOP/COEP — free Chrome-only). **Gate: bit-identical scalar≡SIMD AND 1-thread≡N-thread.** | game / coding chat | `worldsim` build flags + rayon + determinism test | 0 + single-thread parity |
| 1 | **Energy + starvation** (§2.1) — built **in Rust** | game chat | the Rust engine (+food field) | 0 |
| 2 | **Reproduction — gender, children, cycles** (§2.2–2.3) — in Rust | game chat | the Rust engine, ambient renderer (§5) | 0, 1 |
| 3 | **Genome + mutation** (§2.4) — in Rust | game chat | the Rust engine, `Critter.svelte` (size) | 2 |
| 3b | **Emergent construction** — families build, cities self-assemble (§2.7) — in Rust | game chat | the Rust engine + `applyOps`, reuse `city.ts` | 2 (nicer w/ 3) |
| 4 | **Director — narration only** (§3.1–3.3) | **this chat** | NEW `director.svelte.ts`, `worldSummary.ts`, HUD ticker | counts only (works pre-1) |
| 5 | **Director — events & balancing** (§3.4) | **this chat** | director prompt + reuse `applyOps` | 4 (+ richer summary from 1) |
| 6 | **Seasons / new ops** (§3.5) | this chat + game chat | `engine.ts`+`llm-prompt.ts` (lockstep), sim modifiers | 5 |
| 7 | **Autopilot / time-lapse mode** (§3.1) | this chat | camera + clock `rate`, "Living World" toggle | 2, 4 |
| 8 | **Time-travel checkpoint/replay** (§5.5) | game chat | `SimHistory` — with a Rust engine, a snapshot is just a **copy of the WASM linear-memory buffer** (trivial + fast) | 0–3 |

**Order of operations:**
1. **Foundations (DONE):** `clock.ts` + `rng.ts` (built + tested).
2. **§1.6 first** — drive the *current* sim from `clock` at fixed `DT` + replace `Math.random()` with seeded
   `rng` keyed by `(tick, seedId, channel)`. This makes the sim deterministic + worker-ready, which is the
   precondition for a clean port. (Can be folded into the port itself.)
3. **🚨 Phase 0 — the Rust/WASM engine port (URGENT, FIRST).** Port the existing stable sim to a headless Rust
   core, verify parity with the JS behaviour, run it in a worker. **Coordinate with the game chat — they must
   pause JS feature-work on `agents.svelte.ts` during the port** (two chats editing the sim in parallel, one
   JS one Rust, is the failure mode). See §6.6.
4. **Everything else (Phases 1+) is then built IN the Rust engine**, inheriting worker/WASM + the shared CF
   server tick + bit-exact determinism for free.

**Clean seam:** Phase 4 (the LLM Director, narration) needs **no `agents.svelte.ts` edit** — it reads
populations through the already-public `agentManager.forEach()` and `world`. So this chat can build a
genuinely magical first cut (a live chronicle of the existing food chain — "the lone dinosaur is closing on
the rabbits by the lake") **in parallel**, today, with zero collision. The sim loop-closers (1–3) land on the
game-chat side whenever it's free.

---

## 8. Failure modes & answers
- **Extinction cascade** → soft floors (§2.5) + Director reseed (§3.4).
- **Population explosion → frame death** → finite food + hard cap + Director narrates pressure instead of adding.
- **Director feels puppeted** → bias it to 0 ops/most cycles; intervene only on real signals.
- **Share links bloat** → offspring are ambient, never `world.objects` (§4).
- **Non-deterministic shared evolution** → accept divergence for v1, or seed all RNG later (§4 open decision).
- **LLM latency stalls the world** → Director runs async in the worker; the sim never waits on it.

## 9. Open decisions (need a product call)
1. Shared evolution: **deterministic** (seeded) vs **divergent** (per-session). → recommend divergent for v1.
2. Chronicle surface: reuse the `note` 💡 banner vs a dedicated scrolling **Chronicle** panel. → recommend a
   small dedicated ticker so director narration doesn't fight user-build notes.
3. ✅ DECIDED: **baked in** (not a separate mode) — metabolism always on; AI director is add-only + debounced
   to player activity ("god" takes over when idle); time-lapse is just a lens on the same world (§3.1).
4. ✅ DECIDED: metabolism applies to **ALL** agents incl. hand-placed + humans (no "exhibit" exemption); humans
   are gendered and breed into children that grow up; families that bank energy **build** homes that cluster
   into self-making cities (§2.2, §2.7). Placed agents can die/breed; offspring stay ambient; built structures
   are saved world objects.
5. Construction aggressiveness — `BUILD_COST`/`BUILD_CD`/`MAX_STRUCTURES`, how fast/dense cities grow, how big
   the saved state may get. → tune live; hard-cap structures.
6. **Persistence + world scale** (huge pre-authored world, ever-growing, full life-cycle) — see the separate
   `docs/big-world.md` spec; the open fork is local-deterministic vs a server DB (revisits non-negotiables).
