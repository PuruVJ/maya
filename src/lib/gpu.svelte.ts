// Which render backend is live this session, for the WebGPU migration (see the perf-foundation-plan memory +
// the migration TODO). Set ONCE at startup from the ?webgpu flag (+page), never changes. Components whose custom
// GLSL hasn't been ported to TSL yet read `gpu.webgpu` and skip rendering on the WebGPU path — raw ShaderMaterial
// (and Threlte's shader-based Sky/Stars) throw under WebGPU, so until a component is ported we simply don't mount
// it there (the scene renders PLAIN, not broken). Each `{#if !gpu.webgpu}` gate is removed when that shader lands
// in TSL. $state so the gates are reactive even though it's effectively constant after init.
export const gpu = $state({ webgpu: false });
