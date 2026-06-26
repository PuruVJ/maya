# Ideas Queue — the living-world wishlist

A safe, persistent home for every idea the user has raised, so none are lost and they get worked through in order.
Newest ideas added at the top of "Queued". Move items to "Shipped" with the commit when done.

> Working agreement: build them ONE AT A TIME, well + tested (55 cargo tests, `svelte-check` 0/0), commit each.
> Several of these are HUMAN-SOCIETY facets that interact — design them to compose, not contradict.

---

## 🟢 Queued (priority order — top = next)

### 2026-06-24 BIG DESIGN SESSION — emergent-world reproduction + tuning (slot priority WITH the user)

**R3. 3-SETTLEMENT DEMO SEED — ✅ SHIPPED 2026-06-24 (NOT yet committed).** demoWorld.json now seeds THREE walled colonies
~1 km apart with distinct genomes — A=(0,0) bold, B=(1000,100) cautious/herd, C=(450,950) industrious — + rabbits as livestock
+ a few wild kangaroo/lion between. Genome via the seed object's `genome:[5]` field (plumbing already existed). B/C dormant on
load, materialise + wall on arrival (added a fence wake-trigger). Script: scratchpad reseed3.mjs. 75 JS / 0 typecheck / verified.

**R1. REPRODUCTION LIFECYCLE — meet → bond → build → pregnant** (user: "a couple must meet, bond, build a house, THEN
get pregnant — makes no sense they'd get pregnant without a house"). Today bonding+conception fire TOGETHER (a pair breeds →
bonds + conceives) and the build needs the bond → so the house comes AFTER the first baby. Decouple: (a) a BONDING pass —
two ready, unbonded, opposite-sex, unrelated adults nearby pair up, NO pregnancy; (b) build needs the bond (already true);
(c) PEOPLE conception gated on a HOME within ~50 m (sim `nearest_refuge`). Animals stay "meet→breed" (no house). ⚠️ emergent-
balance risk — run full `cargo test` (pop growth / banding / boldness).

