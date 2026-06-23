// A chat-like feed of world events (a baby born, a rabbit caught, a well dug…). The Rust sim emits these in the
// snapshot's events buffer ([code, kind, x, z]×n); sim.ts drains them here, and EventLog.svelte renders the tail.
// Pure render/UI sugar — the events are diagnostic truth from the sim, just made human-readable. Repeats coalesce
// (a birth boom reads "🐣 a rabbit was born ×5") so a fast world doesn't spam.

const KIND = ['rabbit', 'cat', 'kangaroo', 'person', 'lion', 'dinosaur'];

// event code (matches crates/worldsim EV_*) → an icon + a line of text built from the creature kind
const FMT: Record<number, (k: string) => { icon: string; text: string }> = {
	1: (k) => ({ icon: '🩸', text: `a ${k} was caught` }), // EV_KILL
	2: (k) => ({ icon: '🥀', text: `a ${k} starved` }), // EV_STARVE
	3: (k) => ({ icon: '🕊️', text: `a ${k} died of old age` }), // EV_OLDAGE
	4: (k) => ({ icon: '🐣', text: `a ${k} was born` }), // EV_BIRTH
	5: () => ({ icon: '🏠', text: `a house was raised` }), // EV_BUILD
	6: (k) => ({ icon: '❤️', text: `a ${k} pair mated` }), // EV_CONCEIVE
	7: () => ({ icon: '⛲', text: `a well was dug` }) // EV_WELL
};

export type LogEntry = { id: number; icon: string; text: string; n: number };

class EventLog {
	/** Recent events, OLDEST first → newest last (chat order). Capped; the view shows the tail. */
	entries = $state<LogEntry[]>([]);
	#id = 0;
	#cap = 50;

	/** Record one sim event (raw code + kind index from the events buffer). Coalesces an immediate repeat. */
	add(code: number, kindIdx: number): void {
		const fmt = FMT[code];
		if (!fmt) return;
		const { icon, text } = fmt(KIND[kindIdx] ?? 'creature');
		const last = this.entries[this.entries.length - 1];
		if (last && last.icon === icon && last.text === text) {
			last.n++; // same event again → bump the count instead of a new line ("…×3")
			return;
		}
		this.entries.push({ id: ++this.#id, icon, text, n: 1 });
		if (this.entries.length > this.#cap) this.entries.shift();
	}

	clear(): void {
		this.entries = [];
	}
}

/** The world's event feed — sim.ts pushes, EventLog.svelte renders. */
export const eventLog = new EventLog();
