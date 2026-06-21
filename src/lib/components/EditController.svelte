<script lang="ts">
	// In-canvas direct manipulation. Lives inside <Canvas> for the live camera/scene/renderer.
	// Active only when a tool is selected; acts on a TAP (a click that didn't drag-look):
	//   delete → raycast the nearest object (userData.objectId) and remove it.
	//   move   → 1st tap picks an object up (editor.held); 2nd tap drops it where the ray meets the
	//            ground plane. Both go through the shared undo history.
	import { useThrelte } from '@threlte/core';
	import * as THREE from 'three';
	import { editor } from '$lib/editor.svelte';
	import { history } from '$lib/history.svelte';
	import { applyOps } from '$lib/engine';
	import { kindDef } from '$lib/kinds';
	import { deletePoofs } from '$lib/buildFx';
	import { playerState } from '$lib/playerState.svelte';
	import { curvedGroundXZ } from '$lib/curveWorld';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();

	const CREATURES = new Set(['person', 'cat', 'lion', 'rabbit', 'kangaroo', 'dinosaur']); // they vanish, no dust

	const { camera, scene, renderer } = useThrelte();
	const raycaster = new THREE.Raycaster();
	const ndc = new THREE.Vector2();

	let downX = 0;
	let downY = 0;
	let downT = 0;

	const isCanvas = (e: PointerEvent) => (e.target as HTMLElement)?.tagName === 'CANVAS';
	const player = () => ({ pos: playerState.pos, yaw: playerState.yaw });

	function pickObjectId(): string | null {
		for (const h of raycaster.intersectObjects(scene.children, true)) {
			let o: THREE.Object3D | null = h.object;
			while (o) {
				const id = o.userData?.objectId as string | undefined;
				if (id) return id;
				o = o.parent;
			}
		}
		return null;
	}

	function onDown(e: PointerEvent) {
		if (editor.tool === 'none' || !isCanvas(e)) return;
		downX = e.clientX;
		downY = e.clientY;
		downT = performance.now();
	}

	function onUp(e: PointerEvent) {
		if (editor.tool === 'none' || !isCanvas(e)) return;
		// a drag (look) or a long hold is NOT an edit — only a quick tap is
		if (Math.hypot(e.clientX - downX, e.clientY - downY) > 6 || performance.now() - downT > 450) return;
		const cam = camera.current;
		if (!cam) return;
		const rect = renderer.domElement.getBoundingClientRect();
		ndc.x = ((e.clientX - rect.left) / rect.width) * 2 - 1;
		ndc.y = -((e.clientY - rect.top) / rect.height) * 2 + 1;
		raycaster.setFromCamera(ndc, cam);

		if (editor.tool === 'delete') {
			const id = pickObjectId();
			if (id) {
				// kick up a dust poof where it stood (solid objects only) before it's gone — tactile delete
				const o = world.objects.find((ob) => ob.id === id);
				if (o && !CREATURES.has(o.kind)) {
					const sc = Math.max(o.scale?.[0] ?? 1, o.scale?.[2] ?? 1);
					deletePoofs.push({ x: o.pos[0], z: o.pos[2], r: Math.max(0.5, kindDef(o.kind).r * sc) });
				}
				history.push(world);
				applyOps(world, [{ op: 'remove', id }], player());
			}
			return;
		}

		// move tool — drop where the ray meets the CURVED ground (the world-fold rears the ground up; a flat
		// y=0 raycast overshot and dropped things far back, see curvedGroundXZ).
		if (!editor.held) {
			editor.held = pickObjectId(); // pick up (null if the tap missed → just try again)
			const g = editor.held ? curvedGroundXZ(raycaster.ray) : null;
			if (g) editor.ghost = [g.x, 0, g.z];
		} else {
			const g = curvedGroundXZ(raycaster.ray);
			if (g) {
				history.push(world);
				applyOps(world, [{ op: 'move', id: editor.held, pos: [g.x, 0, g.z] }], player());
			}
			editor.held = null; // drop
			editor.ghost = null;
		}
	}

	// while carrying, the ghost hologram follows the cursor's ground point (un-folds the curve so it sits under
	// the cursor, not far behind it)
	function onMove(e: PointerEvent) {
		if (editor.tool !== 'move' || !editor.held || !isCanvas(e)) return;
		const cam = camera.current;
		if (!cam) return;
		const rect = renderer.domElement.getBoundingClientRect();
		ndc.x = ((e.clientX - rect.left) / rect.width) * 2 - 1;
		ndc.y = -((e.clientY - rect.top) / rect.height) * 2 + 1;
		raycaster.setFromCamera(ndc, cam);
		const g = curvedGroundXZ(raycaster.ray);
		if (g) editor.ghost = [g.x, 0, g.z];
	}

	$effect(() => {
		window.addEventListener('pointerdown', onDown);
		window.addEventListener('pointerup', onUp);
		window.addEventListener('pointermove', onMove);
		return () => {
			window.removeEventListener('pointerdown', onDown);
			window.removeEventListener('pointerup', onUp);
			window.removeEventListener('pointermove', onMove);
		};
	});
</script>
