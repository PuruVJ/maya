# Natural NPC & cat movement — research verdict + plan

Deep-research output (107 agents, 3-vote adversarial verification). Goal: move the cat + people
from "lerp-to-random + sine hop" (robotic) to believable life, staying **fully procedural** on our
un-rigged primitive-group models (no skeleton, no clips), cheap enough to run beside WebGPU LLM +
physics.

## The architecture (Craig Reynolds, canonical)

Three layers, top → bottom:

1. **Action selection** — a small behavior state machine / utility AI picks *what to do* (we already
   have idle/walking/jumping/falling; extend with sit/groom/stretch/look-around for the cat).
2. **Steering** — vehicle-model *forces* produce life-like, improvisational *paths*.
3. **Locomotion** — procedural transforms on the primitive group produce the *look* of moving.

## Verified techniques (all passed 3-0)

- **Steering force, not lerp:** `steering = desired_velocity − current_velocity`, applied as
  acceleration each frame; `velocity = truncate(velocity + accel, maxSpeed)`. Compensating for the
  agent's own velocity is what reads as alive. (red3d.com/cwr/steer, natureofcode.com)
- **Wander (the correct method):** keep a target on a circle a fixed distance *ahead* of the agent;
  each frame nudge it a *small random amount* around the circle (`wanderAngle += smallRandom`) — do
  NOT teleport to fresh random points. Smooth wandering with long-term order. (Wander.html, gdx-ai)
- **Arrive:** define a deceleration radius > stop radius; outside it move at max speed, inside it
  scale desired speed from max → 0 at the target. Natural ease-in instead of snapping to a stop.
- **Spring-damper secondary motion:** a closed-form **damped-spring** (frame-rate independent) is a
  drop-in replacement for lerp/tween. Drives tail follow-through, body bob/sway, banking/lean into
  turns — "a follow spring whose equilibrium is updated each frame." Cheap, the biggest life-per-line.
- **Per-leg gait via sine oscillators (CPG-style):** drive each leg's swing/stance from its own
  phase-offset sine, keyed to gait phase — NOT one global sine "hop" (that's the cat's current bug).
- **Naturalness lens (12 principles, applied procedurally):** anticipation vs responsiveness,
  slow-in/slow-out with end-weighted follow-through, decoupled procedural lean, idle variety,
  micro-movements, pauses/hesitation, noise-driven variation, "personality" via parameters.

## Rigged alternative (verdict: skip for now)

Rigged GLB + AnimationMixer + Mixamo/Quaternius clips + foot-IK/look-at gives higher fidelity, but
the GPU bone-texture crowd trick only applies once rigged. For our un-rigged primitive groups, **stay
fully procedural** — adopt rigs only later if fidelity demands it.

## Phased rollout (cheapest-highest-impact first)

1. **Steering core** — shared agent update loop; replace lerp-to-random with wander + arrive +
   seek/flee. Heading follows velocity. (This alone kills most of the "robotic" feel.)
2. **Spring secondary motion** — damped-spring lean-into-turns + body bob + tail follow-through.
3. **Per-leg gait + idle behavior variety** — CPG sine legs; cat sit/groom/stretch/look-around via
   the state machine; utility AI to pick actions.
4. **(Optional) rigged upgrade** — only if fidelity demands.

Performance: one shared update loop over all agents; primitive-group transforms are nearly free;
springs are O(1)/agent. Many agents fine. Determinism not required for ambient critters.
