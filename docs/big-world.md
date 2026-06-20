# The Big World — one shared, persistent, ever-living world (design spec)

> **Vision (decided 2026-06-20):** not per-user sandboxes — **one single, vast, persistent world** that
> everyone quietly shares and shapes, that keeps living and evolving even when no one is there. You **never
> see other players** — only their *effects* (structures, cleared forests, grown cities, ruins). Like **Death
> Stranding**: you're alone with ghostly forces. The AI "director," the sim, and other humans all blur into
> one felt presence — "the world breathing around you."
>
> This is the sequel to [`self-sustaining-world.md`](./self-sustaining-world.md) (the per-agent living sim) and
> [the deterministic foundations](../src/lib/clock.ts) (clock + RNG). It is a **deliberate product pivot**:
> see [§7 Identity pivot](#7-identity-pivot--cost-reality). Status: **DESIGN ONLY**, no code.

---

## 1. Why the async / no-presence model is the right call
Real-time multiplayer is the expensive part: presence, avatar sync, netcode, lag comp, live anti-cheat.
**Death Stranding's asynchronous model deletes all of it** — nobody is online *with* you; you read + write
**durable changes** to a shared world on your own time. It's not an MMO, it's *a database with a world
painted on it*. And because you only ever see *effects*, the player **cannot distinguish** another human's
work from the AI director's from the sim's own evolution — they all read as "the world living around me."
That ambiguity is the design, not a limitation; it makes [the director](./self-sustaining-world.md#3-layer-2--the-director-the-llm)
and the other players the *same* ghostly presence.

## 2. The architecture: deterministic base + DB delta
The trap is "store the whole giant world in a DB." Don't. Split it in two:

- **Deterministic BASE world (client-side, free, never stored).** The topology — biomes, where the jurassic
  valley sits, where cities *can* form — is `f(worldSeed, regionMap)` (see §4), generated on each client from
  a single shared **world-seed**. Identical for everyone. Costs zero storage and zero bandwidth.
- **DB stores only the durable DELTA.** What diverged from the base: structures built/destroyed, a
  settlement's state, population summaries, "this patch of forest was cleared." Every client layers the same
  delta over the same base → everyone sees the same world. The delta is what's persistent + shared + durable.
- **Decay garbage-collects the delta** (§5). Abandoned things fade and are reclaimed → the delta (and the DB)
  stays **bounded** even though the world is "ever-growing." Growth + decay = a churning steady state.

> This is the project's founding principle — **"ops/regions, not geometry"** — applied to the network layer:
> ship the *recipe + the diff*, never the millions of objects.

## 3. Who runs the sim — server-authoritative durable layer
For "the world keeps evolving while you're away," the durable layer must be **server-authoritative**:

- A **backend sim tick** (cron / worker) advances the MACRO world on its own schedule: settlement growth +
  decay, population summaries per region, biome spread/recession, big director events. It writes deltas to
  the DB. This is the coarse, low-frequency truth (minutes/hours, not 30 Hz).
- **Clients render + run the FINE detail locally**, deterministically from `(worldSeed, region, tick)` + the
  durable deltas they fetch: per-agent walk/flee/graze motion, particle/shader life. None of this hits the
  server — it's cosmetic confabulation over the authoritative coarse state (same trick as
  [§1.6 fixed-timestep determinism](./self-sustaining-world.md#16-wiring-the-existing-sim-to-the-clock--rng-game-chat--required)).
- **Authority rule** (carried over from the per-user spec, now networked): the server owns durable structures
  + settlement/population truth; clients may *propose* changes (the player's builds, locally-simulated births)
  which the server validates + commits. Player edits and AI-director ops both become committed deltas.

### 3.5 Backend = Cloudflare (DECIDED 2026-06-20)
Cloudflare Workers (paid/Standard) + **Durable Objects** — DOs are almost purpose-built for this.

> **A "region" is a spatial TILE of the ONE world (like a map chunk) — NOT a server/shard players choose.**
> There is exactly one canonical world; you're simply *in* whatever region your coordinates fall into and
> cross **seamlessly** into the next as you walk. DO-per-region is sharding for **scale + consistency** (one
> DO can't hold/serve a whole continent or take all its writes), invisible to the player. The whole map = the
> **union** of all region DOs — there is no "merge into one pool" step; the tiles *are* the one pool. This is
> the OPPOSITE of MMO realms (parallel copies where players never meet). Cross-tile interactions (a migrating
> herd, a city straddling a boundary) need a small neighbor-awareness protocol between adjacent DOs.

- **Worker (API)** — edge endpoints: clients read region deltas + silhouette summaries; submit op commits.
- **Durable Object per region** (keyed by region cell) — the authority:
  - **Owns the region's delta** (DO transactional storage) and **serializes all writes** → single-threaded per
    region, so **write-conflicts can't happen** (no locks, no LWW races). Player builds + director ops commit here.
  - **Runs the macro sim tick via Alarms** — each *active* region self-schedules an alarm (every few min) to
    advance settlements/populations/biomes + decay (§5 / self-sustaining §2.8). **Idle regions hibernate** (no
    alarm = no cost) → you tick only the living world, not all of it. This is "evolves while you're away,"
    distributed for free.
- **Storage:** DO storage for each region's delta (colocated w/ authority); **KV** for hot eventually-
  consistent read caches (far-silhouette summaries); **D1** for cross-region queries (a world map of cities);
  **R2** for cold region snapshots/exports. Client still generates the deterministic base from `worldSeed` and
  fetches only a region's *delta* on stream-in → DB stays small (deltas, pruned by decay).

**DDoS — free + automatic** (being behind Cloudflare = L3/4/7 mitigation on by default; nothing to build).
**Abuse — platform primitives + a little wiring:** WAF + **Rate Limiting Rules** on the write endpoint;
**Turnstile** (no-friction human check) on session + writes; sign writes w/ nonce+timestamp → no replay;
**decay as anti-grief** (spam fades, §5) + per-region DO snapshots → a mass-grief is **reversible**.

**Op budgets — nobody can flood the world (DECIDED 2026-06-20).** With direct destruction off the table
(§5.5), "destroy the world" = spam-flood it; layered caps stop that:
- **Edge:** a coarse **per-IP request cap** (CF Rate Limiting Rules) on the write endpoint — first line, also
  soaks up floods for free.
- **Per-session/identity budget**, enforced **inside the region DO** (single-threaded → race-free): a
  token-bucket per **anonymous device-token** — small burst/minute + sustained cap/hour. IP is only a
  *backstop* (CGNAT/shared IPs make IP-only unreliable; the device-token is the real unit).
- **Global footprint cap:** one actor owns at most **K live structures** at a time → to build more, their old
  ones must decay or be dismantled. This is the actual "can't blanket the map" guarantee.
- **Per-region write cap:** bound how fast any one actor mutates a *single* region, so nobody dominates a town.
- Over-budget / low-effort builds **decay faster** (§5 / self-sustaining §2.8). Budget + decay + reversibility
  = three independent backstops.

## 4. The huge world itself — regions, primitives, streaming
(From the topology discussion — the same engine whether single- or multi-player.)

- **Region map:** the world is a large but FINITE grid; `regionType(cx, cz) = f(worldSeed, low-freq noise, a
  few hand-authored anchors)`. Hand-place the set pieces ("jurassic valley here, three cities along this
  river"), let seeded RNG fill + vary the rest → designed yet vast. Tiny to store, expands deterministically.
- **Place primitives = parameterized generators we ALREADY have:** `cityOps`, `forestOps`, `lakeOps` in
  [`city.ts`](../src/lib/city.ts) are exactly this (deterministic, collision-resolved, shareable Ops). Add
  `villageOps`, `jurassicOps`, `mountainsOps`, …; vary each instance's params by seeded RNG so every city is
  unique but unmistakably a city. **Variety × primitives = feels infinite**; the finite map = "so darn big"
  but bounded.
- **Streaming = the "silhouette that's real when you arrive" requirement, solved:** far → a cheap
  **silhouette/impostor** drawn straight from the region map (reuse the existing `Skyline` + `AgentImpostors`
  patterns); mid → coarse proxies; near → fully realize the region (run the primitive, spawn real objects,
  wake the sim + fetch that region's durable deltas). It's real on arrival **by construction** — the
  silhouette and the realized town come from the *same* definition, so there's no bait-and-switch.
- `curveWorld.ts` (the Inception fold) + fog already hide the finite far edge → the horizon reads endless.

## 5. The full cycle of life — death + decay at every scale
Decay is load-bearing (it bounds the DB, §2) and the theme. Every scale gets birth AND death:

- **Organisms:** predation/starvation (already in the sim) + **old age** (`age > lifespan → die`).
- **Settlements:** when a town's families die out or migrate, the population near its structures drops → an
  **abandonment timer → dilapidate → ruins → reclaimed** (deltas removed). Ghost towns that genuinely fade.
  This *is* the Death-Stranding "structures nobody sustains crumble" mechanic.
- **Vegetation:** treat forests as a slow population — they seed/spread when dense + healthy and **recede when
  they go sparse** or the biome shifts. Forests advance and retreat over (server) time.
- All three: the server sim tick advances growth + decay; clients see the deltas. Bounded, alive, churning.

## 5.5 Ownership & destruction — can someone remove your build? (DECIDED)
**Decision (2026-06-20): NO direct deletion of other people's structures.** Destruction is something the
*world* does, not a button you point at a stranger's house. This is the Death-Stranding model and it's what
keeps the shared world from becoming a griefing warzone — the "quiet co-building" magic dies the instant a
stranger can nuke your city. **Players are add-only toward each other** (mirrors Mother Nature's add-only
authority in [self-sustaining §3.1](./self-sustaining-world.md#31-cadence-authority--control--baked-in-not-a-separate-mode)).

Removal happens ONLY via:
- **The owner** — you can always dismantle your *own* builds.
- **Natural decay / abandonment** (§5 / self-sustaining §2.8) — the *primary* remover; unsustained things
  crumble → ruins → reclaimed. The cycle of life prunes for you, so player-deletion is rarely even needed.
- **In-world forces** — Mother Nature disasters (drought/fire/flood) + the sim (settlement war, a stampede
  razing a village, a city starving out). The player *influences* these indirectly (lure the herd, let a town
  starve) but never issues a direct "delete that" on another's work.

**Reversibility:** every removal is a durable delta + the region DO keeps snapshots → a malicious/buggy wipe
is rollback-able. Destruction is never silently permanent.

**Live-ness:** **eventually-consistent by default** — you *arrive to find the world changed*, you don't watch
it happen. If you happen to share a region's DO with someone, the DO *may* push a live update (a structure
crumbles before your eyes) shown as a "ghostly force" (§6) — but real-time co-presence is **never required or
guaranteed** (that would drag back the netcode the async model exists to avoid). Live-when-present = nice-to-
have; eventually-consistent = the contract.

**Optional future (NOT v1): contested/earned destruction** — tearing something down requires investment (a
siege, your settlement out-competing it via the sim), never a free instant delete → dynamism without trivial
griefing.

## 6. Rendering the ghosts (taste, decide later)
Spectrum from invisible → hinted:
- **Fully invisible** (purest): you simply find the world already changed.
- **Hinted (Death-Stranding-like):** a structure faintly shimmers in as it commits; sparse "apparition"
  wisps where others recently acted; an optional appreciation/like signal on helpful builds. Recommend a
  *little* hinting — it makes the shared-presence legible without breaking the solitude.

## 7. Identity pivot + cost reality (read before committing)
This consciously **retires two of the four original non-negotiables** ([[project-identity]]): `local & free`
and `world = shareable text link`. What you're taking on instead:

- a **database** + a **server-side sim loop** (cron/worker ticking the macro-world),
- an **API** + read/write of durable deltas, lightweight identity (even anonymous), **write-conflict** handling,
- **abuse / moderation** — a shared mutable world *will* be griefed; need rate limits, decay-of-bad-builds,
  region ownership or reversibility,
- **ongoing hosting cost** + scaling (the world's delta + the sim tick grow with the player base).

Net: it's no longer a weekend-local toy — it's a real backend product. **But the positioning is arguably
more viral:** *"one quiet world we're all secretly building together, that lives on without us."* Stronger
hook than "local sandbox." Go in eyes-open on the cost/ops; that's the trade.

## 8. What still carries over (nothing wasted)
- **Clock + RNG** ([`clock.ts`](../src/lib/clock.ts), [`rng.ts`](../src/lib/rng.ts)) — the deterministic base
  world + client-side fine sim both need exactly this. Already built + tested.
- **The living-sim metabolism** (self-sustaining-world.md §2) — runs per realized region; the server runs the
  coarse version, clients the fine version.
- **The op grammar + `applyOps`** — durable deltas ARE ops (add/remove/addZone/addPath…); the DB stores op
  deltas, `applyOps` replays them. The fine-tuned LLM still authors ops; the director still narrates.
- **city.ts primitives, Skyline, AgentImpostors, curveWorld, the shader work** — all reused as-is.

## 9. Open questions
1. ✅ **Backend stack** — DECIDED: Cloudflare (Workers + DO-per-region + KV/D1/R2), see §3.5.
2. **Identity** — anonymous device id vs lightweight accounts (needed for moderation + appreciation).
3. **Region commit granularity** — per-object deltas vs per-region snapshots in the DB (affects conflict + cost).
4. **Ghost visibility** (§6) — invisible vs hinted; how much.
5. **Moderation model** — region cooldowns, reversibility, decay-of-griefing, reporting.
6. **Cost ceiling** — how big a world/playerbase before it needs real money; is there a free tier shape.
7. **Delta curation in dense regions** — one canonical world, but a hyper-popular region could accumulate
   millions of builds (visual mush + perf death). Show **all** vs a **curated/sampled subset** per viewer
   (Death-Stranding shows a sampling, not everything). → lean curated: favour nearby / recent / high-
   "appreciation" / same-lineage deltas. Keeps "one world" true without burying a popular spot.

---

*Prereqs already shipped:* the deterministic clock + RNG. *Natural build order:* finish the per-user living
sim (self-sustaining-world.md Phases 1–3b) → prove regions/streaming/silhouettes **single-player** → then add
the server delta layer + shared persistence on top. Don't build the backend until the world is worth sharing.
