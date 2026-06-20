# Robust build pipeline — handle the long tail without hardcoding phrasings

The model is an AI; we don't enumerate phrasings. Instead we give it a **complete, composable symbolic
grammar** and a **forgiving, self-correcting engine**, so any reasonable request maps to valid ops and
any bad output degrades gracefully. Five layers:

## 1. Ground (context)
`buildSystem(world, player)` shows the model the live world: the player's spot ("here"), and the
nearest objects with exact ids + kinds + positions. So "this house" / "the tower" resolve to real ids.
(Roadmap: also list nearby **zones** — ponds/lakes/plazas — so "the pond" resolves.)

## 2. Express intent symbolically — a COMPLETE relation grammar (engine resolves coords, never the LLM)
The model never emits coordinates. It emits a kind + a symbolic anchor. The anchor vocabulary is
complete + composable so the model never has to improvise malformed anchors:
- egocentric: `here`, `front`, `behind`, `left`, `right` (+ `dist`)
- **object-relative**: `front:<ref>`, `behind:<ref>`, `left:<ref>`, `right:<ref>` — in that direction
  from an OBJECT ("in front of the house"). NEW — was the #1 gap (model improvised `front:near:tower`).
- `near:<ref>` / `beside:<ref>` / `nextto:<ref>` — beside a thing
- `on:<ref>` — on top of it
- `between:<a>,<b>` — midpoint
- **`around:<ref>`** — a RING around it (for "fences around the house", "rocks around the pond"). NEW.
- areas: north/south/east/west/center/everywhere
`<ref>` resolves fuzzily: exact id → "o"+id → nearest of that KIND → "last"/"it"/"that" (newest).

## 3. Validate & repair (deterministic, in the engine)
The engine is forgiving so garbage never crashes or mangles the world:
- ids never collide (numbered past the max existing, not array length) — fixes the each_key_duplicate crash.
- `paint`/`move`/`remove` resolve targets fuzzily and **no-op on an unknown ref** (a hallucinated id
  like `o6n` does nothing — never nukes a random object).
- counts + dist are clamped (no 1e15 placements, no 9999-object lockups).
- malformed anchors fall back safely (to the player), and `front:near:tower` is tolerated → `front:tower`.
(Roadmap: an optional ONE-SHOT LLM repair pass when the batch is empty/all-invalid — feed the concrete
problem back: "you referenced o6n which doesn't exist; the ids are o1,o2; fix it".)

## 4. Apply (deterministic) + clear feedback
`applyOps` mutates world-state. When the request is out of scope or too big, the model emits a `note`
op (shown as a 💡 banner); if nothing buildable comes back, a client safety-net states the limits. So a
non-technical user always sees *why* nothing happened (monkey → "I only have cats + people").

## 5. Decompose big asks (roadmap)
A 10-clause request ("huge cabin + 2 towers + circular pond + bridge + lamps + rocks + flowers +
fences") is past the small model's ceiling → garbage. Plan-then-execute: the model first lists atomic
sub-steps, each generated + applied in sequence with the world updated between, so every step is a
simple, grounded request. (Earlier regex-splitting was too brittle; this is the AI-native version.)

## 5b. Cap input + encourage focus (shipped)
Rather than decompose now, the build input is capped at 100 chars and the placeholder nudges "build one
thing at a time" — steering users to short, focused prompts the small model nails reliably. The big
19-op garbage prompt simply can't be typed. Decomposition (§5) stays a future option if needed.

## Principle
Keep the model's job EASY (small, grounded, composable) and the engine FORGIVING (resolve fuzzily,
clamp, no-op on nonsense, explain limits). New capabilities = extend the grammar + resolver, not
per-phrase special cases.
