import { describe, it, expect, beforeAll } from 'vitest';
import { fastForward, emptyWorld, type WorldObject } from './world';
import { math } from './math';

// fastForward is the WELCOME-BACK catch-up: advance a saved world toward carrying capacity by however long you were
// away (closed-form, Rust ff_targets). User report: "came back hours later, barely 27, didn't fast-forward." These
// guard that it actually MATERIALISES growth (and isn't a silent no-op when the wasm/away-window is fine).
describe('fastForward — welcome-back population catch-up', () => {
	beforeAll(async () => {
		await math.init(); // load the wasm math (same path +page awaits before calling fastForward)
	});

	const CRE = ['rabbit', 'cat', 'kangaroo', 'person', 'lion', 'dinosaur'];
	const creatures = (w: { objects: WorldObject[] }) => w.objects.filter((o) => CRE.includes(o.kind)).length;
	const colony = () => {
		const w = emptyWorld('t');
		w.objects.push({ id: 'h', kind: 'house', pos: [0, 0, 0] } as WorldObject);
		w.objects.push({ id: 'c', kind: 'cabin', pos: [8, 0, 0] } as WorldObject);
		w.objects.push({ id: 't', kind: 'tower', pos: [-8, 0, 0] } as WorldObject);
		for (let i = 0; i < 8; i++) w.objects.push({ id: 'p' + i, kind: 'person', pos: [i, 0, 5], gene: 1 } as WorldObject);
		for (let i = 0; i < 20; i++) w.objects.push({ id: 'r' + i, kind: 'rabbit', pos: [i, 0, -5], gene: 1 } as WorldObject);
		return w;
	};

	it('grows an undeveloped colony toward carrying capacity after hours away', () => {
		const w = colony();
		const before = creatures(w);
		const res = fastForward(w, 6 * 3600 * 1000, 'ff-', () => 0); // 6 hours away
		const after = creatures(w);
		// eslint-disable-next-line no-console
		console.log(`[ff] creatures ${before} → ${after} (+${res.creatures}), houses +${res.houses}`);
		expect(res.creatures).toBeGreaterThan(0); // the catch-up actually materialised new life
		expect(after).toBeGreaterThan(before);
	});

	it('grows each kind NEAR its existing cluster, not stranded in the empty gap between far-flung groups', () => {
		// a colony of PEOPLE at the origin + a wild RABBIT herd 400 m east, with a big empty gap between (the demo's
		// shape). The away-growth must fill out each group where it lives — not smear arrivals across the dead gap
		// (which made the return look empty at the colony). Guards the fix for the user's "didn't fast-forward".
		const w = emptyWorld('t');
		for (let i = 0; i < 6; i++) w.objects.push({ id: 'pc' + i, kind: 'person', pos: [i, 0, 0], gene: 1 } as WorldObject);
		for (let i = 0; i < 6; i++) w.objects.push({ id: 'rw' + i, kind: 'rabbit', pos: [400 + i, 0, 0], gene: 1 } as WorldObject);
		fastForward(w, 6 * 3600 * 1000, 'g-', () => 0);
		const isNew = (o: WorldObject, k: string) => o.id.startsWith('g-') && o.kind === k;
		const newPeople = w.objects.filter((o) => isNew(o, 'person'));
		const newRabbits = w.objects.filter((o) => isNew(o, 'rabbit'));
		expect(newPeople.length).toBeGreaterThan(0);
		expect(newRabbits.length).toBeGreaterThan(0);
		expect(newPeople.every((o) => o.pos[0] < 100)).toBe(true); // people grew AT the origin colony
		expect(newRabbits.every((o) => o.pos[0] > 300)).toBe(true); // rabbits grew AT the wild herd — none in the 100–300 gap
	});

	it('is a no-op for a blink away (<30 s) — nothing to advance', () => {
		const w = colony();
		expect(fastForward(w, 10_000, 'ff-', () => 0)).toEqual({ creatures: 0, houses: 0 });
	});
});
