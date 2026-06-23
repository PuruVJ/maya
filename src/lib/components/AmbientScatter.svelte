<script lang="ts">
	// Ambient distant forests that FOLLOW the player (so they never end at the horizon), but are
	// world-stable: trees are hashed by absolute world cell, so the near field stays put as you move —
	// only the far ring (fogged) changes when you cross a coarse rebuild boundary, so no visible
	// popping nearby. Forest clumps, grounded on heightAt, spawn area kept clear. Two InstancedMeshes.
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { heightAt } from '$lib/terrain';
	import { playerState } from '$lib/playerState.svelte';
	import { forEachTreeNear, forEachBushNear, onPath } from '$lib/scatter';
	import { rustMathReady } from '$lib/rustMath';
	import { inWater } from '$lib/water';
	import { kindDef } from '$lib/kinds';
	import { leafColorHex, LEAF_GREENS } from '$lib/sharedAssets';
	import { wind } from '$lib/wind';
	import { applyWeather } from '$lib/weather';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();
	const CREATURES = new Set(['person', 'cat', 'lion', 'rabbit', 'kangaroo', 'dinosaur']);

	const MAX = 3000;
	const RADIUS = 250; // reaches the horizon
	const REBUILD = 35; // re-place only when the player crosses a 35 m cell → infrequent

	const MAXB = 1500; // bush instances
	const RADIUS_B = 150; // bushes are small → don't bother placing them to the far horizon
	const trunkGeo = new THREE.CylinderGeometry(0.18, 0.24, 1.4, 6).translate(0, 0.7, 0);
	const canopyGeo = new THREE.ConeGeometry(1.1, 2.6, 7).translate(0, 2.5, 0);
	// a low lumpy shrub (squashed low-poly ball, base at y≈0 so the sway lever bends the top)
	const bushGeo = new THREE.IcosahedronGeometry(0.7, 0).scale(1.1, 0.78, 1.1).translate(0, 0.5, 0);

	// player position (XZ), updated every frame → bushes you brush through bend away from you (see windMat `push`)
	const uPlayer = { value: new THREE.Vector2(9999, 9999) };

	// contact-shadow blobs under the NEAR trees — the ambient forest has castShadow=false (thousands of
	// instances would wreck the shadow pass), so without these it floats. A soft dark disc grounds each near
	// tree in ONE draw call (only out to BLOB_R so far/fogged trees don't get an un-fogged dark dot). NOTE the
	// instanceMatrix in the vertex shader — a raw ShaderMaterial on an InstancedMesh must apply it itself, or
	// every blob stacks at the world origin (the CreatureShadows/LampGlow bug just fixed).
	const MAX_BLOBS = 700;
	const BLOB_R = 70;
	const blobGeo = new THREE.CircleGeometry(1, 12).rotateX(-Math.PI / 2);
	const blobMat = new THREE.ShaderMaterial({
		transparent: true,
		depthWrite: false,
		vertexShader: /* glsl */ `varying vec2 vUv; void main(){ vUv = uv; gl_Position = projectionMatrix * modelViewMatrix * instanceMatrix * vec4(position, 1.0); }`,
		fragmentShader: /* glsl */ `varying vec2 vUv; void main(){ float r = length(vUv - 0.5) * 2.0; float a = smoothstep(1.0, 0.2, r) * 0.28; gl_FragColor = vec4(0.0, 0.0, 0.0, a); }`
	});

	// gentle wind sway, matching the grass so the forest feels alive (not dead-still next to waving grass).
	// Patches the vertex shader: bend by local height (higher = more), phased per-tree from its world base
	// (read from instanceMatrix) so trees don't sway in lockstep. Trunk + canopy share it → the tree bends
	// as one. Pure vertex displacement (no shadow cast → no depth-material mismatch). Uses the shared wind
	// clock (advanced once by Scene), the same one the placed Tree.svelte reads.
	function windMat(color: string, dapple = false, push = false): THREE.MeshStandardMaterial {
		const m = new THREE.MeshStandardMaterial({ color, flatShading: true });
		m.onBeforeCompile = (shader) => {
			shader.uniforms.uTime = wind.uTime;
			if (push) shader.uniforms.uPlayer = uPlayer;
			shader.vertexShader = shader.vertexShader
				.replace('#include <common>', '#include <common>\nuniform float uTime;' + (dapple ? '\nvarying vec3 vLP;' : '') + (push ? '\nuniform vec2 uPlayer;' : ''))
				.replace(
					'#include <begin_vertex>',
					/* glsl */ `#include <begin_vertex>
					${dapple ? 'vLP = position;' : ''}
					vec3 treeBase = vec3(instanceMatrix[3][0], instanceMatrix[3][1], instanceMatrix[3][2]);
					float phase = uTime * 0.9 + treeBase.x * 0.18 + treeBase.z * 0.15;
					float sway = sin(phase) + 0.4 * sin(phase * 2.1 + 1.3);
					float lever = max(position.y, 0.0) * 0.06; // higher verts bend more (canopy ≫ trunk)
					transformed.x += sway * lever;
					transformed.z += cos(phase * 0.8) * lever * 0.6;
					${
						push
							? /* glsl */ `
					// brush-through: a bush within reach bends its top AWAY from the player and presses down a little,
					// so you part the undergrowth as you walk (springs back instantly — recomputed each frame). The
					// base (position.y≈0) is unmoved → the shrub stays rooted. Only the player; cheap per-instance.
					vec2 toB = treeBase.xz - uPlayer;
					float pd = length(toB);
					float pf = 1.0 - smoothstep(0.0, 1.7, pd);
					if (pf > 0.0) {
						vec2 awy = toB / max(pd, 1e-3);
						float plev = max(position.y, 0.0);
						transformed.x += awy.x * pf * 0.55 * plev;
						transformed.z += awy.y * pf * 0.55 * plev;
						transformed.y -= pf * pf * 0.3 * plev;
					}`
							: ''
					}`
				);
			// FOLIAGE dapple (canopy + bushes): break the flat green into leaf clumps + a darker underside, so
			// the ambient forest reads as foliage rather than solid cones — matching the placed Tree.svelte look.
			if (dapple) {
				shader.fragmentShader = shader.fragmentShader
					.replace(
						'#include <common>',
						/* glsl */ `#include <common>
						varying vec3 vLP;
						float fh(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
						float fn(vec2 p){ vec2 i = floor(p), f = fract(p); float a = fh(i), b = fh(i + vec2(1.0, 0.0)), c = fh(i + vec2(0.0, 1.0)), d = fh(i + vec2(1.0, 1.0)); vec2 u = f * f * (3.0 - 2.0 * f); return mix(mix(a, b, u.x), mix(c, d, u.x), u.y); }`
					)
					.replace(
						'#include <color_fragment>',
						/* glsl */ `#include <color_fragment>
						float fol = fn(vLP.xz * 1.6 + vLP.y * 0.7);
						diffuseColor.rgb *= 0.78 + 0.36 * fol;                          // dappled leaf clumps
						diffuseColor.rgb *= 0.72 + 0.32 * smoothstep(-0.5, 2.6, vLP.y); // darker underside`
					);
			}
			// the ambient forest soaks in the rain + caps with snow like every placed surface, so a snow/rain
			// world is coherent at the backdrop too — not bare distant trees around snowy placed ones. Canopy/
			// bush (dapple): no glossy leaf sheen; trunk: a little wet-bark sheen. (Snow's world-up uses the raw
			// normal; trees are upright + only Y-rotated per instance, which preserves normal.y → cap is correct.)
			applyWeather(shader, dapple ? 0.2 : 0.26, dapple ? 0.0 : 0.4);
		};
		return m;
	}

	// canopy COLOUR VARIETY — a real forest is mixed greens with the odd autumn tree, not a monoculture. Each
	// canopy gets a per-tree colour (instanceColor, base = white) hashed by its WORLD CELL → world-stable, the
	// dapple shader multiplies through. Same SHARED palette as placed trees (leafColorHex) so they blend.
	const tmpLeaf = new THREE.Color(); // reused → no per-tree alloc in the rebuild

	const trunks = new THREE.InstancedMesh(trunkGeo, windMat('#6b4a2b'), MAX);
	const canopies = new THREE.InstancedMesh(canopyGeo, windMat('#ffffff', true), MAX); // white base → per-tree instanceColor
	const bushes = new THREE.InstancedMesh(bushGeo, windMat('#ffffff', true, true), MAXB); // white base → per-bush green; wind + dapple + brush-through
	const treeShadows = new THREE.InstancedMesh(blobGeo, blobMat, MAX_BLOBS);
	treeShadows.frustumCulled = false;
	treeShadows.renderOrder = -1; // draw just over the opaque ground, like the creature contact shadows
	treeShadows.count = 0;
	trunks.castShadow = canopies.castShadow = bushes.castShadow = false;
	trunks.frustumCulled = canopies.frustumCulled = bushes.frustumCulled = false;
	trunks.count = 0;
	canopies.count = 0;
	bushes.count = 0;
	const dummy = new THREE.Object3D();
	const blobDummy = new THREE.Object3D();

	let lastCx = NaN;
	let lastCz = NaN;
	let lastLen = -1;
	let lastZones = -1;
	let lastPaths = -1;
	let lastObjs = -1;

	useTask(() => {
		uPlayer.value.set(playerState.pos[0], playerState.pos[2]); // every frame → bushes part as you brush past (cheap)
		// the forest field lives in Rust now — until the wasm math instance has loaded, skip the rebuild WITHOUT
		// caching, so it retries each frame and the trees pop in the moment Rust is ready (not stuck empty).
		if (!rustMathReady()) return;
		const pcx = Math.round(playerState.pos[0] / REBUILD) * REBUILD;
		const pcz = Math.round(playerState.pos[2] / REBUILD) * REBUILD;
		const len = world.terrain.length;
		const zl = world.zones?.length ?? 0; // re-scatter when zones/paths/SOLID objects change (a new lake/road/house clears its trees)
		const pl = world.paths?.length ?? 0;
		// count only SOLID, FOREST-CLEARING objects (buildings/props) — NOT creatures, and NOT graves. Creatures
		// were already excluded (re-scattering 3000 trees on every birth/death was a 10-15ms hitch for no visual
		// change). GRAVES are added on EVERY death (tiny, cosmetic, don't meaningfully clear forest) → counting them
		// re-scattered the whole forest every few seconds in a lively world → a periodic frame hitch the adaptive
		// resolution turned into a visible flicker. Skip them here (a stray tree on a headstone is invisible anyway).
		let ol = 0;
		for (const o of world.objects) if (!CREATURES.has(o.kind) && o.kind !== 'grave') ol++;
		if (pcx === lastCx && pcz === lastCz && len === lastLen && zl === lastZones && pl === lastPaths && ol === lastObjs) return;
		lastCx = pcx;
		lastCz = pcz;
		lastLen = len;
		lastZones = zl;
		lastPaths = pl;
		lastObjs = ol;

		// placed-object footprints (buildings/props/lamps/placed trees) → no ambient tree clips through them.
		// No collision-side change needed: the player + animals already can't enter these footprints, so a culled
		// tree there is unreachable. Creatures are skipped (you don't clear forest around a wandering animal).
		// SPATIAL GRID: each candidate tree/bush checks only the solids in its OWN cell, not all of them — was
		// O(cells × objects) (a re-scatter near a big city = a multi-ms hitch); now O(cells + objects). Each solid
		// is inserted into every cell its bounding circle overlaps (robust even for big scaled buildings), so the
		// candidate's single-cell lookup catches it (it must lie in a cell the solid was inserted into).
		const SOLID_CELL = 16;
		const solidGrid = new Map<string, { x: number; z: number; r2: number }[]>();
		for (const o of world.objects) {
			if (CREATURES.has(o.kind)) continue;
			const rr = kindDef(o.kind).r * Math.max(o.scale?.[0] ?? 1, o.scale?.[2] ?? 1) + 1.0;
			const s = { x: o.pos[0], z: o.pos[2], r2: rr * rr };
			const i0 = Math.floor((s.x - rr) / SOLID_CELL);
			const i1 = Math.floor((s.x + rr) / SOLID_CELL);
			const j0 = Math.floor((s.z - rr) / SOLID_CELL);
			const j1 = Math.floor((s.z + rr) / SOLID_CELL);
			for (let i = i0; i <= i1; i++) {
				for (let j = j0; j <= j1; j++) {
					const k = i + ',' + j;
					let cell = solidGrid.get(k);
					if (!cell) solidGrid.set(k, (cell = []));
					cell.push(s);
				}
			}
		}
		const solidBlocked = (x: number, z: number) => {
			const cell = solidGrid.get(Math.floor(x / SOLID_CELL) + ',' + Math.floor(z / SOLID_CELL));
			if (cell) for (const s of cell) if ((x - s.x) ** 2 + (z - s.z) ** 2 < s.r2) return true;
			return false;
		};

		const px = playerState.pos[0];
		const pz = playerState.pos[2];
		const blobR2 = BLOB_R * BLOB_R;
		let n = 0;
		let nbs = 0;
		// trees come from the RUST forest field (forEachTreeNear, one wasm call, already bounded to RADIUS); JS only
		// culls the ones on its own lakes/roads/placed solids, then instances them. Collision (Scene) reads the SAME field.
		forEachTreeNear(px, pz, RADIUS, (t) => {
			if (n >= MAX) return;
			if (inWater(world.zones, t.x, t.z) || onPath(world.paths, t.x, t.z)) return; // no ambient tree in a lake or on a road
			if (solidBlocked(t.x, t.z)) return; // ...or clipping a placed building/prop
			const gy = heightAt(t.x, t.z, world.terrain);
			dummy.position.set(t.x, gy, t.z);
			dummy.rotation.set(0, t.rot, 0);
			dummy.scale.set(t.scale, t.scaleY, t.scale);
			dummy.updateMatrix();
			trunks.setMatrixAt(n, dummy.matrix);
			canopies.setMatrixAt(n, dummy.matrix);
			canopies.setColorAt(n, tmpLeaf.set(leafColorHex(t.colorHash)).convertSRGBToLinear()); // mixed greens + odd autumn
			n++;
			// ground the NEAR trees with a soft contact-shadow disc sized to the canopy footprint
			const td2 = (t.x - px) ** 2 + (t.z - pz) ** 2;
			if (td2 < blobR2 && nbs < MAX_BLOBS) {
				const br = 1.5 * t.scale;
				blobDummy.position.set(t.x, gy + 0.05, t.z);
				blobDummy.scale.set(br, 1, br);
				blobDummy.updateMatrix();
				treeShadows.setMatrixAt(nbs, blobDummy.matrix);
				nbs++;
			}
		});
		trunks.count = n;
		canopies.count = n;
		trunks.instanceMatrix.needsUpdate = true;
		canopies.instanceMatrix.needsUpdate = true;
		if (canopies.instanceColor) canopies.instanceColor.needsUpdate = true;
		treeShadows.count = nbs;
		treeShadows.instanceMatrix.needsUpdate = true;

		// BUSHES on their own coarser grid — sparser and only out to RADIUS_B (small → not worth the far horizon).
		// Same avoidance as trees (water / road / placed object), but NO collision (you brush through a shrub).
		let nb = 0;
		forEachBushNear(px, pz, RADIUS_B, (b) => {
			if (nb >= MAXB) return;
			if (inWater(world.zones, b.x, b.z) || onPath(world.paths, b.x, b.z)) return;
			if (solidBlocked(b.x, b.z)) return;
			dummy.position.set(b.x, heightAt(b.x, b.z, world.terrain), b.z);
			dummy.rotation.set(0, b.rot, 0);
			dummy.scale.set(b.scale, b.scale, b.scale);
			dummy.updateMatrix();
			bushes.setMatrixAt(nb, dummy.matrix);
			// shrub green varies too (greens only — evergreen, no autumn); Rust's bush colorHash already offsets it
			// from the canopy above, so the undergrowth isn't a clone field of the trees.
			bushes.setColorAt(nb, tmpLeaf.set(LEAF_GREENS[Math.floor(b.colorHash * LEAF_GREENS.length)]).convertSRGBToLinear());
			nb++;
		});
		bushes.count = nb;
		bushes.instanceMatrix.needsUpdate = true;
		if (bushes.instanceColor) bushes.instanceColor.needsUpdate = true;
	});
</script>

<T is={treeShadows} />
<T is={trunks} />
<T is={canopies} />
<T is={bushes} />
