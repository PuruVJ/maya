// "MOTHER NATURE" — the world DIRECTOR. Every so often she stirs the pot with a WILDCARD: a predator pack roams
// in from the wastes, a migration sweeps through, a boom of new life. It keeps the ecosystem from settling into a
// flat steady state — there's always something arriving or upending the balance — and it's how rare apex species
// (dinosaurs) re-enter a world they'd gone extinct in. The spawns happen in Scene (which owns world.objects); this
// module just decides WHEN + WHAT and surfaces the announcement banner to the HUD.

export interface Wildcard {
	kind: string; // species to bring in
	count: number; // how many (a pack / a herd)
	gene: number; // vigour centre — packs arrive hardy
	banner: string; // HUD announcement
}

// the table of events Mother Nature can roll. Predators are rarer (they upend the balance harder).
const EVENTS: { weight: number; make: () => Wildcard }[] = [
	{ weight: 3, make: () => ({ kind: 'kangaroo', count: 5 + ((Math.random() * 3) | 0), gene: 1.15, banner: '🦘 A great migration sweeps across the plains' }) },
	{ weight: 3, make: () => ({ kind: 'rabbit', count: 6 + ((Math.random() * 4) | 0), gene: 1.1, banner: '🐇 A season of plenty — new life floods the meadows' }) },
	{ weight: 2, make: () => ({ kind: 'lion', count: 2 + ((Math.random() * 2) | 0), gene: 1.3, banner: '🦁 A lion pride prowls into the territory' }) },
	{ weight: 2, make: () => ({ kind: 'cat', count: 3 + ((Math.random() * 2) | 0), gene: 1.25, banner: '🐈 A band of wildcats slinks in from the hills' }) },
	{ weight: 1, make: () => ({ kind: 'dinosaur', count: 2 + ((Math.random() * 2) | 0), gene: 1.35, banner: '🦖 The ground trembles — a pack of dinosaurs thunders in from the wastes' }) }
];

class Nature {
	banner = $state(''); // current announcement (HUD reads this; clears itself after a beat)
	#token = 0;

	/** Roll a weighted wildcard event (or null on the rare empty roll). Scene calls this on the wildcard timer. */
	roll(): Wildcard | null {
		const total = EVENTS.reduce((s, e) => s + e.weight, 0);
		let r = Math.random() * total;
		for (const e of EVENTS) {
			r -= e.weight;
			if (r <= 0) return e.make();
		}
		return null;
	}

	announce(text: string): void {
		this.banner = text;
		const tk = ++this.#token;
		setTimeout(() => {
			if (this.#token === tk) this.banner = '';
		}, 7000);
	}
}

export const nature = new Nature();
