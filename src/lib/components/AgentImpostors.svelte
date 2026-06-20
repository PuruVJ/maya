<script lang="ts">
	// Far agents (LOD2, beyond the detail range) drop their articulated group and render here as flat
	// instanced silhouettes — two InstancedMeshes (animals + people) → 2 draw calls for the whole far
	// crowd instead of ~6–8 each. The manager assigns LOD; the full Critter/Npc components hide themselves
	// when far (group.visible = lod !== 2). Each far animal takes its SPECIES colour (instanceColor) and
	// SIZE (from its body radius) so a distant herd of dinos reads as big green shapes, not identical orange
	// cat-boxes. See docs/crowd-separation.md §3.4.
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { heightAt } from '$lib/terrain';
	import { agentManager } from '$lib/agents.svelte';
	import { wind } from '$lib/wind';
	import { clock } from '$lib/clock';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();

	const MAX = 1024; // far-crowd cap (was 256 → a scattered 1000-strong herd silently vanished past 256)
	const PERSON_TINT = '#4a73c4'; // fallback shirt if an agent carries no tint
	// per-species far colour + the base radius (0.35·scale) used to recover each agent's scale from its radius
	const KIND_COLOR: Record<string, string> = {
		cat: '#e8924a',
		lion: '#c79a4b',
		rabbit: '#eceae3',
		kangaroo: '#b07a4a',
		dinosaur: '#5f7d4a'
	};
	const catGeo = new THREE.BoxGeometry(0.45, 0.45, 0.95).translate(0, 0.3, 0);
	const personGeo = new THREE.CapsuleGeometry(0.3, 1.2, 4, 8).translate(0, 0.9, 0);
	// white base so the per-instance colour shows through unmodulated. A tiny VERTEX-SHADER bob (shared wind
	// clock + per-instance phase) gives the far crowd a walking lilt so a distant herd reads as ALIVE, not a
	// field of frozen boxes — costs nothing on the CPU (the matrices we already write stay still). The hop is
	// in LOCAL space → the instance matrix scales it, so a far dino bobs more than a far cat. People stay still
	// (a lurching capsule looks worse than a steady one).
	const animalMat = new THREE.MeshStandardMaterial({ color: 0xffffff, flatShading: true });
	animalMat.onBeforeCompile = (shader) => {
		shader.uniforms.uTime = wind.uTime;
		shader.vertexShader = shader.vertexShader
			.replace('#include <common>', '#include <common>\nuniform float uTime;\nattribute vec3 aEye;\nvarying vec3 vEye;\nvarying vec3 vLocal;')
			.replace(
				'#include <begin_vertex>',
				/* glsl */ `#include <begin_vertex>
				vEye = aEye;
				vLocal = position; // undisplaced local pos → fixes the eye spots on the body
				float iPhase = float(gl_InstanceID) * 1.7;
				transformed.y += max(0.0, sin(uTime * 4.0 + iPhase)) * 0.06; // footfall hop (up only — feet never sink)
				transformed.x += sin(uTime * 2.0 + iPhase) * 0.02;            // gentle weight-shift sway`
			);
		// TWO glowing eyes on the FRONT (+Z) face at head height → DISTANT EYESHINE: a far predator glints at you
		// from the dark/fog. Per-instance colour×intensity (aEye, baked night×predator below) → 0 in daytime, so
		// the day crowd is byte-identical. Additive emissive only (can't break the geometry).
		shader.fragmentShader = shader.fragmentShader
			.replace('#include <common>', '#include <common>\nvarying vec3 vEye;\nvarying vec3 vLocal;')
			.replace(
				'#include <emissivemap_fragment>',
				/* glsl */ `#include <emissivemap_fragment>
				float eyeFront = step(0.40, vLocal.z); // the +Z (forward) face only
				float dEye = min(distance(vLocal.xy, vec2(0.09, 0.44)), distance(vLocal.xy, vec2(-0.09, 0.44)));
				float eyeGlow = 1.0 - smoothstep(0.0, 0.055, dEye); // bright at the eye centre, falls to 0 (edge0<edge1 → defined)
				totalEmissiveRadiance += vEye * (eyeFront * eyeGlow);`
			);
	};
	// people: white base + per-instance colour so a far crowd keeps each person's shirt tint (matching the near
	// NPCs) instead of a uniform-blue blob. No bob — a lurching capsule reads worse than a steady one.
	const personMat = new THREE.MeshStandardMaterial({ color: 0xffffff, flatShading: true });
	const animals = new THREE.InstancedMesh(catGeo, animalMat, MAX);
	const people = new THREE.InstancedMesh(personGeo, personMat, MAX);
	animals.castShadow = people.castShadow = false;
	animals.frustumCulled = people.frustumCulled = false;
	animals.count = 0;
	people.count = 0;
	const dummy = new THREE.Object3D();
	const col = new THREE.Color();
	// per-instance EYESHINE colour×intensity (vec3) for the far animals — predators a warm amber, prey a cool
	// pale glint, both scaled by how nocturnal it is; 0 in day / for corpses → no glow.
	const aEye = new Float32Array(MAX * 3);
	catGeo.setAttribute('aEye', new THREE.InstancedBufferAttribute(aEye, 3));
	const PREDATORS = new Set(['cat', 'lion', 'dinosaur']);
	const EYE_PRED = [1.0, 0.7, 0.28]; // amber (matches the near EYE_PRED_MAT)
	const EYE_PREY = [0.73, 0.85, 0.75]; // cool pale

	useTask(() => {
		let na = 0;
		let np = 0;
		const night = agentManager.nightValue; // 0 day … 1 night → eyeshine brightness
		agentManager.forEach((m) => {
			if (m.lod !== 2) return; // far agents (living OR corpses) → impostors; near ones draw in full
			const a = m.agent;
			a.interpolate(clock.alpha); // smooth the fixed-rate sim across render frames
			dummy.position.set(a.rx, heightAt(a.rx, a.rz, world.terrain), a.rz);
			dummy.rotation.set(0, a.rh, m.dead ? Math.PI / 2 : 0); // a far corpse lies tipped on its side
			if (m.kind === 'person') {
				if (np < MAX) {
					dummy.scale.setScalar(m.radius / 0.4); // recover the person's size (radius = 0.4·scale) → scaled people match near
					dummy.updateMatrix();
					people.setMatrixAt(np, dummy.matrix);
					people.setColorAt(np, col.set(m.tint ?? PERSON_TINT)); // each person's own shirt tint
					np++;
				}
			} else if (na < MAX) {
				dummy.scale.setScalar(m.radius / 0.35); // recover the species' body scale (radius = 0.35·scale)
				dummy.updateMatrix();
				animals.setMatrixAt(na, dummy.matrix);
				animals.setColorAt(na, col.set(m.tint ?? KIND_COLOR[m.kind] ?? KIND_COLOR.cat));
				// eyeshine: a far PREDATOR glints amber at you from the dark; prey a cool pale; corpses/day → none.
				// (A hunting predator is always within ~24m → a near Critter, not an impostor, so it glares there.)
				const glow = m.dead ? 0 : night * (PREDATORS.has(m.kind) ? 1.2 : 0.65);
				const ec = PREDATORS.has(m.kind) ? EYE_PRED : EYE_PREY;
				aEye[na * 3] = ec[0] * glow;
				aEye[na * 3 + 1] = ec[1] * glow;
				aEye[na * 3 + 2] = ec[2] * glow;
				na++;
			}
		});
		animals.count = na;
		people.count = np;
		animals.instanceMatrix.needsUpdate = true;
		people.instanceMatrix.needsUpdate = true;
		if (animals.instanceColor) animals.instanceColor.needsUpdate = true;
		if (people.instanceColor) people.instanceColor.needsUpdate = true;
		(catGeo.getAttribute('aEye') as THREE.InstancedBufferAttribute).needsUpdate = true; // eyeshine
	});
</script>

<T is={animals} />
<T is={people} />
