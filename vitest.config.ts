import { defineConfig } from 'vitest/config';

// Engine tests are pure TS (no SvelteKit/Svelte plugin needed) — keep this config minimal
// and separate from vite.config.ts so the sveltekit() plugin doesn't load under test.
export default defineConfig({
	test: {
		include: ['src/**/*.{test,spec}.{js,ts}'],
		environment: 'node'
	}
});
