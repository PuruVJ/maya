import { defineConfig } from 'vitest/config';

// Opt-in, slow LLM suite (runs the real model via node-llama-cpp). Kept OUT of the default
// `pnpm test` (which only globs src/**). Run with `pnpm test:llm`.
export default defineConfig({
	test: {
		include: ['tests/llm/**/*.test.ts'],
		environment: 'node',
		testTimeout: 600_000,
		hookTimeout: 300_000,
		disableConsoleIntercept: true, // let the scorecard print straight through
		pool: 'forks' // native addon is happier in a forked process
	}
});
