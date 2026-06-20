# Sim changes log — radical changes + the forks behind them

> Per the user (2026-06-21, all-night autonomous session): **be radical, take risks** — "we're nothing
> without taking risks." Before anything drastic, **commit a checkpoint** and record here **what I changed vs
> what the status quo was**, to present on request. Also: **hysteresis in everything** (latch state with two
> thresholds → no per-tick flip-flop). Goal: the living-world **metabolism + behavioral realness** (behaviors,
> not textures) + the **clock-drive determinism migration** the user explicitly demanded.
>
> Checkpoints: `dd9c14d` = baseline (before any sim work).

Each entry: **what changed → the status quo it replaced → the risk → how to revert.**

---

### C1 — Clock-driven fixed-timestep sim + render interpolation  ✅ DONE (high-risk, user-demanded)
**What changed:** the ambient-agent sim (`agentManager`) no longer steps by the raw render `dt`. It's now
driven by the `SimClock` at a **fixed 30 Hz** tick:
- `AgentSystem.svelte`: `const n = clock.advance(dt); for i<n → agentManager.tick(DT)` (was `tick(dt)` once
  per frame). Frame stalls emit a few capped catch-up ticks.
- `clock.ts`: added a cheap `get alpha()` (sub-tick fraction `#acc/DT`, clamped 0..1) for interpolation.
- `steering.ts` `Agent`: added `prevX/prevZ/prevHeading` + `savePrev()` + `interpolate(alpha)` → fills
  `rx/rz/rh` (shortest-arc heading). No allocation.
- `agents.svelte.ts`: `a.savePrev()` at the top of the per-agent loop (snapshot pose before moving).
- `Critter.svelte`, `Npc.svelte`, `AgentImpostors.svelte`: render at the **interpolated** `rx/rz/rh`
  (`agent.interpolate(clock.alpha)`) instead of the raw `agent.x/z/heading`.

**Status quo it replaced:** the sim ran once per render frame at variable dt (≈60 Hz on a fast machine,
clamped to ≤50 ms). Behaviour was frame-rate-dependent (faster sim on a 144 Hz monitor) and there was no
clock — no basis for pause / time-lapse / seek / shared determinism.

**Why it's safe-ish despite the risk:** DT (33 ms) sits inside the old clamped dt range (16–50 ms), so the
tuned steering/flocking constants see no new regime. The 30 Hz update (half the old 60) would stutter — the
prev→current interpolation by `clock.alpha` hides that, and is continuous across tick boundaries (alpha=1 of
one interval == alpha=0 of the next at the shared endpoint), so there's no hitch regardless of useTask order.
`svelte-check` 0/0; all 7 clock unit tests pass. AgentSystem is the sole `clock.advance` caller (no
double-advance).

**Risk / what to watch:** motion is now ~0–1 tick (≤33 ms) behind real time (interpolation latency —
imperceptible for creatures). If the clock is ever `pause()`d the sim freezes (intended, for time-travel).
**Not yet done:** per-tick RNG (`steering.ts` still uses `Math.random` for wander/explorer/behaviour, and the
`Agent` constructor for birth rolls) — so the sim is fixed-DT + frame-independent but **not yet reproducible**.
That's the next migration step (key wander/fights by `rng(tick, seedId, ch)`; thread `seedId` into `Agent`).

**Revert:** `AgentSystem.svelte` back to `useTask((dt) => agentManager.tick(dt))`; the renderers back to
`agent.x/z/heading`. The clock/steering additions are inert if unused. Or hard-revert to `dd9c14d`.

---

## Pending decisions for the metabolism (next, not yet built)

### F2 — Energy as a NEW field vs overloading `stamina`
My earlier carnivore work put metabolism/hunger/eat-refuel on `stamina`. Plan: split — `energy` (0..1) = the
metabolic life resource (drain → graze/eat → starve at 0, drives HUNGER); `stamina` reverts to sprint/activity
(drains sprinting, recovers resting; gates `canSprint` + the chase-distance give-up). Hunger MUST be
energy-based or a rested-but-starving predator never hunts. Hysteresis on the hunger latch (hunt < LO, sated >
HI).

### F3 — Food field (depletion grid) — build it, for real carrying capacity
Radical version: a world-stable depleting `grassEnergy[cell]` grid → grazers deplete patches + move to fresh
grass → Lotka–Volterra population cycles once reproduction (Phase 2) lands. (Earlier I'd have deferred this to
save code; per "be radical," build the real ecosystem.)
