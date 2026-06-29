<script lang="ts">
	// Cheap CONTACT-SHADOW blobs that ground each creature. The directional light only budgets real shadows
	// to the nearest few agents (shadow-map cost), so a mid-distance herd floats. This draws a single
	// InstancedMesh of soft dark discs — one under each NEAR/MID agent (far ones are impostored + sub-pixel, so
	// skipped) on the terrain — in ONE draw call, so the crowd reads as planted, not pasted on. A soft radial
	// fade (tiny shader, no texture). Composes WITH the real cast shadow (centred contact-AO vs offset cast).
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { groundYCached } from '$lib/terrain';
	import { agentManager } from '$lib/agents.svelte';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();

	const MAX = 512;
	const geo = new THREE.CircleGeometry(1, 18).rotateX(-Math.PI / 2); // unit disc lying flat on the ground (xz)
	const mat = new THREE.ShaderMaterial({
		transparent: true,
		depthWrite: false,
		vertexShader: /* glsl */ `varying vec2 vUv; void main(){ vUv = uv; gl_Position = projectionMatrix * modelViewMatrix * instanceMatrix * vec4(position, 1.0); }`, // instanceMatrix MUST be applied for InstancedMesh (a raw ShaderMaterial doesn't get project_vertex) → without it every blob stacked at the world origin
		fragmentShader: /* glsl */ `
			varying vec2 vUv;
			void main(){
				float r = length(vUv - 0.5) * 2.0;          // 0 centre .. 1 rim
				float a = smoothstep(1.0, 0.15, r) * 0.34;  // soft dark blob, fades to nothing at the rim
				gl_FragColor = vec4(0.0, 0.0, 0.0, a);
			}
		`
	});
	const blobs = new THREE.InstancedMesh(geo, mat, MAX);
	blobs.frustumCulled = false; // positions written every frame from agent state; CPU bounds are meaningless
	blobs.renderOrder = -1; // draw early (just over the opaque ground) so it reads as a shadow, not an overlay
	blobs.count = 0;
	const dummy = new THREE.Object3D();

	useTask(() => {
		let n = 0;
		agentManager.forEach((m) => {
			if (n >= MAX || m.lod === 2) return; // far agents are impostored + tiny → their sub-pixel blob isn't
			// worth a heightAt + matrix compose each frame (matters when a 1000-strong herd is mostly far away)
			const a = m.agent;
			dummy.position.set(a.x, groundYCached(m, a.x, a.z, world.terrain) + 0.04, a.z);
			const s = m.radius * 1.9; // blob a touch wider than the body footprint
			dummy.scale.set(s, 1, s);
			dummy.updateMatrix();
			blobs.setMatrixAt(n++, dummy.matrix);
		});
		blobs.count = n;
		blobs.instanceMatrix.needsUpdate = true;
	});
</script>

<T is={blobs} />
