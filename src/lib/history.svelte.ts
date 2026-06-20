import type { World } from './world';

// Unified undo/REDO history across ALL world edits — LLM builds, palette adds, click-deletes — so the
// undo/redo buttons reverse (and re-apply) whatever happened last. Stores plain (non-proxy) snapshots taken
// just before each mutation; a parallel redo stack remembers states you've undone past.
class History {
	#stack: World[] = [];
	#redoStack: World[] = [];
	#savedRedo: World[] | null = null; // redo future stashed during a push() — restored if that push is discard()ed
	canUndo = $state(false);
	canRedo = $state(false);

	/** Snapshot the world BEFORE a mutation. A new edit invalidates the redo future. */
	push(world: World): void {
		this.#stack.push($state.snapshot(world) as World);
		if (this.#stack.length > 40) this.#stack.shift();
		this.canUndo = true;
		// a fresh edit clears the redo future — but stash it first, so a discard() (a build that did nothing)
		// can put it back rather than silently dropping a still-valid redo.
		this.#savedRedo = this.#redoStack;
		this.#redoStack = [];
		this.canRedo = false;
	}

	/** Drop the most recent snapshot (e.g. a build that produced nothing) — the redo future survives it. */
	discard(): void {
		this.#stack.pop();
		this.canUndo = this.#stack.length > 0;
		if (this.#savedRedo) {
			this.#redoStack = this.#savedRedo;
			this.canRedo = this.#redoStack.length > 0;
		}
		this.#savedRedo = null;
	}

	/** Most recent snapshot without consuming it (re-roll restores this before retrying). */
	peek(): World | undefined {
		return this.#stack[this.#stack.length - 1];
	}

	/** Pop + restore the most recent snapshot; remember where we were so redo() can return here. */
	undo(world: World): boolean {
		const snap = this.#stack.pop();
		this.canUndo = this.#stack.length > 0;
		if (!snap) return false;
		this.#redoStack.push($state.snapshot(world) as World);
		this.canRedo = true;
		this.#savedRedo = null;
		this.restore(world, snap);
		return true;
	}

	/** Re-apply the most recently undone state; it becomes undoable again. */
	redo(world: World): boolean {
		const snap = this.#redoStack.pop();
		this.canRedo = this.#redoStack.length > 0;
		if (!snap) return false;
		this.#stack.push($state.snapshot(world) as World);
		this.canUndo = true;
		this.#savedRedo = null;
		this.restore(world, snap);
		return true;
	}

	/** Copy a snapshot's contents into the live (reactive) world without touching the stack. */
	restore(world: World, snap: World): void {
		const c = structuredClone(snap);
		world.name = c.name;
		world.ground = c.ground;
		world.sky = c.sky;
		world.objects = c.objects;
		world.zones = c.zones;
		world.paths = c.paths;
		world.terrain = c.terrain;
	}
}

export const history = new History();
