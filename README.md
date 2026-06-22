# World

**One quiet world we're all secretly building together — that lives on without us.**

A 3D world that builds itself. Type a sentence and a fine-tuned language model — running **100% in your
browser** — turns it into things in the world: villages, forests, lamps, paths. Walk around in first person,
build in place, and leave. The world keeps living while you're gone: animals hunt, breed, starve, and age;
settlements grow and decay into ruins; "Mother Nature" (an in-browser AI director) nudges it all along.

It's [Death Stranding](https://en.wikipedia.org/wiki/Death_Stranding)-style async: you never see other
players, only their *effects* — the structures they built, the forests they cleared, the cities that grew.
Their work, the AI's, and the sim's own evolution all blur into one felt presence: the world breathing around you.

> **Status:** active prototype. The single-player living world (build + ecosystem + day/night + decay) runs
> today; the shared-world backend is rolling out incrementally. Expect rough edges.

---

## How it works

The founding principle is **"ops/regions, not geometry"** — never ship millions of objects, ship the *recipe*.

- **The LLM emits grammar-constrained build ops, not coordinates.** Two small fine-tunes (`WorldGen-1.5B`
  default, `WorldGen-0.5B` ~280 MB) run locally via [WebLLM](https://github.com/mlc-ai/web-llm) / WebGPU.
  A deterministic renderer turns ops into geometry; the model never does math or places things by hand.
- **Rust owns all the heavy compute.** The ecosystem sim (agents, steering, eco, clock) is a Rust/WASM crate
  (`crates/worldsim`) running in a Web Worker. JavaScript + three.js do *rendering only*.
- **No true randomness.** The world is a pure function of `(seed, tick)` — fully replayable and
  fast-forwardable. All randomness flows through a seeded hash-RNG; `Math.random()` / `Date.now()` in game
  logic is a bug, by design.
- **Visuals are procedural shaders**, not big asset downloads — grass, water, sky, weather are GPU-generated.

Design docs live in [`docs/`](./docs) — start with [`docs/big-world.md`](./docs/big-world.md) and
[`docs/self-sustaining-world.md`](./docs/self-sustaining-world.md).

## Tech stack

SvelteKit 5 (runes) · three.js + Threlte · Rust → WASM (`wasm-pack`) · WebLLM · Cloudflare (Workers + D1) ·
Drizzle ORM · Tailwind 4 · Vitest.

## Develop

```sh
pnpm install
pnpm build:wasm          # compile the Rust sim crate → static/worldsim/ (needs the Rust toolchain + wasm-pack)
pnpm db:migrate:local    # create the D1 tables in the local .wrangler SQLite (shared-world + telemetry)
pnpm dev                 # http://localhost:5173
```

The DB layer is [Drizzle](https://orm.drizzle.team) over Cloudflare D1; the schema lives in
[`src/lib/server/db/schema.ts`](./src/lib/server/db/schema.ts) and is the single source of truth. After
changing it, run `pnpm db:generate` to emit a new migration into `drizzle/`, then apply it (commands below).
The app degrades gracefully with no DB bound, so `pnpm dev` works even before you migrate — you just won't get
shared-world persistence locally.

The fine-tuned model weights (~840 MB / ~280 MB) are **not** in the repo. On first run the app fetches them
from the configured URL (see `TUNED[...].url` in [`src/lib/llm.svelte.ts`](./src/lib/llm.svelte.ts)); for
local model work, drop them in `static/models/` (git-ignored). See [`training/`](./training) to reproduce the
fine-tunes.

Useful scripts:

| Command | What |
| --- | --- |
| `pnpm test` | unit tests (Vitest) |
| `pnpm test:rust` | Rust sim tests (`cargo test`) |
| `pnpm test:llm` | model emit tests (downloads a small test model) |
| `pnpm check` | `svelte-check` type/diagnostics |
| `pnpm build` | production build |

## Deploy (Cloudflare)

The app targets **Cloudflare Workers + Static Assets** (adapter configured inline in
[`vite.config.ts`](./vite.config.ts) — there is intentionally no `svelte.config.js`).

1. **Create the D1 database** (persists the shared world + telemetry):
   ```sh
   wrangler d1 create world
   ```
   Paste the returned `database_id` into [`wrangler.jsonc`](./wrangler.jsonc) (replacing `"local-dev"`).
2. **Apply the migrations** to the remote DB (creates the tables from the Drizzle schema):
   ```sh
   pnpm db:migrate     # wrangler d1 migrations apply world --remote
   ```
3. **Build the WASM sim** and **deploy**:
   ```sh
   pnpm build:wasm
   pnpm deploy         # vite build && wrangler deploy
   ```
4. **Custom domain** (`world.puruvj.dev`): add it under the Worker's *Settings → Domains & Routes*, or via
   `wrangler`. Cloudflare manages the DNS record + TLS automatically.

The big model weights are excluded from CF assets (25 MiB/file limit) via
[`static/.assetsignore`](./static/.assetsignore); host them on R2 or Hugging Face and point `TUNED[...].url`
at the absolute URL.

> **No secrets live in this repo.** Cloudflare auth is handled by your local `wrangler` login (OAuth) — there
> are no API keys or account IDs committed. Don't add any; use Wrangler/`.env` (git-ignored) for anything secret.

## A note on the shared world

The world-persistence and telemetry endpoints (`/api/world`, `/api/telemetry`) are **public and anonymous** —
that's the design (one world, no accounts). They're protected by per-IP token-bucket rate limiting + payload
size caps ([`src/lib/server/ratelimit.ts`](./src/lib/server/ratelimit.ts)), which stops casual griefing but is
**not** a hard security boundary. The world is intentionally resettable. The robust per-region authority
(Durable Objects, device-token quotas, moderation) is specced in [`docs/big-world.md`](./docs/big-world.md).

## License

[MIT](./LICENSE) © Puru Vijay
