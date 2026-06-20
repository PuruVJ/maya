<script lang="ts">
	// A street/garden lamp that actually responds to the time of day: its bulb is dim in daylight and
	// blazes warm at night, with an additive GLOW HALO that fades in after dark (cheap fake bloom, no
	// post-processing, no per-lamp real lights). Matches the building-window night glow so dusk scenes read
	// as a lit-up little world. Scene routes 'lamp' here; pole stays a plain dark post. Pop-in like Prop.
	import { untrack } from 'svelte';
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { kindDef } from '$lib/kinds';
	import { partGeo, litMat, PRIM } from '$lib/sharedAssets';
	import type { World, WorldObject } from '$lib/world';

	let { obj, world }: { obj: WorldObject; world: World } = $props();
	// snapshot static inputs; pos/rot read REACTIVELY in the markup so the move tool relocates the lamp
	const O = untrack(() => ({ color: obj.color, sx: obj.scale?.[0] ?? 1, sy: obj.scale?.[1] ?? 1, sz: obj.scale?.[2] ?? 1 }));
	const rot = $derived(((obj.rot ?? 0) * Math.PI) / 180);
	const def = kindDef('lamp');
	const pole = def.parts[0]; // dark post (cyl)
	const bulb = def.parts[1]; // glowing sphere
	const bulbColor = O.color ?? bulb.color; // initial bulb colour (kept in sync with paint in the $effect below)

	// own (uncached) bulb + halo materials so we can drive them by the sky without touching shared litMat
	const bulbMat = new THREE.MeshStandardMaterial({ color: bulbColor, emissive: bulbColor, flatShading: true });
	const haloMat = new THREE.MeshBasicMaterial({ color: '#ffdca0', transparent: true, opacity: 0, depthWrite: false, blending: THREE.AdditiveBlending });

	// warm LIGHT POOL cast on the ground beneath the lamp — a cheap additive radial disc (NO real point light),
	// so a lamp actually lights its patch of ground at night instead of just glowing in the dark. Only near
	// lamps mount (Scene's lazy reveal), so the additive overdraw is bounded → FPS-safe.
	const POOL_R = 4.5;
	const poolGeo = new THREE.CircleGeometry(1, 20).rotateX(-Math.PI / 2);
	const poolMat = new THREE.ShaderMaterial({
		uniforms: { uOpacity: { value: 0 }, uColor: { value: new THREE.Color('#ffca7a') } },
		transparent: true,
		depthWrite: false,
		blending: THREE.AdditiveBlending,
		vertexShader: /* glsl */ `varying vec2 vUv; void main(){ vUv = uv; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }`,
		fragmentShader: /* glsl */ `uniform float uOpacity; uniform vec3 uColor; varying vec2 vUv; void main(){ float d = distance(vUv, vec2(0.5)) * 2.0; float a = smoothstep(1.0, 0.12, d); a *= a; gl_FragColor = vec4(uColor, a * uOpacity); }`
	});
	poolMat.toneMapped = false;

	// 0 (full day) → 1 (full night). bulb glows brighter and the halo fades in as it gets darker.
	const NIGHT: Record<string, number> = { day: 0.08, sunset: 0.5, fog: 0.3, night: 1, space: 1 };
	$effect(() => {
		const n = NIGHT[world.sky] ?? 0.08;
		bulbMat.emissiveIntensity = 0.25 + n * 1.6;
		haloMat.opacity = n * 0.55;
		poolMat.uniforms.uOpacity.value = n * 0.5; // warm ground pool fades in with the dark
		const c = obj.color ?? bulb.color; // react to paint → recolour the bulb (glow follows its colour)
		bulbMat.color.set(c);
		bulbMat.emissive.set(c);
	});

	// pop-in (easeOutBack), same as Prop
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
		<T.Mesh geometry={partGeo(pole)} position={pole.pos} material={litMat(pole.color)} castShadow receiveShadow />
		<T.Mesh geometry={partGeo(bulb)} position={bulb.pos} material={bulbMat} castShadow />
		<!-- additive glow halo (a big soft sphere around the bulb; opacity tracks night) -->
		<T.Mesh geometry={PRIM.sphere} position={bulb.pos} scale={2.4} material={haloMat} />
	</T.Group>
	<!-- warm light pool cast on the ground (additive radial disc, night-gated; not part of the pop-in) -->
	<T.Mesh geometry={poolGeo} position={[0, 0.04, 0]} scale={[POOL_R * O.sx, 1, POOL_R * O.sz]} material={poolMat} />
</T.Group>