**R2. NOMADIC HUTS vs PERMANENT SETTLEMENTS — resolves the 1 km tension** (user: "next settlement can't be within 1 km…
a roaming couple builds a temporary small house, makes a baby, stays around, then moves on to properly build a settlement").
With R1 a couple needs a house to breed, but can't build within 1 km of a town (NEW_GAP). Fix: a roaming couple in the
75 m–1 km dead-zone builds a TEMPORARY HUT (exempt from NEW_GAP, small, fast decay), breeds, rears, then MOVES ON to found a
PROPER settlement (≥1 km) or join a near one. Huts decay when abandoned → don't violate the 1 km spacing for permanent towns.
build_ops: lone build ≥1 km → house (new colony); 75 m–1 km → hut (temporary); <75 m → join colony.

**R4. PREDATOR FEEDING + appetite/sleep levers** (user: "a cat killing a rabbit then instantly moving away — show it eating
a few seconds"). On a kill the hunter should HUNKER + eat for a few seconds before moving (partly there: `feeding=FEED_SECS`,
`full_after`→sleep — tune so it actually pins movement + reads as eating). Expose levers: appetite (hunger threshold to hunt),
feeding duration, sleep length.

**R5. EMERGENT-WORLD TUNING PASS (meta)** (user: "should we be tuning our emergent world thing better?"). Stop reacting to
single observations. Use R3's seed + the Rust scenario harness (cargo test scenario_) to tune the big levers DELIBERATELY —
breeding rate, dispersal/founding, predation/appetite, migration, genomic divergence — one at a time, measured over thousands
of ticks. [[sim-scenario-testing]] [[emergence-is-endgame]].

---

### EF11. PLANNED SETTLEMENTS — a matured house-cluster upgrades into a designed town (BIG, top of mind)
User: settlements must be built to a PLAN, not random. Past a housing threshold a cluster gains a well, a
perimeter fence, a watchtower, ROADS connecting things, etc. Build a set of ~40 "make-city" layouts, varied by
SIZE; when an emergent settlement gets big (full of houses) it SNAPS into a planned, well-built town.
- Design fork (asked): (a) a deterministic PLANNER `settlementPlan(center,size,seed)` that lays out radial/grid
  streets + houses-along-roads + central well/plaza + watchtower + perimeter fence (effectively unlimited varied
  planned towns, maintainable) — RECOMMENDED; or (b) ~40 hand-authored fixed templates (curated, more control,
  much more authoring). Either way needs NEW renderable prop kinds: well, watchtower, road segment (Prop/new
  components) + the upgrade trigger in Scene (detect a ≥N-house cluster, replace the ad-hoc houses with the plan,
  tag it `planned` so it doesn't re-fire) + keep player/LLM builds exempt. Visual → needs the user's eye to tune.
  - **2026-06-24 additions (user):** humans' INSTINCT is to clump into settlements; **max 10 houses per settlement**
    (COLONY_MAX now 10). A lone house in the middle of nowhere is ALLOWED and survives, but its inhabitants are at
    HIGHER predator risk (less refuge/guard support) → so they're incentivised to migrate to a budding settlement,
    OR to wait for other humans to arrive and grow the lone house INTO a settlement. So: planner draws fence + ROADS
    (paths) + central well + watchtower when a cluster matures; lone houses stay un-planned but riskier.
  - **Already shipped this session toward this:** "reward building" carrying-capacity (`world_area_scale` → builds/25,
    clamp 4 — cities now grow to ~150 people, Rust 153 tests green, NOT yet wasm-rebuilt); GRAVEYARDS (deaths near a
    settlement bury into a town-edge plot, wild deaths leave no grave — Scene `graveyardSpot`). Props that already
    render: house/cabin/tower(=watchtower)/well/fence/lamp + path/plaza colours. Clump-migration already in the sim.

### 🌊 2026-06-22 idea-flood (emergent-world session) — captured, awaiting priority
Done this session: **emergent brain is the world default + on par with Manual** (scenario-tested), **genome spread
widened** (0.3‥1.7), **player IMMUNITY** (Sim sets it on — no predator hunts/menaces you, danger stays 0). The rest,
to build one at a time:

- **EF1. World "wall" visibility (curved horizon).** The curved-up terrain (the gravity-fold) fades into sky/fog and
  is fully sky at the top; user wants the wall CLEARLY VISIBLE with clarity when looking up. Root cause (explored):
  `curveWorld.ts` (radius 800) lifts far ground overhead only at ~2.5 km, but `FogExp2` (kinds.ts `SKY_FOG`) + the
  grass dissolve-to-ground (Grass.svelte) + clouds at ALT=130m (Clouds.svelte) all blend the rise into sky before
  it's visible. Levers: lower fog density / switch to ranged fog, tighten curve radius (sooner rise), lower/soften
  clouds, keep terrain colour saturated at distance. [render — needs the user's eye to tune]
- **EF2. Too many live objects (~800 in-area).** Trim the live/near object count for perf — tighten streaming
  offload radius / soft targets / breeding plateau so the individually-simulated set stays bounded (memory says
  ~<400 target). Check whether 800 counts creatures only or +trees/houses. [sim/streaming/perf]
- **EF3. Genuine weird mutations.** Rare visible mutations — cancerous growths, "very weird" morphologies, and
  MISSING LIMBS (rarely, from surviving an animal attack). A heritable/again-random visual+stat deviation; injury-
  driven amputations persist on survivors. [sim genome + render — big]
- **EF4. Pair-bonding families.** Mates STICK TOGETHER while bonded / mating / gestating / raising a young one; when
  the young matures to adult the pair MAY break up (or not). Bond = a tracked partner index with a strong mutual
  tether, released probabilistically at the child's adulthood. [sim — composes with juvenile-follow + breeding]
- **EF5. Old-age made visible (all animals) + slower + more vulnerable.** Frailty already SLOWS elders (FRAIL_ONSET);
  add a VISUAL aging cue across every species (greying/thinning/stooped scale) AND make elders more VULNERABLE
  (easier to catch / lower health). [sim already half-there + render]
- **EF6. Setup/loading overlay.** An overlay "setting up…" window showing setup progress; it WAITS for object
  creation / the world to STABILISE, then fades away. [UI — gate on agent/object count settling]
- **EF7. Predators avoid settlements + RAID/DEFENCE dynamics (2026-06-24 user).** Predators steer AWAY from a
  settlement's fenced perimeter and, if they end up inside, try to get back OUT. On top of that avoidance:
  - **Lion:** LOW probability per approach to RAID — sneak in, kill exactly ONE, then flee back out (a "grab-and-go"
    directive, not a sustained hunt inside). Even if it breaches, it takes one life and runs.
  - **Dinosaur:** like the lion but HIGHER probability (it's bigger) — and it BREAKS the fence to enter (remove/“break”
    the nearest fence panel on breach). When a dino is inside a settlement, the WHOLE ADULT population converges and
    mobs it until it's dead (settlement-scale GUARD_RALLY). Dino doesn't specifically target the fence — same raid
    instinct as the lion, just a higher chance and it smashes through.
  - Build needs: the sim to know each settlement's perimeter (derive from refuge/house centres + the wall radius),
    an avoidance/repel field at the perimeter, a per-approach raid roll (low lion / higher dino), a one-kill-then-flee
    state, a "break a fence panel" hook back to Scene, and a mob-the-intruder rally for adults. Composes with the
    existing refuges channel + GUARD_RALLY. [sim — big; do AFTER the fence visual is confirmed]
- **EF13. Hunter-provider trips (2026-06-24 user).** A male with the HUNTER phenotype (high food/industry genome)
  whose mate is SAFE inside a fenced settlement may leave her to go out and hunt + provision. Gated by family state:
  LOW probability of leaving while she's PREGNANT (he stays close/guards); a bit HIGHER once the CHILD is BORN (family
  established + walled → he can range out to hunt). Composes with the hunt-and-provision loop (carry the kill back to
  the larder), pair-bonding (EF4), the fence (mate's safety), and the genome phenotype. [sim]
- **EF10. Prey must SPREAD far (anti-crowding near settlements).** A settlement formed near home and rabbits/rodents
  pool in that area, crowding it. The world's north-star is to SPREAD OUT as far as possible. Push prey dispersal
  harder — stronger outward drive / lower crowd threshold for prey near dense areas, so herds keep colonising new
  ground instead of pooling around a town. Composes with the existing flock dispersal (DISPERSE_CROWD/BLOB_CROWD)
  + the emergent brain (could add a "seek open range" drive when crowded). [sim — flock/emergent dispersal]
- **EF9. Combat needs a sense of FIGHTING (not instant kills).** User saw a lion prowl into a couple → both died
  instantly; wants a sense of struggle (not necessarily visual) — a brief fight: the prey resists / it takes a beat
  / health drains over a short grapple before death, instead of an on-contact one-shot. Reuse the slash/health
  mechanics for single predator↔prey, not just mobs. [sim — emergent + manual catch path]
- **EFbug1. Cats fidget weirdly ON rabbit CARCASSES.** Emergent is great + determined overall (user loves it), but a
  cat at a dead rabbit jitters/circles oddly. Likely the Scavenge approach (CHASE_W·0.7 toward the corpse) overshoots
  + the anti-overlap with the carcass body bounces it, or the idle FSM plays over the feed. Smooth the carcass feed
  (stop/settle on contact, no re-approach jitter). [sim — emergent Scavenge primitive / corpse anti-overlap]
- **EF8. Genome cross-birth inheritance (finish Tier 2).** Today founders vary but babies re-roll from their own
  seed → no selection DRIFT. Carry the 5 genome weights through the births buffer (like the vigor gene) so a
  population's average strategy drifts under selection, visible as a HUD number like ⚡vigor. [sim plumbing]

### A. Dynamic "Mother Nature" director (homeostatic parameter control)
When a population stagnates / drifts, Mother Nature should TWEAK PARAMETERS (not just spawn migrations): e.g. make
a species more reproductive or more aggressive, raise/lower caps, etc. A feedback controller that nudges the sim's
constants toward a healthy, churning balance — boost a sagging species, rein in a booming one. Pure math is fine
(no LLM needed); the LLM-as-director is an option later.
- Needs: per-kind runtime-adjustable levers in Rust (breed vitality, aggression, cap multiplier) + a JS controller
  that reads pop trends (telemetry/agentManager) and sets them.

### B. Habitation / house decay + inhabitation  ⚠️ MUST NOT decay the player's builds
- Houses DECAY over time; UNINHABITED ones die (reclaimed) → bounds sprawl (big-world "churning steady state").
- A house with humans BELONGING to it (residents) never decays.
- PLAYER-built houses: protected — never decay (or ≥10× slower). NPCs can't claim/inhabit them.
- Implementation note: tag player/LLM-placed buildings with a `keep` flag; only auto-built (emergent) empty homes decay.

### C. In-house protection + flee-to-safety — SHIPPED (flee-to-safety; hard in-house immunity = optional follow-up)
- Houses are fed to the Rust sim as REFUGE points (new `set_refuges` channel, full pipeline mirroring `set_fish`:
  world.rs field/setter/`nearest_refuge` → lib.rs Sim → worker `refuges` msg → rustSim `setRustRefuges` → Scene
  feeds building centres each obstacle-rebuild). A fleeing PERSON within REFUGE_R=40m blends a home-ward bias
  (REFUGE_PULL=0.8) into her escape vector — UNLESS home is behind the predator (then plain flight wins, never run
  INTO the hunter). "Protection" emerges: she runs to the houses where the guard men cluster (existing GUARD_RALLY).
- Test `a_threatened_woman_flees_toward_a_house` (refuge vs none → flight curves toward home).
- ⏭️ OPTIONAL follow-up: HARD immunity — a person within a few metres of a house can't be caught at all (suppress
  the catch). Not done yet; current behaviour is "run home + the guards defend you," which is more emergent.

### D. Neighbour mechanic
- When a human reaches a house that isn't its own → probability flip on whether the neighbour lets it stay (depends
  on "neighbour type").
- If conditions are right, it may REPRODUCE with the neighbour.

### E. Low-population human banding (survival instinct) — SHIPPED (truce + gather; long-range seek = follow-up)
- IN RUST (`world.rs`): a LATCHED `person_banding` flag (hysteresis PERSON_BAND_LOW=12 ↑ PERSON_BAND_RELEASE=20),
  computed at tick START from the live person count. While banding: (1) aggressive infighting is SUPPRESSED — an
  aggressive person won't target fellow people (truce gate in `target()`); (2) cohesion rises for EVERYONE incl.
  men (`coh_w += BAND_GATHER_W`) and dispersal is held off, so those near each other clump into a town nucleus.
- Tests: `low_population_humans_band_with_hysteresis`, `banding_humans_dont_hunt_their_own`.
- LONG-RANGE convergence — SHIPPED: a banding person with < BAND_SEEK_QUORUM(3) flock-neighbours steers toward the
  NEAREST other person on the wider seek grid (SEEK=100m) at BAND_SEEK_W(0.5) drive, so far-flung survivors walk
  over and regroup; local cohesion takes over once a quorum gathers. Test: `banding_survivors_seek_each_other…`.

### F. Equal-gender colonising bands — SHIPPED (in Rust)
- When a young person disperses out of a blob it pairs with its nearest opposite-sex young neighbour (tracked in the
  flock scan). LEADER/FOLLOWER: the lower-seed member leads (takes the outward compass heading + a gentle tether to
  its partner, BAND_PAIR_W=0.45 swept); the higher-seed FOLLOWS at full outward strength. So each band is a man +
  woman striking out together ("like missionaries"), gender-mixed by construction — no single-sex dead-end colonies.
- Test `dispersing_bands_stay_gender_mixed`: from a 16-person blob, the worst-off man stays within ~⅓ of the
  dispersal radius of a woman (the pairing roughly halves that gap vs. independent random headings).

### G. Men literally hunt prey (food role)  — HELD pending balance
- Adult males chase rabbits/prey for food. Gate to "only when hungry" so it doesn't over-pressure the prey base.
- Open question: the current "men range out + guard" may already give the hunter *feel* — confirm before adding predation.

### H. Bigger / more-frequent prey booms
- "More prey = more total life." Make Mother Nature's "season of plenty" rabbit/kangaroo booms larger or more frequent.

### J. Fence not movable (bug) — INCONCLUSIVE (needs a live repro)
- A fence renders via `Prop.svelte`, whose ROOT group already sets `userData={{ objectId: obj.id }}`, and the move
  tool's `pickObjectId` walks parents for that id — so on a code read it SHOULD be pickable. No obvious cause found.
- Likely culprits to check WITH the game open: the fence mesh is thin (raycast misses the slats — try tapping dead-
  centre on a post), or it's a multi-segment ring where each tap moves one segment. Left for the user to reproduce.

### lakes-validation-done. No static objects spawn in water — SHIPPED
- Settlers raising homes (Scene live + `world.ts fastForward` away-growth) and colony trees now skip any plot that
  `inWater()` reports — no more houses/trees floating in a lake. (Creatures still may cross water but get pushed out
  by the obstacle, and graves sit where a person died, which is dry since water is an obstacle — both left as-is.)
- Also fixed a STALE pre-existing test: the runaway-count clamp expected 120 but MAX_COUNT was raised to 1000.

### N. Per-user spawn regions (localStorage)
- A "user" doesn't spawn at (0,6) — each gets their OWN spawn area spread across the grid (decent distance apart:
  far enough to feel separate / see another's place at a distance, NOT so close it crashes). Reuse the existing
  region/area system.
- Store per-user in **localStorage** for now (no real auth yet): { spawnPoint, lastPosition, … } — "favour
  computing/persisting as much as possible." (Composes with N's multi-region + B's keep-flag.)

### litter-cluster-done. Litters born clustered around the mother — SHIPPED (in Rust, polish)
- Newborns already delivered at the mother's spot, but ALL littermates at her EXACT point → anti-overlap exploded
  them apart on tick 1. Now each baby gets a tiny seeded ring offset (0.4 + 0.25·sibling-index m) so a litter emerges
  as her brood clustered around her. Test `a_litter_is_born_clustered_around_the_mother`.

### juvenile-follow-done. Juveniles trail a parent — SHIPPED (in Rust, realism)
- A baby ANIMAL (non-person; people's children already cluster via coh_w) steers toward the nearest grown adult of
  its kind it can see in the flock — fawn/duckling family trains. FOLLOW_W=0.1 (just above herd cohesion), juvenile
  = age < 0.18×lifespan, parent = age ≥ 0.3×lifespan. Balance-neutral (keeps the young in the herd → a touch safer).
- Test `a_juvenile_animal_trails_a_parent` (a parked adult; the baby closes the gap).

### frailty-done. Age frailty — SHIPPED (in Rust, realism)
- In the last fifth of its life (past FRAIL_ONSET=0.8 of lifespan) an animal SLOWS — its gait cap ramps down to
  FRAIL_MIN=0.72 at death (multiplies `behave.0`, composing with the injury limp). So predators naturally cull the
  OLD & weak (a slow elder is the easy meal), and generations turn over instead of everyone dying at the cap. The
  player's pet is exempt (it never ages out). Test `an_aged_animal_slows_with_frailty` (same seed, old covers less).

### scavenging-done. Carrion / scavenging — SHIPPED (in Rust, realism)
- A death is no longer wasted food. Every corpse (old age / starvation / a kill's leftovers) carries CARRION_MEAT=26
  edible-seconds, rotting at 1×/s (fed into the seek grid so scavengers can find it). A HUNGRY carnivore with no
  live prey pads to the nearest fresh carcass (SCAVENGE_R=16m) and feeds (SCAVENGE_GAIN energy/s), draining the
  carcass faster so it feeds a few then is picked clean — no food-coma (scraps top up, a fresh kill still gorges).
- Behaviour rank: live prey > carcass > idle fish-lure/wander. Self-limiting (feeding clears the hunger latch).
- ⚠️ INVARIANT (regression fixed): corpses are inserted into the SEEK grid (so scavengers find them) but `target()`
  skips dead agents at the top of its neighbour loop — else a fresh same-rank predator carcass was picked as a
  territorial RIVAL and the living one charged + bled on it. Test `a_predator_does_not_fight_a_corpse_as_a_rival`.
- Tests: `a_hungry_carnivore_scavenges_a_fresh_carcass`, `a_carcass_rots_away_even_uneaten` (+ `fast_forward_relaxes…`
  now covers ff_targets natively). 63 cargo tests.

### A-done. Dynamic Mother Nature director — SHIPPED (in Rust, homeostatic)
- The sim now drifts each kind's breeding vitality toward a target from pop/carrying-capacity each tick (struggling
  → breed hard, booming → ease off). Fully in-sim, no JS controller. Vitality eases breed_ready + the cooldown.

### caps-done. Carrying-cap math → Rust — SHIPPED
- `cap_for` (the trophic carrying-capacity formula) is the single source of truth in `world.rs`; exported as the
  `pop_caps` wasm fn. JS `world.ts popCaps` now calls it via `rustMath.ts` (a SECOND, stateless main-thread wasm
  instance — the worker's sim wasm is unreachable synchronously). `+page.svelte` awaits `initRustMath()` before any
  cap/scatter math so caps are the real Rust numbers, never the permissive fallback. No duplicated balance formula.

### I-done. Fast-forward logistic math → Rust — SHIPPED
- The closed-form logistic relaxation (rates + floors + prey-before-predators ordering) is now `ff_targets` in
  `world.rs`, exported as the `ff_targets` wasm fn and called from `world.ts fastForward` via `rustMath.ts`. JS only
  materialises the deltas (random object scatter — state glue, stays JS). No duplicated FF balance math.

### L. Move the engine's math to Rust (non-UI parts)
- `src/lib/engine.ts` (applyOps placement, `findFreeSpot` O(n²) collision search, scatter spiral, anchor maths) is
  heavy compute living in JS. Per the architecture law it belongs in Rust; the op-orchestration / world-object
  authoring stays JS. Big refactor — the engine is tightly coupled to `world.objects` + the LLM ops. Worth it.
- ⚠️ BLOCKER for the cheap shim route: `findFreeSpot`'s free-test calls `inWater` (`water.ts`), whose blob
  shoreline (`waterEdgeFactor`, sum-of-sines) is DELIBERATELY mirrored to the GLSL in Water.svelte AND reused by
  the Player wade/city. A stateless main-thread Rust port would force a THIRD copy of that edge math — the exact
  mirror-sync hazard we avoid (cf. terrain heightAt↔Grass). So this is NOT a `rustMath` one-off like caps/FF.
- Correct path: do placement INSIDE the sim (the unified entity buffer, big-world.md §6.9) where obstacles +
  water zones are already resident — Rust owns the geometry once, no per-call re-shipping, no duplicated water
  math. Defer until the buffer lands. (findFreeSpot is build-time, not a per-frame hot path, so no perf urgency.)

### M. Regional / session creature caps (big-world)
- Player creations: dinosaurs ≤10 per player per session, ≤20 per region; other creatures spread across regions.
- Distinguish PLAYER-created creatures (intentional, should persist up to their cap) from the ecosystem's
  proportional auto-breeding caps (`capCreatures` currently trims everything to the proportional cap on load).
- Needs the region model (big-world.md) + a "keep/player-made" tag (shared with B's house-decay flag).

### K. Predator prey-choice: prefer the BIGGEST, fall back to the nearest
- A predator should target the biggest prey available (most food), but if that's far, take a smaller one that's
  close — a size-vs-proximity balance.
- NOTE: largely already in `target()` via the score `prize(kind) / dist²` (big = higher prize, near = lower dist²).
  This is a TUNING item — adjust the prize/distance weighting so "biggest unless something's much closer" reads right.

---

## ✅ Shipped (recent arc — for context)
- Human-breeding crash fix (crowd gate sterilised city-dwellers; people exempted) → pop recovered 10 → 100+.
- Village GUARDS — men charge predators threatening the community; women + children flee.
- Family grouping — women+children cohere (home), men range out (role-based cohesion).
- Person dispersal — big human BLOBS split into bands that strike out + found settlements (threshold 9).
- House size variety (auto-build cabins / houses / manors) + LLM size words ("big house", "small cabin").
- HMR resume fix — a code change no longer teleports the player home (resumes live position).
- Move-tool placement — invert the world-fold so structures land under the cursor, not "far back".
- Grave visibility (pale moonlit stone), white-hills threshold raised, house roof alignment (45° pyramid fix).
- `goto(x,z)` dev teleport. Deterministic time fast-forward (population + town growth + graves while away).
- Earlier: reproduction, immigration (founding groups / gene flow / genetic rescue), trophic caps (prey∝area,
  predators∝prey), carnivore metabolism, Mother Nature wildcards (+ dino reintroduction), building impostors,
  colony trees, graves, telemetry, DB-persisted live positions.
