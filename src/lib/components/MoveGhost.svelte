<script lang="ts">
	// A translucent cyan "hologram" of the object you're carrying in Move mode, following the cursor's
	// ground point (editor.ghost) so you see exactly where it'll land before you drop it.
	import { T } from '@threlte/core';
	import * as THREE from 'three';
	import { editor } from '$lib/editor.svelte';
	import { kindDef } from '$lib/kinds';
	import { partGeo } from '$lib/sharedAssets';
	import { heightAt } from '$lib/terrain';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();

	const ghostMat = new THREE.MeshStandardMaterial({
		color: '#5fd6ff',
		emissive: '#5fd6ff',
		emissiveIntensity: 0.55,
		transparent: true,
		opacity: 0.45,
		depthWrite: false,
		flatShading: true
	});

	const obj = $derived(editor.held ? world.objects.find((o) => o.id === editor.held) : undefined);
	const def = $derived(obj ? kindDef(obj.kind) : undefined);
	const gy = $derived(editor.ghost ? heightAt(editor.ghost[0], editor.ghost[2], world.terrain) : 0);
</script>

{#if obj && def && editor.ghost}
	<T.Group
		position={[editor.ghost[0], gy, editor.ghost[2]]}
		rotation={[0, ((obj.rot ?? 0) * Math.PI) / 180, 0]}
		scale={obj.scale ?? [1, 1, 1]}
	>
		{#each def.parts as part, i (i)}
			<T.Mesh position={part.pos} geometry={partGeo(part)} material={ghostMat} />
		{/each}
	</T.Group>
{/if}
