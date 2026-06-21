# Ideas Queue — the living-world wishlist

A safe, persistent home for every idea the user has raised, so none are lost and they get worked through in order.
Newest ideas added at the top of "Queued". Move items to "Shipped" with the commit when done.

> Working agreement: build them ONE AT A TIME, well + tested (55 cargo tests, `svelte-check` 0/0), commit each.
> Several of these are HUMAN-SOCIETY facets that interact — design them to compose, not contradict.

---

## 🟢 Queued (priority order — top = next)

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

### C. In-house protection + flee-to-safety
- A human IN / right beside a house is SAFE — predators can't harm it there.
- A threatened human flees toward the nearest house (its own, ideally) and is safe if it makes it.
- Needs: feed house positions to Rust as "refuges" (precedent: `set_fish` lure points), steer + suppress catch near one.

### D. Neighbour mechanic
- When a human reaches a house that isn't its own → probability flip on whether the neighbour lets it stay (depends
  on "neighbour type").
- If conditions are right, it may REPRODUCE with the neighbour.

### E. Low-population human banding (survival instinct)
- When human pop drops too low: they STOP killing their own kind (aggression off).
- Scattered survivors actively CONVERGE on any human they can see → regroup in one place → found a town there
  (abandoning their old houses if needed).

### F. Equal-gender colonising bands
- When a clump splits/disperses, the bands should be roughly gender-balanced (a man + woman strike out together,
  "like missionaries") so each new band can actually found + grow a settlement.

### G. Men literally hunt prey (food role)  — HELD pending balance
- Adult males chase rabbits/prey for food. Gate to "only when hungry" so it doesn't over-pressure the prey base.
- Open question: the current "men range out + guard" may already give the hunter *feel* — confirm before adding predation.

### H. Bigger / more-frequent prey booms
- "More prey = more total life." Make Mother Nature's "season of plenty" rabbit/kangaroo booms larger or more frequent.

### I. Move the fast-forward math to Rust
- The closed-form logistic FF currently runs main-thread in JS (`world.ts fastForward`). The Rust sim is in a Web
  Worker, unreachable synchronously at load. Proper fix: make the FF a Rust sim op that emits its deltas through the
  existing birth/death channel; JS just materialises. (Architecture law: heavy math belongs in Rust.)

### J. Fence not movable (bug)
- A built fence can't be picked up by the move tool. Investigate the raycast / `userData.objectId` path for fences.

### L. Move the engine's math to Rust (non-UI parts)
- `src/lib/engine.ts` (applyOps placement, `findFreeSpot` O(n²) collision search, scatter spiral, anchor maths) is
  heavy compute living in JS. Per the architecture law it belongs in Rust; the op-orchestration / world-object
  authoring stays JS. Big refactor — the engine is tightly coupled to `world.objects` + the LLM ops. Worth it.

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
