// UI edit-mode state, shared between the build bar (which renders the tool toggles) and the in-canvas
// click handler (EditController, which reads `tool`/`held`). Tiny reactive singleton.
export type Tool = 'none' | 'delete' | 'move';

class Editor {
	paletteOpen = $state(false);
	tool = $state('none' as Tool); // none = build/look · delete = tap to remove · move = tap-pick then tap-place
	held = $state(null as string | null); // object id currently picked up (move tool)
	ghost = $state(null as [number, number, number] | null); // live cursor ground point while carrying
	modelPickerOpen = $state(false);
}

export const editor = new Editor();
