<script lang="ts">
	// Maps the world `sky` enum onto Threlte's atmospheric <Sky> (day/sunset/fog) and <Stars>
	// (night/space). The starfield rides WITH the camera (centred on it every frame) so it feels
	// infinitely far — no translational parallax, and it never disappears as you roam off the origin.
	import { Sky, Stars } from '@threlte/extras';
	import { T, useTask, useThrelte } from '@threlte/core';
	import * as THREE from 'three';
	import Clouds from './Clouds.svelte';
	import Moon from './Moon.svelte';
	import ShootingStars from './ShootingStars.svelte';
	import Aurora from './Aurora.svelte';
	import { gpu } from '$lib/gpu.svelte'; // WebGPU migration: Sky/Stars/Clouds/Moon are shader-based → skip on ?webgpu

	let { sky, ground = 'grass' }: { sky: string; ground?: string } = $props();

	// <Sky> hijacks renderer.toneMappingExposure; night/space have none, so pin it (else ground blows white).
	const { renderer, camera } = useThrelte();
	const EXPOSURE: Record<string, number> = { day: 0.5, sunset: 0.5, fog: 0.55, night: 0.5, space: 0.5 };
	let starGroup = $state<THREE.Group>();

	useTask(() => {
		if (renderer) renderer.toneMappingExposure = EXPOSURE[sky] ?? 0.5;
		const cam = camera.current;
		if (starGroup && cam) starGroup.position.copy(cam.position); // keep stars centred on the camera
	});
</script>

<!-- whole sky is shader-based (Threlte Sky/Stars + our Clouds/Moon/ShootingStars/Aurora) → gated until ported to
     TSL; on ?webgpu the page's background colour shows through the transparent canvas instead -->
{#if gpu.webgpu}{:else if sky === 'day'}
	<Sky elevation={28} azimuth={150} turbidity={8} rayleigh={2} />
	<Clouds tint="#ffffff" opacity={0.7} cover={0.62} />
{:else if sky === 'sunset'}
	<Sky elevation={10} azimuth={95} turbidity={10} rayleigh={2.5} mieCoefficient={0.006} mieDirectionalG={0.8} />
	<Clouds tint="#ffc89a" opacity={0.72} cover={0.62} />
{:else if sky === 'fog'}
	<Sky elevation={12} azimuth={150} turbidity={20} rayleigh={1} />
	<Clouds tint="#dfe3e8" opacity={0.75} cover={0.8} />
{:else if sky === 'night'}
	<T.Group bind:ref={starGroup}>
		<Stars count={4000} radius={180} depth={70} speed={0.15} />
	</T.Group>
	<Moon dim={1} />
	<ShootingStars />
	{#if ground === 'snow'}<Aurora />{/if}<!-- a snowy winter night earns the northern lights -->

{:else}
	<!-- space -->
	<T.Group bind:ref={starGroup}>
		<Stars count={9000} radius={260} depth={120} factor={4} speed={0.25} />
	</T.Group>
	<Moon dim={0.85} />
	<ShootingStars />
{/if}
