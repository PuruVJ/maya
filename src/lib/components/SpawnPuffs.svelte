<script lang="ts">
	// Build DUST — a soft ring of dust kicked up at the base of an object the moment it's BUILT (a house
	// thuds in, a tree breaks through the turf). The counterpart to DustPuffs' footstep dust: that one
	// completes "the world reacts to your movement", this one completes "the world reacts to your building".
	// Self-contained: watches world.objects, dusts each newly-added solid object (creatures walk in, so they
	// don't kick build-dust). World-anchored + ground-tinted, same proven GPU-Points ring buffer as DustPuffs.
	// GATED past the initial world load (and share-link/undo restores) by a short settle delay, so loading a
	// world doesn't dust-storm — only builds you make after it's settled puff.
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { heightAt } from '$lib/terrain';
	import { kindDef, GROUND_COLOR } from '$lib/kinds';
	import { deletePoofs } from '$lib/buildFx';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();

	const CREATURES = new Set(['person', 'cat', 'lion', 'rabbit', 'kangaroo', 'dinosaur']); // walk in → no build-dust
	const N = 90; // ring buffer (a build can spawn several objects at once → a touch bigger than the footstep cloud)
	const LIFE = 0.72; // seconds a puff lives

	const posArr = new Float32Array(N * 3);
	const ageArr = new Float32Array(N).fill(1); // 1 = dead/invisible
	const velArr = new Float32Array(N * 2);
	for (let i = 0; i < N; i++) posArr[i * 3 + 1] = -9999; // park dead particles far below

	const geo = new THREE.BufferGeometry();
	geo.setAttribute('position', new THREE.BufferAttribute(posArr, 3));
	geo.setAttribute('aAge', new THREE.BufferAttribute(ageArr, 1));

	const mat = new THREE.ShaderMaterial({
		uniforms: { uColor: { value: new THREE.Color('#c9bfa6') } },
		transparent: true,
		depthWrite: false,
		vertexShader: /* glsl */ `
			attribute float aAge;
			varying float vAge;
			void main() {
				vAge = aAge;
				vec4 mv = modelViewMatrix * vec4(position, 1.0);
				gl_PointSize = clamp((0.3 + aAge * 1.0) * 280.0 / max(-mv.z, 1.0), 2.0, 110.0); // grows as it dissipates
				gl_Position = projectionMatrix * mv;
			}
		`,
		fragmentShader: /* glsl */ `
			precision mediump float;
			uniform vec3 uColor;
			varying float vAge;
			void main() {
				if (vAge >= 1.0) discard;
				float d = length(gl_PointCoord - 0.5);
				if (d > 0.5) discard;
				float soft = smoothstep(0.5, 0.12, d);
				float op = soft * (1.0 - vAge) * smoothstep(0.0, 0.18, vAge) * 0.5;
				if (op < 0.01) discard;
				gl_FragColor = vec4(uColor, op);
			}
		`
	});
	const points = new THREE.Points(geo, mat);
	points.frustumCulled = false;

	$effect(() => {
		// tint to the ground colour, lightened so it reads as a puff (snow → near-white, sand → pale tan)
		const g = new THREE.Color(GROUND_COLOR[world.ground] ?? GROUND_COLOR.grass);
		g.lerp(new THREE.Color(1, 1, 1), 0.4);
		mat.uniforms.uColor.value.copy(g);
	});

	let head = 0;
	function emitRing(ox: number, oz: number, rr: number) {
		const count = Math.min(11, 4 + Math.round(rr * 2)); // bigger footprint → more dust
		for (let k = 0; k < count; k++) {
			head = (head + 1) % N;
			const a = (k / count) * Math.PI * 2 + Math.random() * 0.6;
			const dist = rr * (0.55 + Math.random() * 0.5);
			const x = ox + Math.cos(a) * dist;
			const z = oz + Math.sin(a) * dist;
			posArr[head * 3] = x;
			posArr[head * 3 + 1] = heightAt(x, z, world.terrain) + 0.1;
			posArr[head * 3 + 2] = z;
			velArr[head * 2] = Math.cos(a) * (0.4 + Math.random() * 0.5); // billows outward from the impact
			velArr[head * 2 + 1] = Math.sin(a) * (0.4 + Math.random() * 0.5);
			ageArr[head] = 0.0001;
		}
	}

	// diff world.objects → dust each NEW solid object, but only once the world has settled (so a load/restore,
	// which makes every object "new" at once, doesn't dust-storm). The settle flag flips a beat after mount.
	const seen = new Set<string>();
	let settled = false;
	$effect(() => {
		const t = setTimeout(() => (settled = true), 1200);
		return () => clearTimeout(t);
	});
	$effect(() => {
		for (const o of world.objects) {
			if (seen.has(o.id)) continue;
			seen.add(o.id);
			if (!settled || CREATURES.has(o.kind)) continue;
			const sc = Math.max(o.scale?.[0] ?? 1, o.scale?.[2] ?? 1);
			emitRing(o.pos[0], o.pos[2], Math.max(0.5, kindDef(o.kind).r * sc));
		}
	});

	useTask((dt) => {
		// drain any tap-DELETE poofs queued by the edit tool → dust where the object stood as it vanishes
		for (const p of deletePoofs) emitRing(p.x, p.z, p.r);
		deletePoofs.length = 0;
		let any = false;
		for (let i = 0; i < N; i++) {
			if (ageArr[i] >= 1) continue;
			any = true;
			ageArr[i] += dt / LIFE;
			posArr[i * 3] += velArr[i * 2] * dt;
			posArr[i * 3 + 1] += dt * 0.4; // drift up as it billows and thins
			posArr[i * 3 + 2] += velArr[i * 2 + 1] * dt;
		}
		if (any) {
			geo.attributes.position.needsUpdate = true;
			geo.attributes.aAge.needsUpdate = true;
		}
	});
</script>

<T is={points} />
