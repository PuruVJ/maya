// Shared touch input — written by TouchControls.svelte (the on-screen joystick + jump button) and read
// by Player.svelte every frame. A plain mutable object, NOT reactive state: it's touched 60×/s on the
// hot path, so making it $state would churn the reactive graph (same rule as the agent manager).
export const touchInput = {
	moveX: 0, // strafe: -1 (left) .. +1 (right)
	moveZ: 0, // forward: -1 (back) .. +1 (forward)
	jump: false // one-shot — Player consumes it and clears
};

/** True on touch-primary devices (phones/tablets) — gates whether the on-screen controls render. */
export function isTouchDevice(): boolean {
	return typeof matchMedia !== 'undefined' && matchMedia('(pointer: coarse)').matches;
}
