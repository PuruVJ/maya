# World-Data Boundary Rewrite — Design

Killing the JSON round-trips between JS and the Rust/WASM worldgen, so the world stays smooth even left running for days. From a 10-agent architecture study (5 boundary maps + an adversarial SharedArrayBuffer-feasibility verdict + 3 competing designs + a synthesized plan). Evidence is `file:line`.

> **Status: DESIGN ONLY. Nothing here is built.** Implementation is gated on explicit greenlight.

## TL;DR

Your instinct is correct: stop passing an ever-growing JSON blob; share **binary structs**. But the study found two things worth knowing before we build:

1. **SharedArrayBuffer is the wrong tool for *this* cost.** The expensive worldgen ops (`build_ops`, `settlement_ops`, …) run in a **second wasm instance on the *main thread*** (`math.ts`). There is **no thread boundary** to share across — the cost is purely *serialize → parse → serialize → parse* of a JSON string, on one thread. A cross-thread shared buffer buys nothing there. The fix is to **stop serializing** (hand Rust a flat binary view it reads in place), not to share memory faster.
2. **"Running for days" is actually two separate costs.** Streaming already bounds the *live* structure set to ~220 near the player, so the per-event payload is **not** O(world-age) — it just *feels* unbounded because of the JSON. But the **save blob** genuinely is O(world-age) and is a real problem. Different fixes.

So: a **binary struct-of-arrays (SoA) ABI** replaces the JSON, shipped op-by-op with zero new headers and zero determinism risk. SAB is **verified feasible** (the model-load fear was partly misplaced) and kept in reserve for a later, orthogonal win (zero-copy render reads).

## The current boundary (what's bad)

There are **two** wasm instances of the same `.wasm` bytes:
- **Worker sim** (`worldsim.worker.ts` ⟷ `world.rs`): owns the agent simulation, posts per-tick agent snapshots back via **true zero-copy Transferables** (`.slice()` owned buffers + postMessage transfer-list, ~28 KB/1000 agents, `worldsim.worker.ts:200-219`). **This is already good** — cost scales with live agent count + per-tick deltas, never with world size.
- **Main-thread "math glue"** (`math.ts` ⟷ `worldgen.rs`): stateless worldgen ops — `build_ops`, `well_ops`, `grave_site`, `vegetation_ops`, `settlement_ops`, etc. **This is the problem.** Each call:
  - `JSON.stringify(world-subset)` on the JS side (`Scene.svelte:364` `structWorld()` = every non-creature object; `Scene.svelte:660` `fenceWorld` = every fence panel) →
  - Rust `jzon`-parses it and does a **full `world["objects"].members()` linear scan** (`worldgen.rs:381,445,510,535,341`) →
  - returns a JSON op string → JS `JSON.parse`s it (`math.ts:211-259`).

  That's an O(n) serialize + O(n) parse + O(n) scan + O(n) parse, on **every** build / well / grave / vegetation / fence event, where n = the live structure count.

## The key correction: the live set is already bounded

`enforceLiveBudget` caps live structures at `STRUCT_BUDGET = 220` over a ~600 m 3×3 region span (`streaming.ts:35,191,204,228`); far structures are collapsed into `world.regions[*].statics` and are **not** in `world.objects`. So `structWorld()` packs ~220 structures whether the world is one minute or one week old — **the per-event payload is O(live)≈220, not O(world-age)**. The felt cost is the JSON machinery, not payload growth.

## SharedArrayBuffer — feasibility verdict

**Verdict: feasible, but not needed for the round-trip.** The adversarial check found:
- **It would *not* break the model load** (the `worldsim.worker.ts:13` fear is partly misplaced). The WebLLM model weights (`huggingface.co`, `llm.svelte.ts:22`) and model-lib wasm (`raw.githubusercontent.com`) are **cross-origin and CORS/CORP-clean**; WebLLM fetches them in default CORS mode. `COOP: same-origin` + `COEP: credentialless` passes them through. The only same-origin asset is the worldsim wasm glue — unaffected.
- **But SAB doesn't help the main-thread round-trip** (no thread is crossed). It only earns its `COOP/COEP` cost as a *separate, future* win: letting the 7 InstancedMesh renderers read a shared binary structure buffer **zero-copy** instead of scanning the `$state` proxy ~14×/frame.
- **Action if we ever want that:** add the two headers to `static/_headers` (Cloudflare emits it) **and** the (currently unset) Vite dev `server.headers`; use `credentialless` (not `require-corp`); verify `crossOriginIsolated === true` + model still loads in a real Chrome run. Chrome/Edge only — already the supported set (WebGPU gate).

