# Relative / Relational Spatial Placement — research + plan

*Deep-research synthesis (25 sources, 113 claims, 22 confirmed via 3-vote adversarial verification). Question: can we support "between the two houses", "to my left", "on top of X", "in a circle", etc. — and what would it take?*

## TL;DR

**Yes, and our architecture is exactly the proven design.** Every major NL→3D system uses our split: **the LLM emits symbolic spatial relations/anchors (never coordinates); a deterministic engine resolves them into collision-free geometry.** So "relative spacing" = (1) extend the anchor grammar with a small fixed set of relation primitives, and (2) teach the deterministic engine to resolve each one. The LLM's job stays easy (name the relation); the engine keeps doing all coordinate math, collision, distribution sampling, and frame transforms.

**The one hard part is reference frames (FoR).** "My left" (egocentric) vs "the house's left" (intrinsic) is inherently ambiguous, and benchmarks show even 7–70B models are inconsistent at it. A ~1.5B model **cannot** be trusted to disambiguate frames — so the engine must commit to an explicit default (player yaw) and the grammar should let the model *tag* a frame at most, never *reason* about one.

## Survey — how the field does it

| System | Relation representation | Resolver | Takeaway for us |
|---|---|---|---|
| **Holodeck** (arXiv 2312.09067) | LLM emits constraints e.g. `coffee table, in front of, sofa`; **10 primitives in 5 families**: Global(edge/middle), Distance(near/far), Position(in-front/side/above/on-top), Alignment(center), Rotation(face-to) | DFS over (x,y,w,d,rot) + optional MILP | **The canonical compact grammar.** Direct-coordinate gen scored far worse (0.364 vs 0.706 MRR) → keep coords out of the LLM. No egocentric frame — we add that. |
| **SpatialGrammar** (arXiv 2604.27555) | LLM emits discrete BEV grid index + yaw; **ON sub-layout**: child attaches to a parent FACE (`_on_top`, `_on_left`…) | deterministic compiler → 6-DOF poses, verifiable collision/support | **"On top of X" = face-anchoring** — the LLM never predicts heights; the engine knows X's faces. |
| **SceneCraft** (arXiv 2403.01248) | relational scene-graph "blueprint" (bipartite G=(A,R,E)) → numeric constraints | stochastic constraint optimization | Separate **soft relational** constraints from **hard physical** (collision) ones. |
| **R3L** (arXiv 2605.06758) | relations factored into **intra-unit** (within an anchor+members cluster, local frame) vs **inter-unit** (between cluster anchors) | differentiable optimization | **Resolve clusters in a local frame, then place the cluster** — limits error-prone frame transforms. Precedent for "build a village around X". |
| **LayoutGPT** (arXiv 2305.15393) | CSS-style structured layout via **in-context demos** (training-free) | (emits boxes directly) | Endorses our **grammar-JSON + few-shot** mechanism. (Note: it *does* emit numbers — don't cite it for "no coords".) |
| **3D Scene-Gen survey** (arXiv 2505.05474) | scene graphs (object nodes, relation edges); design rules = co-occurrence, **symmetry, alignment, co-circularity** | various | Co-circularity/alignment back our **region/distribution** primitives (circle, lining edges). |
| **Game editors** (Unreal/Unity/Blender/Max) | vertex/face/grid **snapping**, parent-relative anchors | deterministic | Industry norm: relative placement is *snapping to references*, exactly engine-side. |

**Reference frames (the catch)** — FoREST (arXiv 2502.17775) & COMFORT (arXiv 2410.17385): LLMs/VLMs show "large, inconsistent FoR gaps" with a strong **egocentric default bias** (GPT-4o 97.9% relative vs 72.2% intrinsic; smaller models worse, near-chance on object-centered frames). Linguistics (Levinson: intrinsic/relative/absolute) confirms directional terms are frame-dependent. **Refuted (0-3):** "spatial-guided prompting reliably helps weak models" — don't expect prompt tricks to fix FoR on a small model.

## Plan for our stack

Keep the contract: **LLM names a relation + which object(s); engine computes everything.** Extend anchors from bare strings to a tiny vocabulary + a few optional numeric fields. Stays URL-compact (short enums + a couple numbers; distributions are ONE parametric op, not N expanded anchors).

**New anchor / relation primitives, by family** (all resolved in `engine.ts`):

1. **Egocentric** (rotate offset by player `yaw`): `left`, `right`, `behind` (we have `here`, `front`). Trivial extension of `front`.
2. **Inter-object**: `between:<a>,<b>` (midpoint) · `on:<id>` (parent top face, SpatialGrammar ON pattern — engine knows height) · `facing:<id>` (sets rotation, not position) · `near:<id>` (have, fuzzy). `against the wall` ≈ face-anchor on a wall/fence.
3. **Distance & scale**: an optional `dist` modifier on any directional anchor (`near`≈4, `far`≈30, or a number = metres). Engine scales the offset vector.
4. **Region & distribution**: extend `scatter` / add `arrange` with `pattern: row|circle|grid|ring|scatter|along:<pathId>` + `spacing`/`radius`/`count`. Engine does the math (parametric for row/circle/grid; Poisson-disk for natural scatter; sample points for along-path).

**Reference-frame rule:** default **everything to egocentric (player yaw)** — the strong English default. Pick ONE convention for "left of X" (treat as player-relative for now). Optionally allow a `frame:` tag later; never require the model to reason about frames.

**Resolver:** keep the deterministic spiral-search (fast, URL-reproducible). It's greedy/local — fine for single anchors; it can't jointly satisfy "between X and Y **and** against the wall." For now the grammar simply won't conjoin anchors; a bounded local DFS fallback is a Phase-3 option only if needed.

**Ambiguity ("which X"):** our `id→kind→nearest` fuzzy resolver is already a sound policy (no surveyed system does embodied interactive disambiguation). A recency/gaze tiebreak is a later nicety.

### Phased rollout (cheapest → highest impact)

- **Phase 1 — egocentric + simple inter-object.** `left`/`right`/`behind` (yaw rotation), `between:a,b`, `on:<id>` (face-anchor), `dist` modifier. Pure engine math, ~tens of lines. **Biggest bang for the buck.**
- **Phase 2 — distribution/region.** `pattern: row|circle|grid|ring|along` parametric ops; Poisson-disk scatter. Compact, high "wow".
- **Phase 3 — frames & joint constraints.** Explicit `frame:` tag, bounded multi-constraint resolver, smarter disambiguation. Only if Phase 1–2 demand it.

### Trade-offs & failure modes
- **Greedy resolver** can't satisfy conjoined constraints → forbid conjunction (or Phase-3 DFS).
- **FoR ambiguity** → committing to egocentric will sometimes mis-handle "the house's left"; acceptable for a sandbox, re-roll/undo cover it.
- **Distribution compactness** → parametric ops keep URLs tiny; the cost is the engine owns the layout math (fine, it already does).

### ⚠️ Critical gate (scale gap)
**No benchmark tested a 1.5B model** — all FoR/spatial results are ≥7B. Conclusions are directionally safe (gaps widen as models shrink) but unverified at our size. **Every new primitive must pass our crux test** (does Qwen2.5-1.5B reliably emit the right primitive for the NL?) before shipping. If it can't reliably produce `left`/`between`/`on`, simplify the vocab or bump those to 3B.

### Open questions (need our own measurement)
1. Can 1.5B reliably emit a directional primitive (+ optional frame tag), or must the engine hard-default to egocentric and ignore intrinsic requests?
2. Most URL-compact encoding for distributions (parametric vs per-instance)?
3. Does a recency/salience tiebreak beat `id→kind→nearest` for "which X"?
4. When does greedy spiral-search fail on joint constraints, and is a DFS fallback worth the complexity for a moddable sandbox?

### Sources
Holodeck [2312.09067](https://arxiv.org/abs/2312.09067) · SpatialGrammar [2604.27555](https://arxiv.org/html/2604.27555v1) · SceneCraft [2403.01248](https://arxiv.org/abs/2403.01248) · R3L [2605.06758](https://arxiv.org/html/2605.06758v2) · LayoutGPT [2305.15393](https://arxiv.org/abs/2305.15393) · 3D Scene-Gen survey [2505.05474](https://arxiv.org/html/2505.05474v1) · FoREST [2502.17775](https://arxiv.org/abs/2502.17775) · COMFORT [2410.17385](https://arxiv.org/pdf/2410.17385) · Frame of reference (linguistics) [wikipedia](https://en.wikipedia.org/wiki/Linguistic_frame_of_reference) · Poisson-disk [sighack](https://sighack.com/post/poisson-disk-sampling-bridsons-algorithm) · editor snapping: Unreal/Unity/Blender/3ds Max docs.

> Provenance note: SpatialGrammar (2604.*) and R3L (2605.*) are dated after the model's Jan-2026 cutoff; verifiers confirmed quotes via mirrors, and their *patterns* (face-anchor ON, invariant units) stand regardless of citation recency. The SpatialGrammar BEV-index claim was the only non-unanimous vote (2-1).
