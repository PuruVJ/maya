<script lang="ts">
	// Sits inside the Threlte canvas so it can wrap renderer.render with a real GPU TIMER QUERY (the only way to see
	// actual GPU time — the thing heating the fans — since the browser otherwise hides it). Also times the CPU cost of
	// the draw submission, the frame interval (fps + worst frame = hitch/wake-storm detector), and reads draw calls +
	// triangles. Flushes to the `profiler` store ~4×/s for the HUD overlay. Toggle the overlay with 'p'.
	import { onMount } from 'svelte';
	import { useTask, useThrelte } from '@threlte/core';
	import { profiler } from '$lib/profiler.svelte';

	const { renderer } = useThrelte();
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	const gl = renderer.getContext() as any;
	const ext = gl.getExtension('EXT_disjoint_timer_query_webgl2');
	const query: WebGLQuery | null = ext ? gl.createQuery() : null;
	let queryActive = false;
	let lastGpuMs = -1;
	let renderCpuMs = 0;

	// wrap the renderer's draw: bracket it with the GPU timer query + time the CPU submission. One query in flight at a
	// time (its result lands a few frames later) → a sampled GPU ms, plenty for spotting the bottleneck.
	const orig = renderer.render.bind(renderer);
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	renderer.render = ((scene: any, camera: any) => {
		const t0 = performance.now();
		const doQ = !!(ext && query && !queryActive);
		if (doQ) gl.beginQuery(ext.TIME_ELAPSED_EXT, query);
		orig(scene, camera);
		if (doQ) {
			gl.endQuery(ext.TIME_ELAPSED_EXT);
			queryActive = true;
		}
		renderCpuMs = performance.now() - t0;
	}) as typeof renderer.render;

	onMount(() => {
		const onKey = (e: KeyboardEvent) => {
			if (e.key === 'p' && !(document.activeElement instanceof HTMLInputElement)) profiler.on = !profiler.on;
		};
		window.addEventListener('keydown', onKey);
		return () => {
			window.removeEventListener('keydown', onKey);
			renderer.render = orig; // restore the un-wrapped render when the scene unmounts
		};
	});

	let frames = 0;
	let sumFrame = 0;
	let maxFrame = 0;
	let sumRender = 0;
	let acc = 0;
	let last = performance.now();
	useTask(() => {
		const now = performance.now();
		const dt = now - last;
		last = now;
		frames++;
		sumFrame += dt;
		if (dt > maxFrame) maxFrame = dt;
		sumRender += renderCpuMs;
		// poll the GPU timer (its result is ready a few frames after endQuery; GPU_DISJOINT invalidates the timing)
		if (queryActive && ext && query && gl.getQueryParameter(query, gl.QUERY_RESULT_AVAILABLE) && !gl.getParameter(ext.GPU_DISJOINT_EXT)) {
			lastGpuMs = (gl.getQueryParameter(query, gl.QUERY_RESULT) as number) / 1e6;
			queryActive = false;
		}
		acc += dt;
		if (acc >= 250) {
			profiler.fps = Math.round((1000 * frames) / sumFrame);
			profiler.frameMs = sumFrame / frames;
			profiler.frameMax = maxFrame;
			profiler.renderMs = sumRender / frames;
			profiler.gpuMs = lastGpuMs;
			profiler.calls = renderer.info.render.calls;
			profiler.tris = renderer.info.render.triangles;
			frames = sumFrame = maxFrame = sumRender = acc = 0;
		}
	});
</script>
