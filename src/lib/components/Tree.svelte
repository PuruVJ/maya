<script lang="ts">
	// A placed tree / pine, rendered as a SHADER rather than a static primitive cluster (the user's
	// shader-first direction). Same composed parts as the kinds registry, but the materials are patched
	// (onBeforeCompile): a wind-sway vertex shader bends the whole tree (canopy ≫ trunk, phased per-tree so
	// the forest ripples), and a fragment fbm dapples the foliage + darkens its underside so the canopy
	// reads as leaves, not a flat ball. Shares the global wind clock (wind.uTime, ticked once by Scene).
	// Pop-in matches Prop. Scene routes tree/pine here; everything else stays on Prop.
	import { untrack } from 'svelte';
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { kindDef } from '$lib/kinds';
	import { partGeo, leafColorHex } from '$lib/sharedAssets';
	import { wind, WIND_NOISE, swayVertex } from '$lib/wind';
	import { applyWeather } from '$lib/weather';
	import type { WorldObject } from '$lib/world';

	let { obj }: { obj: WorldObject } = $props();
	// geometry/material are snapshotted once (kind/colour don't change after build); pos/rot are read
	// REACTIVELY in the markup below so the move tool / LLM move ops actually relocate the tree
	const O = untrack(() => ({ kind: obj.kind, color: obj.color, sx: obj.scale?.[0] ?? 1, sy: obj.scale?.[1] ?? 1, sz: obj.scale?.[2] ?? 1 }));
	const rot = $derived(((obj.rot ?? 0) * Math.PI) / 180);
	const def = kindDef(O.kind);

	const CANOPY = new Set(['sphere', 'cone', 'pyramid']);
	// broadleaf trees take a varied leaf colour from the SHARED palette (hashed by id → stable, blends with the
	// ambient forest); pines stay their evergreen colour; paint always wins.
	const idHash = (s: string) => {
		let h = 2166136261;
		for (let i = 0; i < s.length; i++) ((h ^= s.charCodeAt(i)), (h = Math.imul(h, 16777619)));
		return (h >>> 0) / 4294967296;
	};
	const canopyTint = untrack(() => (O.kind === 'tree' && !O.color ? leafColorHex(idHash(obj.id)) : undefined));

	// One sway/foliage material per part — created once. (Identical shader source → three caches the
	// compiled program across all trees, so this stays cheap even for a scattered forest.)
	function makeMat(color: string, canopy: boolean): THREE.MeshStandardMaterial {
		const m = new THREE.MeshStandardMaterial({ color, flatShading: true });
		m.onBeforeCompile = (shader) => {
			shader.uniforms.uTime = wind.uTime;
			shader.vertexShader = shader.vertexShader
				.replace('#include <common>', '#include <common>\nuniform float uTime;\nvarying vec3 vLocalPos;')
				.replace('#include <begin_vertex>', '#include <begin_vertex>\n' + swayVertex(0.05));
			shader.fragmentShader = shader.fragmentShader.replace(
				'#include <common>',
				'#include <common>\nvarying vec3 vLocalPos;' + (canopy ? '\n' + WIND_NOISE : '')
			);
			if (canopy) {
				shader.fragmentShader = shader.fragmentShader.replace(
					'#include <color_fragment>',
					/* glsl */ `#include <color_fragment>
					float fol = windFbm(vLocalPos.xz * 1.4 + vLocalPos.y * 0.6);
					diffuseColor.rgb *= 0.74 + 0.4 * fol;                                  // dappled leaf clumps
					diffuseColor.rgb *= 0.7 + 0.3 * smoothstep(-0.5, 3.2, vLocalPos.y);     // darker underside`
				);
			}
			// rain darkens + snow caps the tree, like every other surface. Canopy: no glossy sheen on leaves
			// (sheen 0); trunk: a little wet-bark sheen. Snow settles on the up-facing canopy/branch faces.
			applyWeather(shader, canopy ? 0.2 : 0.26, canopy ? 0.0 : 0.4);
		};
		return m;
	}

	// bake each part's offset INTO its geometry (clone so the shared cache stays immutable) so the vertex's
	// local Y is its true height above the trunk base → the sway lever bends the tree as one coherent piece.
	const parts = untrack(() =>
		def.parts.map((p) => {
			const canopy = CANOPY.has(p.geo);
			// a broadleaf canopy's resting colour is its varied leaf tint (so unpaint reverts to it, not flat green)
			const base = canopy && canopyTint ? canopyTint : p.color;
			return {
				geo: partGeo(p).clone().translate(p.pos[0], p.pos[1], p.pos[2]),
				mat: makeMat(canopy ? (obj.color ?? base) : p.color, canopy), // paint tints leaves, not trunk
				canopy,
				base
			};
		})
	);
	// react to paint ("make the tree red") → re-tint the canopy materials (trunk keeps its bark colour)
	$effect(() => {
		const c = obj.color;
		for (const part of parts) if (part.canopy) part.mat.color.set(c ?? part.base);
	});

	// pop-in: spring up from nothing with a little overshoot (easeOutBack), same as Prop
	let model = $state<THREE.Group>();
	let t = 0;
	const eob = (x: number) => {
		const c1 = 1.70158;
		const c3 = c1 + 1;
		return 1 + c3 * (x - 1) ** 3 + c1 * (x - 1) ** 2;
	};
	// settle, then STOP the task — a finished pop-in needn't keep a per-frame callback alive (see Prop)
	const { stop } = useTask((dt) => {
		if (!model) return;
		t = Math.min(1, t + dt * 3.5);
		const s = eob(t);
		model.scale.set(O.sx * s, O.sy * s, O.sz * s);
		if (t >= 1) stop();
	});
</script>

<T.Group position={[obj.pos[0], obj.pos[1], obj.pos[2]]} rotation={[0, rot, 0]} userData={{ objectId: obj.id }}>
	<T.Group bind:ref={model} scale={[0, 0, 0]}>
		{#each parts as part, i (i)}
			<T.Mesh geometry={part.geo} material={part.mat} castShadow receiveShadow />
		{/each}
	</T.Group>
</T.Group>
