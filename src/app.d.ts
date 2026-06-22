// See https://svelte.dev/docs/kit/types#app.d.ts
// for information about these interfaces
/// <reference types="@cloudflare/workers-types" />

declare global {
	namespace App {
		// interface Error {}
		// interface Locals {}
		// interface PageData {}
		// interface PageState {}
		// No Cloudflare bindings — the game is 100% local (single-player, IndexedDB). The only server route left is
		// the dev-only /api/debug log sink, which uses node:fs, not platform bindings. Deploy is a static SPA.
	}
}

export {};
