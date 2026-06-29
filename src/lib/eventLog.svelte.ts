// A chat-like CHRONICLE of the SETTLEMENT's story — a human died, a house was raised, a well was dug, a new town took
// root. The Rust sim emits raw events in the snapshot ([code, kind, x, z]×n); sim.ts drains them here. We deliberately
// keep ONLY the human + structure beats (see FILTER below) so the feed reads as a settlement journal, not a wildlife
// ticker — a thriving world births/kills hundreds of rabbits a minute and that churn drowned the log (user: "full of
// rabbits, filter it to just humans + structures"). The live POPULATION churn still lives in the EcoStats chips; this
// is the milestone log. Player/LLM builds are logged too via `note()` (so "adding a structure automatically counts").
// Repeats coalesce (a birth boom reads "🐣 a person was born ×5") so even the kept events don't spam.

const KIND = ['rabbit', 'cat', 'kangaroo', 'person', 'lion', 'dinosaur'];
const PERSON = 3; // index of 'person' in KIND (Rust Kind order)
// Structure events carry NO creature kind → always kept. Everything else is a CREATURE event and is kept ONLY for
// people (a person born/died/mated), never for wildlife. This is THE filter that turns the rabbit ticker into a
// human/settlement chronicle.
const STRUCTURE_CODES = new Set([5, 7]); // 5 = EV_BUILD (a house), 7 = EV_WELL

// event code (matches crates/worldsim EV_*) → an icon + a line of text built from the creature kind
const FMT: Record<number, (k: string) => { icon: string; text: string }> = {
	1: (k) => ({ icon: '🩸', text: `a ${k} was caught` }), // EV_KILL
	2: (k) => ({ icon: '🥀', text: `a ${k} starved` }), // EV_STARVE
	3: (k) => ({ icon: '🕊️', text: `a ${k} died of old age` }), // EV_OLDAGE
	4: (k) => ({ icon: '🐣', text: `a ${k} was born` }), // EV_BIRTH
	5: () => ({ icon: '🏠', text: `a house was raised` }), // EV_BUILD
	6: (k) => ({ icon: '❤️', text: `a ${k} pair mated` }), // EV_CONCEIVE
	7: () => ({ icon: '⛲', text: `a well was dug` }), // EV_WELL
	8: (k) => ({ icon: '⚔️', text: `a ${k} was slain` }) // EV_SLAIN (combat: mobbed / a rival scrap / a slash)
};

export type LogEntry = { id: number; icon: string; text: string; n: number };

class EventLog {
	/** Recent events, OLDEST first → newest last (chat order). Capped; the view shows the tail. */
	entries = $state<LogEntry[]>([]);
	#id = 0;
	#cap = 50;

	/** Append one line, coalescing an immediate repeat into a ×N count. Shared by the sim drain + `note()`. */
	#push(icon: string, text: string): void {
		const last = this.entries[this.entries.length - 1];
		if (last && last.icon === icon && last.text === text) {
			last.n++; // same line again → bump the count instead of a new row ("…×3")
			return;
		}
		this.entries.push({ id: ++this.#id, icon, text, n: 1 });
		if (this.entries.length > this.#cap) this.entries.shift();
	}

	/** Record one SIM event (raw code + kind index). FILTERED to the human/settlement chronicle: structure events
	 *  always; creature events ONLY for people — so the wildlife churn (rabbits/cats/…) never reaches the feed. */
	add(code: number, kindIdx: number): void {
		const fmt = FMT[code];
		if (!fmt) return;
		if (!STRUCTURE_CODES.has(code) && kindIdx !== PERSON) return; // wildlife birth/death/… → not in the chronicle
		const { icon, text } = fmt(KIND[kindIdx] ?? 'creature');
		this.#push(icon, text);
	}

	/** Record a NON-sim event — a player/LLM build, a milestone — so "adding a structure automatically counts". */
	note(icon: string, text: string): void {
		this.#push(icon, text);
	}

	clear(): void {
		this.entries = [];
	}
}

/** The world's chronicle — sim.ts pushes filtered sim events, BuildBar pushes player builds, EventLog.svelte renders. */
export const eventLog = new EventLog();
