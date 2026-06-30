import { describe, it, expect, beforeAll } from 'vitest';
import { emptyWorld, type WorldObject } from './world';
import { catchUpAway } from './streaming';
import { math } from './math';

// `catchUpAway` is the away/jump CATCH-UP: it packs the world to the binary boundary, runs the closed-form advance in
// RUST (worldsim::catch_up — population relax + settlement founding + dormant spread, all under the unified town cap),
// then merges the result back. The DEEP behaviour is pinned by the Rust catchup tests (cargo); these JS tests guard the
// pack → call → unpack/merge round-trip (that the wasm boundary + the merge-by-id preserve the world correctly).
describe('catchUpAway — away/jump catch-up (Rust-backed)', () => {
	beforeAll(async () => {
		await math.init(); // the catch-up runs in wasm — same instance +page awaits before calling it
	});

	const CRE = ['rabbit', 'cat', 'kangaroo', 'person', 'lion', 'dinosaur'];
	const far = (o: WorldObject) => Math.hypot(o.pos[0], o.pos[2]) > 240;

	function colony() {
		const w = emptyWorld('t');
		for (let i = 0; i < 10; i++) w.objects.push({ id: 'h' + i, kind: 'house', pos: [(i % 5) * 8, 0, ((i / 5) | 0) * 8] } as WorldObject);
		for (let i = 0; i < 40; i++) w.objects.push({ id: 'p' + i, kind: 'person', pos: [(i % 8) * 2, 0, 20 + ((i / 8) | 0) * 2], gene: 1 } as WorldObject);
		for (let i = 0; i < 30; i++) w.objects.push({ id: 'r' + i, kind: 'rabbit', pos: [i, 0, -30], gene: 1 } as WorldObject);
		return w;
	}

	it('is a no-op for a blink away (<60 s)', () => {
		const w = colony();
		const n = w.objects.length;
		expect(catchUpAway(w, 10_000)).toEqual({ creatures: 0, houses: 0 });
		expect(w.objects.length).toBe(n);
	});

	it('grows the colony + founds DISTANT towns with residents over a day away', () => {
		const w = colony();
		const p0 = w.objects.filter((o) => o.kind === 'person').length;
		const res = catchUpAway(w, 24 * 3600 * 1000);
		const ppl = w.objects.filter((o) => o.kind === 'person');
		const farHomes = w.objects.filter((o) => (o.kind === 'house' || o.kind === 'cabin') && far(o)).length;
		// eslint-disable-next-line no-console
		console.log(`[catchup] people ${p0}→${ppl.length}, far homes ${farHomes}, far people ${ppl.filter(far).length}, +${res.houses} homes`);
		expect(ppl.length).toBeGreaterThan(p0 * 1.5); // grew past the carrying cap toward the breeding plateau
		expect(farHomes).toBeGreaterThan(0); // founded DISTANT towns
		expect(ppl.filter(far).length).toBeGreaterThan(0); // …with RESIDENTS, not ghost houses
		expect(res.houses).toBeGreaterThan(0);
	});

	it('MERGES by id — existing objects keep EVERY JS-only field (genome/pfam/ageFrac); only pos is overlaid', () => {
		// the binary boundary only carries pos/scale/rot/keep/gene; the merge-by-id must restore the rest from the live
		// object, or the away-catch-up would silently wipe genome/lineage/maturity (the live-state preservation contract).
		const w = emptyWorld('t');
		for (let i = 0; i < 6; i++) w.objects.push({ id: 'h' + i, kind: 'house', pos: [(i % 3) * 8, 0, ((i / 3) | 0) * 8] } as WorldObject);
		w.objects.push({ id: 'vip', kind: 'person', pos: [4, 0, 4], gene: 1.2, genome: [0.1, 0.2, 0.3, 0.4, 0.5], pfamA: 7, juvenile: true, ageFrac: 0.5 } as WorldObject);
		for (let i = 0; i < 24; i++) w.objects.push({ id: 'p' + i, kind: 'person', pos: [(i % 5) * 2, 0, 10], gene: 1 } as WorldObject);
		catchUpAway(w, 6 * 3600 * 1000);
		const vip = w.objects.find((o) => o.id === 'vip');
		expect(vip, 'the existing person survived the round-trip').toBeTruthy();
		expect(vip!.genome).toEqual([0.1, 0.2, 0.3, 0.4, 0.5]); // JS-only fields preserved through the merge
		expect(vip!.pfamA).toBe(7);
		expect(vip!.juvenile).toBe(true);
		expect(vip!.ageFrac).toBe(0.5);
	});

	it('develops + SPREADS the dormant far world (regions), not just the live slice', () => {
		const w = emptyWorld('t');
		const statics: WorldObject[] = [];
		for (let i = 0; i < 12; i++) statics.push({ id: `dh${i}`, kind: 'house', pos: [1000 + (i % 4) * 8, 0, 1000 + ((i / 4) | 0) * 8] } as WorldObject);
		w.regions = { '5,5': { counts: { person: 56 }, gene: 1, statics, lastTick: 0 } }; // region cell of (1000,1000) at REGION_SIZE 200
		const before = Object.keys(w.regions).length;
		catchUpAway(w, 24 * 3600 * 1000);
		const keys = Object.keys(w.regions!);
		expect(keys.length).toBeGreaterThan(before); // the full dormant town spread a satellite into a NEW region
		expect(CRE.length).toBe(6); // (kinds sanity — keeps the lint happy)
	});
});
