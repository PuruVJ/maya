# Emergent Behavior — a switchable AI mode (design spec)

> Status: **BUILT — Tier 1 + Tier 2 substrate, now the GAME DEFAULT** (2026-06-22). The needs+primitives+utility
> brain ships in `crates/worldsim/src/emergent/` behind the `BehaviorMode` switch. It is the default for the
> live game (`Sim::new` → Emergent); `World::new` still defaults to Manual so the 110+ manual unit tests keep
> pinning the hand-coded brain. Emergent is validated **on par with Manual** by mirrored scenario tests
> (`scenario_emergent_*`): predation (44 vs 34 kills/100 s), dispersal (148 vs 117 m), population (×1.7 sustain),
> trophic pyramid (apex stays rare), and a settlement that builds homes. Flip live via the 🧠/⚙️ HUD toggle.
> **Still open:** true cross-birth genome inheritance (founders vary today; babies re-roll from their own seed —
> needs the births buffer to carry the 5 weights) and **Tier 3** (the LLM director authoring primitives/weights).
>
> Original design intent below — an **opt-in MODE that coexists with the hand-coded behavior**, never a rewrite,
> never a deletion. If it underperforms, flip back to Manual and keep shipping.

## 0. The ask (verbatim intent)
Today every behavior is **hand-coded** in Rust: social structures, house-building, mob defense, fleeing, breeding.
To add "dig ponds", "herd sheep", "have jobs", we'd code each one by hand. The user wants the option of a system
that **discovers/composes behaviors itself** and lets the world **evolve on its own** — *and* the ability to
**switch modes**: if the emergent approach isn't working, switch back to **Manual** and keep it. **No deletion of
old code. All new code lives in a new place.**

## 1. Why a MODE, not a replacement
Open-ended emergence is unsolved frontier research — high risk. The current hand-coded sim is proven, tested
(112 cargo tests), and *good*. So we treat emergent behavior as a **parallel decision layer** behind a switch:

- **`BehaviorMode::Manual`** — today's hand-coded behavior in `world.rs`. **Untouched. The default. Always shippable.**
- **`BehaviorMode::Emergent`** — the new needs + primitives + utility system in a **new module**.
- (room for **`Hybrid`** later — per-species mode, e.g. predators Manual, people Emergent.)

The switch is a runtime setting (wasm setter + a HUD/dev toggle), defaulting to Manual. Both code paths compile and
run; you can A/B them live in the same world.

## 2. The clean seam — where the switch lives
The sim pipeline each tick is roughly: **decide → steer → collide → step → read-back**. Only the **decide** step
differs between modes; *everything downstream is shared*. So the seam is exactly one branch:

```
// world.rs step(), conceptually:
match self.behavior_mode {
    BehaviorMode::Manual   => self.decide_manual(),        // the EXISTING behaviour pass, renamed, code unchanged
    BehaviorMode::Emergent => emergent::decide(self),      // NEW module: crates/worldsim/src/emergent/
}
// ...then the SAME steering/collision/step/read-back as today.
```

Both `decide_*` produce the **same output contract**: each agent's desired intent (target point / action + a
desired-velocity or force). Downstream physics, mobbing-combat resolution, collision, and the SoA read-back to JS
stay identical → the render layer and tests don't care which brain ran.

**No deletion rule, concretely:** the current behaviour code is *moved as-is* into `decide_manual()` (a rename/extract,
byte-equivalent logic, re-run the 112 tests to prove parity). The emergent brain is **all-new files** under
`crates/worldsim/src/emergent/`. Manual remains the default and the fallback forever.

## 3. Emergent mode — the three tiers
### Tier 1 — Needs + Primitives + Utility (the substrate; build first)
Stop coding *behaviors*; code **needs**, **primitives**, and a **scorer**:
- **Needs** (per agent, 0..1, drift over time): `hunger`, `thirst`, `safety`, `comfort`, `social`, `rest`, `purpose`.
- **Primitives** (the verb library — small, composable, each already half-exists in the manual code): `move_to`,
  `gather`, `carry`, `place`, `dig`, `drink`, `eat`, `follow`, `flee`, `join`, `rest`, `attack`.
