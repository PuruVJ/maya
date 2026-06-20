<script lang="ts">
	// On-screen touch controls for phones/tablets — a virtual joystick (bottom-left, analog move) and a
	// jump button (bottom-right). Shared worlds open from a link on mobile, so without this they're
	// unplayable (WASD-only). Writes into `touchInput`, which Player reads each frame. Look/camera is the
	// existing canvas drag (Player handles it) — these controls only own their own corners, so a drag on
	// the rest of the screen still rotates the view. Renders only on touch-primary devices.
	import { onMount } from 'svelte';
	import { touchInput, isTouchDevice } from '$lib/touchControls';

	let show = $state(false);
	onMount(() => {
		show = isTouchDevice();
	});

	const R = 56; // joystick travel radius (px)
	let stickId = -1; // active pointer id for the joystick
	let knobX = $state(0);
	let knobY = $state(0);
	let baseEl = $state<HTMLDivElement>();

	function stickStart(e: PointerEvent) {
		stickId = e.pointerId;
		(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
		stickMove(e);
	}
	function stickMove(e: PointerEvent) {
		if (e.pointerId !== stickId || !baseEl) return;
		const rect = baseEl.getBoundingClientRect();
		const cx = rect.left + rect.width / 2;
		const cy = rect.top + rect.height / 2;
		let dx = e.clientX - cx;
		let dy = e.clientY - cy;
		const len = Math.hypot(dx, dy);
		if (len > R) {
			dx = (dx / len) * R;
			dy = (dy / len) * R;
		}
		knobX = dx;
		knobY = dy;
		touchInput.moveX = dx / R; // right = +strafe
		touchInput.moveZ = -dy / R; // up = +forward
	}
	function stickEnd(e: PointerEvent) {
		if (e.pointerId !== stickId) return;
		stickId = -1;
		knobX = 0;
		knobY = 0;
		touchInput.moveX = 0;
		touchInput.moveZ = 0;
	}
</script>

{#if show}
	<!-- joystick (bottom-left) -->
	<div
		bind:this={baseEl}
		class="fixed bottom-8 left-8 z-20 h-32 w-32 touch-none rounded-full border border-white/30 bg-black/25 backdrop-blur"
		onpointerdown={stickStart}
		onpointermove={stickMove}
		onpointerup={stickEnd}
		onpointercancel={stickEnd}
		role="application"
		aria-label="Move joystick"
	>
		<div
			class="pointer-events-none absolute left-1/2 top-1/2 h-14 w-14 rounded-full bg-white/70 shadow-lg"
			style:transform="translate(calc(-50% + {knobX}px), calc(-50% + {knobY}px))"
		></div>
	</div>

	<!-- jump (bottom-right) -->
	<button
		class="fixed bottom-10 right-10 z-20 h-20 w-20 touch-none rounded-full border border-white/30 bg-black/30 text-sm font-bold text-white backdrop-blur active:bg-white/30"
		onpointerdown={(e) => {
			e.preventDefault();
			touchInput.jump = true;
		}}
		aria-label="Jump"
	>
		JUMP
	</button>
{/if}
