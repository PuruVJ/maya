<script lang="ts">
	// Headless: the single per-frame loop that steps every ambient agent (cats + people) through the
	// shared manager — one grid rebuild + one flock pass for all of them, instead of N separate loops.
	import { useTask } from '@threlte/core';
	import { agentManager } from '$lib/agents.svelte';

	useTask((dt) => agentManager.tick(dt));
</script>
