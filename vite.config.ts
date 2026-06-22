import adapter from '@sveltejs/adapter-cloudflare';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
	plugins: [
		tailwindcss(),
		sveltekit({
			compilerOptions: {
				// Force runes mode for the project, except for libraries. Can be removed in svelte 6.
				runes: ({ filename }) =>
					filename.split(/[/\\]/).includes('node_modules') ? undefined : true
			},

			// Deploy target is Cloudflare Static Assets (the game is a 100% client-side SPA — single-player, IndexedDB;
			// no server routes beyond the dev-only /api/debug log sink). Heavy model weights are NOT shipped as CF
			// assets (25 MiB/file limit) — see static/.assetsignore + the R2/HF note in wrangler.jsonc.
			adapter: adapter()
		})
	],
	// Force a SINGLE three.js instance across @threlte/core, /extras and /rapier.
	// Without this, Rapier pulls a second copy and Threlte's makeDefault camera breaks.
	resolve: {
		dedupe: ['three']
	},
	optimizeDeps: {
		include: ['three']
	}
});
