<script lang="ts">
	// SIMPLE stylized water for a lake/pond zone. Deliberately NOT a heavy shader: just a flat organic-blob disc
	// (a triangle fan whose rim uses the SAME `waterEdgeFactor` outline the wade check uses, so the visible water
	// matches where you can actually wade) with a baked deep→shallow vertex-colour gradient, lit by the scene. No
	// Gerstner waves, no reflections, no per-frame shader — calm, cheap, readable. (Replaced the old 64×64 wave grid
	// + big fragment shader, which was needless cost — especially once the world has many natural ponds.)
	import { untrack } from 'svelte';
	import { T } from '@threlte/core';
	import * as THREE from 'three';
	import { waterSeed, waterSurfaceY, waterEdgeFactor } from '$lib/water';
	import type { Zone, TerrainFeature } from '$lib/world';

	let { zone, terrain = [] }: { zone: Zone; sky?: string; terrain?: TerrainFeature[] } = $props();

	// each Water is keyed by zone id in Scene, so its zone never changes identity → read props once (untrack).
	const Z = untrack(() => ({ size: zone.size, id: zone.id, pos: zone.pos }));
	const waterLevel = untrack(() => waterSurfaceY(zone, terrain)); // shared with LakeFish so fish ride the surface

	// a flat BLOB disc: centre + a rim of SEG points at `size × waterEdgeFactor(angle)` (the organic shoreline). The
	// centre vertex is dark (deep), the rim lighter (shallow) → a baked radial gradient reads as depth with no shader.
	const geo = untrack(() => {
		const seed = waterSeed(Z.id);
		const SEG = 40;
		const deep = [0.04, 0.16, 0.3];
		const shallow = [0.16, 0.42, 0.52];
		const positions: number[] = [0, 0, 0];
		const colors: number[] = [...deep];
		for (let i = 0; i <= SEG; i++) {
			const a = (i / SEG) * Math.PI * 2;
			const r = Z.size * waterEdgeFactor(seed, a);
			positions.push(Math.cos(a) * r, 0, Math.sin(a) * r);
			colors.push(...shallow);
		}
		const idx: number[] = [];
		for (let i = 1; i <= SEG; i++) idx.push(0, i + 1, i); // fan; winding so the disc faces up
		const g = new THREE.BufferGeometry();
		g.setAttribute('position', new THREE.Float32BufferAttribute(positions, 3));
		g.setAttribute('color', new THREE.Float32BufferAttribute(colors, 3));
		g.setIndex(idx);
		g.computeVertexNormals();
		return g;
	});

	// lit standard material (a soft sun sheen reads as water) with the baked depth gradient via vertexColors.
	const mat = new THREE.MeshStandardMaterial({ vertexColors: true, roughness: 0.3, metalness: 0.1 });
	const mesh = new THREE.Mesh(geo, mat);
	mesh.position.set(Z.pos[0], waterLevel, Z.pos[2]);
	mesh.renderOrder = 1;
</script>

<T is={mesh} />
