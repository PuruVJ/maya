<script lang="ts">
	// FOOTPRINTS — a fading trail the player presses into SOFT ground (snow / sand). Grass already springs
	// aside (the grass-trample shader); firm ground (plaza/stone) holds nothing — so prints only stamp on
	// snow & sand. One InstancedMesh of small flat oval decals (1 draw call), world-anchored in a ring buffer,
	// each oriented along the stride and fading over LIFE seconds via a per-instance age attribute. Stamps a
	// new print every STRIDE metres of ground travel, alternating left/right of the path. Completes the
	// "ground reacts to you" set alongside the grass trample.
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { playerState } from '$lib/playerState.svelte';
	import { heightAt } from '$lib/terrain';
	import { GROUND_COLOR } from '$lib/kinds';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();

	const N = 48; // ring-buffer size → ~N·STRIDE metres of trail before the oldest recycles
	const LIFE = 11; // seconds a print lingers before it has fully faded
	const STRIDE = 0.7; // ground distance between successive prints
	const FOOT = 0.13; // how far each print sits to the side of the path (alternating → a left/right gait)

	const ageArr = new Float32Array(N).fill(1); // 1 = dead/invisible (fragment discards)
	const geo = new THREE.PlaneGeometry(0.34, 0.5).rotateX(-Math.PI / 2); // lies flat; long axis = local +Z (stride dir)
	geo.setAttribute('aAge', new THREE.InstancedBufferAttribute(ageArr, 1));

	const mat = new THREE.ShaderMaterial({
		uniforms: { uColor: { value: new THREE.Color('#9aa6b8') } },
		transparent: true,
		depthWrite: false,
		vertexShader: /* glsl */ `
			attribute float aAge;
			varying float vAge;
			varying vec2 vUv;
			void main() {
				vAge = aAge;
				vUv = uv;
				gl_Position = projectionMatrix * modelViewMatrix * instanceMatrix * vec4(position, 1.0);
			}
		`,
		fragmentShader: /* glsl */ `
			precision mediump float;
			uniform vec3 uColor;
			varying float vAge;
			varying vec2 vUv;
			void main() {
				if (vAge >= 1.0) discard;
				// elongated oval (longer along the stride = v), soft edge
				float d = length((vUv - 0.5) * vec2(2.0, 1.5));
				float mask = smoothstep(1.0, 0.55, d);
				float op = mask * (1.0 - vAge) * smoothstep(0.0, 0.06, vAge) * 0.5; // fade in fast, ebb away over life
				if (op < 0.01) discard;
				gl_FragColor = vec4(uColor, op);
			}
		`
	});

	const prints = new THREE.InstancedMesh(geo, mat, N);
	prints.frustumCulled = false; // positions live in instance matrices we write; CPU bounds are meaningless
	prints.renderOrder = -1; // draw just over the opaque ground (like the contact shadows), not as an overlay
	const dummy = new THREE.Object3D();
	for (let i = 0; i < N; i++) {
		dummy.position.set(0, -9999, 0); // park dead prints far below (also guarded by aAge in the shader)
		dummy.updateMatrix();
		prints.setMatrixAt(i, dummy.matrix);
	}
	prints.instanceMatrix.needsUpdate = true;

	// print tint: a soft compressed-shadow of the ground (snow → cool blue-grey dimple, sand → darker tan scuff)
	$effect(() => {
		const g = new THREE.Color(GROUND_COLOR[world.ground] ?? GROUND_COLOR.grass);
		g.multiplyScalar(0.62); // darker than the ground → reads as a pressed-in dent
		mat.uniforms.uColor.value.copy(g);
	});

	let head = 0;
	let lastX = 0;
	let lastZ = 0;
	let inited = false;
	let side = 1;

	useTask((dt) => {
		const px = playerState.pos[0];
		const pz = playerState.pos[2];
		if (!inited) {
			lastX = px;
			lastZ = pz;
			inited = true;
		}
		// only SOFT ground holds prints; skip while wading (no prints under water)
		const soft = (world.ground === 'snow' || world.ground === 'sand') && !playerState.inWater;
		if (soft && playerState.grounded) {
			const dx = px - lastX;
			const dz = pz - lastZ;
			const d = Math.hypot(dx, dz);
			if (d >= STRIDE) {
				const ang = Math.atan2(dx, dz); // travel bearing → orient the print + find the across-path axis
				const rx = Math.cos(ang); // unit vector across the path (perpendicular to travel)
				const rz = -Math.sin(ang);
				const fx = px + rx * FOOT * side; // place the print to one side, alternating each step
				const fz = pz + rz * FOOT * side;
				side = -side;
				dummy.position.set(fx, heightAt(fx, fz, world.terrain) + 0.04, fz);
				dummy.rotation.set(0, ang, 0);
				dummy.updateMatrix();
				prints.setMatrixAt(head, dummy.matrix);
				prints.instanceMatrix.needsUpdate = true;
				ageArr[head] = 0.0001;
				head = (head + 1) % N;
				lastX = px;
				lastZ = pz;
			}
		} else {
			// keep the reference point with the player so re-entering soft ground doesn't dump a burst of prints
			lastX = px;
			lastZ = pz;
		}

		// age every live print toward death
		let dirty = false;
		for (let i = 0; i < N; i++) {
			if (ageArr[i] >= 1) continue;
			ageArr[i] = Math.min(1, ageArr[i] + dt / LIFE);
			dirty = true;
		}
		if (dirty) geo.attributes.aAge.needsUpdate = true;
	});
</script>

<T is={prints} />
