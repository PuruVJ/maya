export type MoveState = 'idle' | 'walking' | 'jumping' | 'falling';

/**
 * Player transform + a small movement state machine. Player.svelte writes it each frame;
 * the build bar reads pos/yaw to resolve "here"/"front" anchors against the live player.
 * (Uses `$state(... as T)` rather than `$state<T>()` — the generic form isn't reliably
 * transformed into reactive state in class fields.)
 */
export class PlayerState {
	pos = $state([0, 0, 6] as [number, number, number]);
	yaw = $state(0);
	grounded = $state(true);
	state = $state('idle' as MoveState);
	danger = $state(0); // 0..1 — how imminent a player-hunting predator is (drives the danger vignette)
	dangerBehind = $state(0); // 0..1 — the hunter is BEHIND you (out of view) → the vignette darkens with dread
	// wading in water → the water shader rings ripples around the player; hot-path field (read 60×/s),
	// deliberately NOT $state so writing it every frame doesn't churn the reactive graph
	inWater = false;
	// one-shot teleport request (e.g. the "go home" command sets a target); Player consumes + clears it next
	// frame. Plain field (polled each frame), not $state.
	teleportTo: [number, number, number] | null = null;

	place(pos: [number, number, number], yaw: number) {
		this.pos = pos;
		this.yaw = yaw;
	}

	// FSM: derive the movement state from this frame's facts
	tick(moving: boolean, grounded: boolean, vy: number) {
		this.grounded = grounded;
		this.state = !grounded ? (vy > 0.1 ? 'jumping' : 'falling') : moving ? 'walking' : 'idle';
	}
}

export const playerState = new PlayerState();
