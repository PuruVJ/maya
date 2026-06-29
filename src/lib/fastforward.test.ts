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

	it('a settled town keeps DEVELOPING over a long absence — not stuck at its starting size', () => {
		// THE "came back hours later, the world was STUCK at similar numbers" bug. A town already past hamlet size must
		// keep growing houses AND people over a long away (the co-development spiral), not sit at a sparse fixed point.
		const w = emptyWorld('t');
		for (let i = 0; i < 6; i++) w.objects.push({ id: 'h' + i, kind: 'house', pos: [(i % 3) * 8, 0, ((i / 3) | 0) * 8], keep: true } as WorldObject);
		for (let i = 0; i < 40; i++) w.objects.push({ id: 'p' + i, kind: 'person', pos: [(i % 8) * 2, 0, 20 + ((i / 8) | 0) * 2], gene: 1 } as WorldObject);
		for (let i = 0; i < 30; i++) w.objects.push({ id: 'r' + i, kind: 'rabbit', pos: [i, 0, -30], gene: 1 } as WorldObject);
		const homes = (x: { objects: WorldObject[] }) => x.objects.filter((o) => o.kind === 'house' || o.kind === 'cabin').length;
		const ppl = (x: { objects: WorldObject[] }) => x.objects.filter((o) => o.kind === 'person').length;
		const h0 = homes(w);
		const p0 = ppl(w);
		fastForward(w, 12 * 3600 * 1000, 'dev-', () => 0); // 12 hours away
		// eslint-disable-next-line no-console
		console.log(`[ff-dev] houses ${h0}→${homes(w)}, people ${p0}→${ppl(w)}`);
		expect(homes(w)).toBeGreaterThan(h0 + 4); // the town BUILT OUT (homes co-grow with people — the spiral)
		expect(ppl(w)).toBeGreaterThan(p0 + 8); // …and its population climbed with the new homes (not stuck at ~40)
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

	it('SPREADS into new towns once a settlement fills up — not one fat blob (people↔houses spread)', () => {
		// A dense settlement: enough people that the housing target (~1 home / 13 people) far exceeds one town's house
		// cap, so the surplus must FOUND new towns ≥FOUND_GAP (240 m) out instead of cramming every home into the origin.
		const w = emptyWorld('t');
		for (let i = 0; i < 6; i++) w.objects.push({ id: 'b' + i, kind: 'house', pos: [(i % 3) * 8, 0, ((i / 3) | 0) * 8], gene: 1 } as WorldObject);
		for (let i = 0; i < 400; i++) w.objects.push({ id: 'p' + i, kind: 'person', pos: [(i % 20) - 10, 0, ((i / 20) | 0) - 10], gene: 1 } as WorldObject);
		const res = fastForward(w, 24 * 3600 * 1000, 'sp-', () => 0); // a day away → lots of housing demand
		expect(res.houses).toBeGreaterThan(0);
		const newHomes = w.objects.filter((o) => o.id.startsWith('sp-') && ['house', 'cabin', 'tower'].includes(o.kind));
		// at least one new home is founded a real distance (≥ ~half the found gap) from the origin town → a SECOND town
		const far = newHomes.filter((h) => Math.hypot(h.pos[0], h.pos[2]) > 120);
		expect(far.length, `spread: ${newHomes.length} new homes, ${far.length} of them >120 m out`).toBeGreaterThan(0);
	});

	it('is a no-op for a blink away (<30 s) — nothing to advance', () => {
		const w = colony();
		expect(fastForward(w, 10_000, 'ff-', () => 0)).toEqual({ creatures: 0, houses: 0 });
	});

	it('REFITS the wall after away-growth — no home left standing on a fence panel, wall encloses the grown town', () => {
		// the user came back to a town the catch-up had grown: a house ON a fence, and the wall not closed round the new
		// edge. away-growth must re-fit the perimeter, not leave a stale ring. Seed a colony WITH a small fence already.
		const w = colony();
		for (let i = 0; i < 4; i++) w.objects.push({ id: 'p' + (8 + i), kind: 'person', pos: [i, 0, 6], gene: 1 } as WorldObject); // enough people to build
		const res = fastForward(w, 12 * 3600 * 1000, 'tf-', (x, z) => 0); // 12 h away → city growth raises homes
		expect(res.houses).toBeGreaterThan(0); // the catch-up actually built
		const fences = w.objects.filter((o) => o.kind === 'fence');
		const homes = w.objects.filter((o) => ['house', 'cabin', 'manor'].includes(o.kind));
		expect(fences.length).toBeGreaterThan(8); // the wall was (re)fitted around the grown town
		// no HOME sits on top of a fence panel (the user's "house on a fence")
		for (const h of homes) {
			const onFence = fences.some((f) => Math.hypot(f.pos[0] - h.pos[0], f.pos[2] - h.pos[2]) < 2.0);
			expect(onFence, `home ${h.id} at ${h.pos} sits on a fence panel`).toBe(false);
		}
		// the wall RINGS the homes: every home is inside the fence's max radius from the home centroid
		const cx = homes.reduce((s, h) => s + h.pos[0], 0) / homes.length;
		const cz = homes.reduce((s, h) => s + h.pos[2], 0) / homes.length;
		const fenceR = Math.max(...fences.map((f) => Math.hypot(f.pos[0] - cx, f.pos[2] - cz)));
		const homeR = Math.max(...homes.map((h) => Math.hypot(h.pos[0] - cx, h.pos[2] - cz)));
		expect(fenceR).toBeGreaterThan(homeR); // the perimeter sits OUTSIDE the furthest home (encloses it)
	});
});