## Recommended architecture — a binary SoA ABI (Design 3)

`world.objects` (the Svelte `$state` array) **stays the single source of truth** — save / share / `fastForward` / decay / `repairIds` all read its non-render fields (`gene/keep/genome/dead`, `world.ts:15-23`) and must not move. We change only the **ABI** between JS and the worldgen wasm:

- One main-thread **packer** walks the live non-creature objects **once per frame** (same memoize discipline as today's `_structJson`) into a **reused `Float64Array`** with a fixed stride — `[x, z, kindCode, rot, scaleX, keepFlag]` — plus a parallel `zones` `Float64Array` and a JS-side `idTable: string[]` indexed by row.
- Rust receives it as a wasm-bindgen `&[f64]` view over linear memory (the way the worker already builds typed views over `xs_ptr`, `worldsim.worker.ts:99-106`) and **iterates by stride** instead of `members()`. No JSON parse.
- Rust returns a single packed `Float64Array` op stream — `[opCode, kindCode, x, z, rot, scaleX, removeRow, …]` — which JS decodes in a tight loop. **REMOVE comes back as a row-index**, mapped through `idTable` (ids never cross into Rust).
- `settlement_ops`' position-diff (`worldgen.rs:341-365`) still works because the existing fence panels are *in* the SoA (kindCode = fence) with their row-index.

**Why this wins:** kills the serialize **and** the parse on both sides; O(local) by construction (pinned to the bounded live set); **no shared memory, no COOP/COEP, no moved compute, zero determinism risk** (same numbers in/out, just unboxed from f64 lanes instead of JSON — `mulberry32/hash1` untouched, `worldgen.test.ts` parity holds).

### Gotchas the layout must respect
- **`kindCode` enum = one shared source of truth** (Rust + JS), pinned by a parity test — else houses mis-place as fences.
- **`rot` unit mismatch is load-bearing:** fences/settlement store **degrees**, graves store **radians-read-as-degrees** (`Graves.svelte:9`). Carry the exact stored f64; **never normalize**.
- **`scale`/`color` are optional** (default `[1,1,1]`, `color ?? 0`): use a **sentinel** (`scaleX = 0` ⇒ "default 1"), not a missing lane, or builds get invisible scale-0 houses.
- **REMOVE-by-row-index is stale** if the packer rebuilds between produce and apply: apply ops against the **same** snapshot that produced them, within the frame.
- `&[f64]` *copies* into linear memory per call — trivial at 220×6, but claim **zero-parse**, not zero-copy. If `STRUCT_BUDGET` ever hits thousands, drop geometry to `Float32Array`.

## The three distinct costs (and what fixes each)

| Cost | What it is | Fix | When |
|---|---|---|---|
| **A — the JSON round-trip** | `JSON.stringify(~220)`+parse on every structure event (`math.ts`/`Scene.svelte:364,660`) | **Binary SoA ABI** (above) | **Now** (this doc's plan) |
| **B — the days-away SAVE blob** | `saveWorld` structured-clones the **whole** World incl. *every dormant region's verbatim `statics[]`* into IndexedDB **every 1 s** + on tab-hide (`worldStore.ts:40-49`, `+page.svelte:262`). **Genuinely O(world-age)** — a week-old world copies a large blob every second. | **Incremental / binary persistence** — a Rust-owned append-mostly structure arena saved as one `ArrayBuffer` (write-dirty-page / append-delta), or at minimum a dirty-region save that doesn't re-serialize untouched dormant regions. | **Next** (separate, also real) |
| **C — render proxy scans** | 7 InstancedMesh renderers each scan the `$state` proxy ~2×/frame (~14 full scans/frame) | **Dirty-epoch mirror** (cheap) or a **SAB render projection** (zero-copy, needs the headers) | **Deferred** (orthogonal) |

**Important:** the recommended binary-SoA fix solves **A**. It does **not** by itself solve **B** — and **B is the literal "running for days" failure** you described. So the doc proposes A now (low-risk, high-leverage, unblocks the felt jank) and treats **B** as the immediate follow-on. If we want to solve A and B *together* with one mechanism, that's the **stateful Rust StructureStore** (see "Alternatives"), a bigger rewrite we can graduate to.

## Migration plan (op-by-op, each shippable + parity-tested)

The op ABI is already a clean add/remove `Op[]` that `Scene` applies identically regardless of producer, so we migrate one function at a time behind it:

1. **Pin the `kindCode` enum + SoA stride contract** (Rust + JS) + a parity test. *Pure addition; JSON path still runs.* — low
2. **Add `well_ops_bin`** (smallest scan) alongside the JSON `well_ops`; parity-test bin vs JSON. — low
3. **Build the shared per-frame `packStructs()`** (reused `Float64Array` + `idTable`); feed `well_ops_bin` only. — medium
4. **Cut `Scene`'s `wellOps` call to binary** end-to-end; live smoke that wells still place. — medium
5. **Migrate `build_ops`**, keeping its global `HOUSE_CAP`/`FOUND_GAP`/colony scans over the full live SoA (bounded 220) so distinct-town founding is preserved; `cargo test scenario_` confirms no merge/over-build. — medium
6. **Migrate `grave_site`** (carry exact stored `rot`). — low
7. **Migrate `vegetation_ops`**. — low
8. **Migrate `settlement_ops` last** (heaviest): centroid clustering + position-diff read fence/home rows from the SoA; REMOVE returns row-index; drop `fenceWorld` stringify. Walls stay idempotent across build/decay. — high
9. **Delete the dead JSON `*_ops` surface** from `math.ts`/`worldgen.rs` (keep pure helpers + the rare BuildBar one-shots). — low
10. **Then tackle Cost B** (incremental save) and, if profiling still shows render scan pain, **Cost C** (dirty-epoch mirror or SAB projection behind a `crossOriginIsolated` flag).

Each step keeps the game working and is reversible; `worldgen.test.ts` pins byte-parity throughout.

## Alternatives considered (and why not, for now)

- **Move worldgen into the worker (Design 1):** structures become a Rust SoA beside the sim; ops run in the worker, only deltas cross. Elegant and solves A+B+the render projection together — **but** `world.objects` is reassigned wholesale every frame by `collapseRegion`/`enforceLiveBudget` (`streaming.ts:71,228`) and is the save source, so a second owner must absorb **all** of streaming + budget + save at once: a big-bang rewrite. It's the eventual endgame (and aligns with "Rust owns compute" + determinism), but not the first move while the live set stays bounded.
- **SAB shared store (Design 2):** right tool only for Cost C (render reads), wrong for Cost A. Kept in reserve behind a feature flag; feasibility is proven, so it's available when render reads become the bottleneck.

## LOCKED DECISIONS & BUILD ORDER (2026-06-27)

User decisions: **(1)** strict live cap, **`STRUCT_BUDGET = 250`** (done, `streaming.ts:35`) — bounds the SoA, so geometry can stay `f64`/`f32` without worrying about thousands live. **(2)** do the **big rewrite from the structure store** (not the incremental-only ABI). **(3)** **SAB now** (enable cross-origin isolation + zero-copy render reads). **(4)** **migrate to binary**, perf-conscious throughout. **(5, new)** persistence (Cost B) gets a **surgical multi-key IndexedDB** fix, *separate* from the structure rewrite.

### Cost B — surgical multi-key IndexedDB (DECOUPLED, ships first, fixes the literal "days-away")
Today `saveWorld` writes the **whole** World under one key `current` every 1 s, re-serializing every dormant region's `statics[]` (`worldStore.ts:11,45`) — O(world-age) every second. Fix = independently-written keys:
- `meta` — name/ground/sky/start/savedAt (+ `terrain` heightfield, static, written once). Tiny.
- `live` — `{objects (live), zones, paths}`. Bounded by the live caps (250 struct + 240 creature). **Written every 1 s.**
- `region:<key>` — each dormant region aggregate `{counts, gene, statics, lastTick}`. **Written ONCE on `collapseRegion`, deleted on `wakeRegion`** (`streaming.ts:51,91`) — never touched by the 1 s autosave.

So per-second save = O(live) forever; dormant structures are written once-per-collapse, not re-serialized each second. `loadWorld` reads `meta`+`terrain`+`live`+ all `region:*` (cursor), reassembles the World, and falls back to the legacy `current` key once for migration. IndexedDB's async is exactly right. Surgical: `worldStore.ts` (the multi-key API) + `+page.svelte` (autosave→`saveLive`, load→new `loadWorld`) + `streaming.ts` (collapse→`saveRegion`, wake→`deleteRegion`).

### Cost A + C — the structure-store rewrite (the big one)
**Target end-state:** a Rust-owned **`StructureStore`** (binary SoA: `id,kind,x,z,rot,sx,sy,sz,color,keep,region` + a free-list + a ~64 m spatial grid) is the single source of truth for structure geometry. Worldgen ops run against it (stateful, no JSON, O(local) via the grid). Its bounded live slice (≤250) lives in a **SharedArrayBuffer** that **both** the render thread (zero-copy `instanceMatrix` memcpy, replacing the ~14 proxy scans/frame) **and** the worker-sim (reads structure positions for avoidance/migration — no more `setRefuges` copies) read directly. `world.objects` keeps only the non-render sim fields (gene/genome/keep/dead) in a parallel side-table for the surgical persistence above.

**Ordered, shippable phases** (each keeps the game working + parity/scenario-tested):
- **A0 — StructureStore + binary ABI, stateful, main-thread first.** Stand up the Rust SoA + spatial grid in the worldgen wasm; migrate ops one at a time (well→build→grave→veg→settlement) to read the store via the grid + return binary op streams; seed it from `world.objects` at load. Kills the JSON round-trip (Cost A) with no threading/headers yet. `world.objects` still mirrors for render+save.
- **C — SAB render projection.** Add `COOP:same-origin` + `COEP:credentialless` to `static/_headers` **and** the (unset) Vite dev `server.headers`; verify `crossOriginIsolated===true` + the WebLLM model still loads in real Chrome. The store writes its live slice into a SAB (per-kind SoA + `[epoch,counts…]` header, Atomics-published, double-buffered single-writer); the 7 InstancedMesh renderers read by offset, rebuild on `epoch` change. Kills the render proxy scans (Cost C).
- **A1 — the endgame: one shared store.** Relocate the store so the worker-sim reads the same SAB directly (drop `setRefuges`); `world.objects` becomes a thin render/sim-field mirror. The boundary is now fully binary/shared — O(local change) to write, O(1) to "pass", zero JSON.

**Perf ceiling after all phases:** per structure-event comms = a few floats; per-frame render = a memcpy on the rare epoch bump; per-second save = O(live); the sim reads structures with zero copies. A world running for days has the same cost as a fresh one.

## Open questions for you

1. **Is `STRUCT_BUDGET = 220` the permanent live ceiling?** If you later want *thousands* live at once (not just total), we drop SoA geometry to `Float32Array` and revisit the worker-arena/SAB path.
2. **Cost B priority:** the binary ABI fixes the per-event jank but **not** the 1 s save-blob growth (the true days-away failure). Do we do A then immediately B, or fold both into the bigger stateful-arena rewrite from the start?
3. **Render reads (Cost C):** worth the COOP/COEP headers now for zero-copy instanced rendering, or strictly deferred until A/B prove out?
4. **BuildBar one-shots** (forest/lake/city, `math.ts:40-42`): migrate to binary too, or leave them JSON since they're rare + user-triggered?
