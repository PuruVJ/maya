// See https://svelte.dev/docs/kit/types#app.d.ts
// for information about these interfaces
declare global {
	// Minimal Cloudflare D1 surface (just what /api/world uses) — avoids pulling in @cloudflare/workers-types.
	interface D1PreparedStatement {
		bind(...values: unknown[]): D1PreparedStatement;
		first<T = unknown>(colName?: string): Promise<T | null>;
		run(): Promise<unknown>;
	}
	interface D1Database {
		prepare(query: string): D1PreparedStatement;
	}

	namespace App {
		// interface Error {}
		// interface Locals {}
		// interface PageData {}
		// interface PageState {}
		// Cloudflare bindings exposed to endpoints. `DB` is the world D1 database (wrangler.jsonc). Optional
		// because it's absent in `vite dev` without the platform proxy → endpoints degrade to the client's cache.
		interface Platform {
			env?: { DB?: D1Database };
		}
	}
}

export {};