- **Utility scorer**: each tick (or every N ticks, for cost) score candidate `(primitive, target)` options by how
  much they'd reduce the agent's most-pressing needs, weighted by feasibility/distance; pick the best; commit with
  hysteresis (no per-tick flip-flop — reuse the manual sim's hysteresis lessons).

Complex behavior **emerges**: "dig a pond" = thirsty + idle + near a low wet cell → `dig` scores high. "Herd sheep"
= a `purpose:shepherd` need + `follow`/`contain` primitives. "Jobs" = stable high-utility primitive loops for a need.
Cheap, **deterministic** (scoring is a pure function of state + seeded tie-breaks), pure Rust, no ML.

### Tier 2 — Evolve the behavior (no training, just selection)
Extend the existing **vigor gene** to a small **behavior genome**: the agent's *utility weights* (how much it values
safety vs food vs social vs building) + a few primitive biases. Inherited (avg ± mutation), selected by survival/
breeding — exactly like vigor today. Now lineages *discover* strategies (cautious, industrious, nomadic) with **zero
authored behaviors**. Deterministic (seeded mutation). This is "keeps learning" via **evolution, not gradient descent**.

### Tier 3 — LLM as a slow behavior-AUTHOR (the "invents new behaviors" part)
The LLM does **not** drive agents per-tick. Every few minutes (the [self-sustaining-world] "Mother Nature director"
seam) it reads a world summary and **emits new behavior rules / need-weight tweaks** as **grammar-constrained ops**
(the project's existing "LLM emits ops, deterministic engine runs them" pattern). E.g. it authors "agents with
`purpose` + idle time near water → form a `dig_pond` job". The engine runs it deterministically; the seed still
replays. Touchstone: *Voyager* (an LLM writing its own Minecraft skill library). This is the only stochastic seam,
already gated by the no-true-randomness north star.

## 4. Determinism (non-negotiable — see project-identity north star)
Every tier stays a pure function of `(seed, tick)`: utility scoring is deterministic; tie-breaks use the seeded
hash-RNG; evolution uses seeded mutation; the LLM call is the *single* gated stochastic event (its output is
committed as data, so replays are deterministic from the committed ops). Time-travel / fast-forward / shareable
seeds all still hold.

## 5. What we deliberately DON'T do
Per-agent neural nets / reinforcement learning ticking in the browser: needs a GPU, breaks determinism (no replay/
seed/time-travel), is a black box (not inspectable, not shareable as text). It fights every project pillar. Excluded.

## 6. Staging
1. **Extract** the current behaviour pass into `decide_manual()` (parity-tested, no logic change) + add
   `BehaviorMode` enum + `set_behavior_mode()` wasm setter + a dev HUD toggle. *Manual stays default.*
2. **Tier 1** in `crates/worldsim/src/emergent/`: needs, a starter primitive set (move/eat/drink/flee/follow), the
   utility scorer. Get a recognizable village living on it. A/B against Manual in the same world.
3. **Tier 2**: behavior genome on top of the vigor gene; watch strategies evolve.
4. **Tier 3**: extend the Mother Nature director to author primitives/weights as ops.
5. Each tier is independently shippable; Manual is always the safety net.

## 7. File layout (the "new place")
```
crates/worldsim/src/
  world.rs            # unchanged manual brain → decide_manual() (extracted, parity-tested)
  emergent/
    mod.rs            # decide(world): the Emergent decision pass (same output contract as decide_manual)
    needs.rs          # need definitions + per-tick drift
    primitives.rs     # the verb library + per-primitive feasibility/effect
    utility.rs        # the scorer + hysteresis
    genome.rs         # (Tier 2) behavior-weight genome
```
Mode lives on `World` (`behavior_mode`), set from JS via a new wasm export; persisted in the local world blob so a
chosen mode survives reload.

See [self-sustaining-world.md](./self-sustaining-world.md) (the director seam this reuses) and
[ideas-queue.md](./ideas-queue.md).
