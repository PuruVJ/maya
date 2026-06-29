// FRAME PROFILER — find what's actually heating the GPU/CPU. Written by Profiler.svelte (inside the Threlte loop,
// where it can wrap renderer.render with a real GPU timer query) and read by the HUD overlay in +page. Values are
// flushed ~4×/s (not per frame) so the reactive overlay doesn't itself become a cost. gpuMs is the headline for "why
// are my fans on": the actual GPU time per render (shadow + scene); -1 means the WebGL timer-query ext is unavailable.
class Profiler {
	on = $state(true); // overlay visible (toggle with 'p') — default on during the perf investigation
	fps = $state(0);
	frameMs = $state(0); // average frame interval over the window
	frameMax = $state(0); // worst single frame in the window (hitch detector)
	gpuMs = $state(-1); // GPU time of the render (timer query) — the fan signal; -1 = unsupported
	renderMs = $state(0); // CPU time spent in renderer.render (draw-call submission)
	calls = $state(0); // draw calls — high → CPU/driver bound (want more instancing)
	tris = $state(0); // triangles — high → GPU vertex/fill bound (want LOD / less geometry)
}

export const profiler = new Profiler();
