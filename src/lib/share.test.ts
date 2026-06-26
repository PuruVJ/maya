import { describe, it, expect } from 'vitest';
import { encodeWorld, decodeWorld } from './share';
import { demoWorld, emptyWorld, type World } from './world';
import { heightAt } from './terrain';

// total living population = live objects + every dormant region's aggregated counts
const totalPop = (w: World) => w.objects.length + Object.values(w.regions ?? {}).reduce((s, a) => s + Object.values(a.counts).reduce((x, n) => x + n, 0), 0);

describe('world share link (encode/decode round-trip)', () => {
	it('round-trips the demo world: settings, object kinds & positions, scenery', async () => {
		const w = demoWorld();
		const back = await decodeWorld(await encodeWorld(w));
		expect(back.name).toBe(w.name);
		expect(back.ground).toBe(w.ground);
		expect(back.sky).toBe(w.sky);
		expect(back.objects.length).toBe(w.objects.length);
		expect(back.objects.map((o) => o.kind)).toEqual(w.objects.map((o) => o.kind));
		expect(back.zones.length).toBe(w.zones.length);
		expect(back.terrain.length).toBe(w.terrain.length);
		for (let i = 0; i < w.objects.length; i++) {
			expect(Math.abs(back.objects[i].pos[0] - w.objects[i].pos[0])).toBeLessThan(0.06);
			expect(Math.abs(back.objects[i].pos[2] - w.objects[i].pos[2])).toBeLessThan(0.06);
		}
	});

	it('re-grounds Y from terrain on decode (the dropped axis)', async () => {
		const w = emptyWorld('hilly');
		w.terrain.push({ center: [0, 0], radius: 20, height: 10, rough: 0 });
		w.objects.push({ id: 'o0', kind: 'house', pos: [0, heightAt(0, 0, w.terrain), 0] });
		const back = await decodeWorld(await encodeWorld(w));
		expect(back.objects[0].pos[1]).toBeCloseTo(heightAt(0, 0, back.terrain), 3);
		expect(back.objects[0].pos[1]).toBeGreaterThan(5); // sitting on the 10-high hill
	});

	it('preserves color & non-default scale, drops defaults', async () => {
		const w = emptyWorld('p');
		w.objects.push({ id: 'o0', kind: 'house', pos: [1, 0, 2], color: '#b22222', scale: [2, 2, 2] });
		w.objects.push({ id: 'o1', kind: 'tree', pos: [3, 0, 4] });
		const back = await decodeWorld(await encodeWorld(w));
		expect(back.objects[0].color).toBe('#b22222');
		expect(back.objects[0].scale).toEqual([2, 2, 2]);
		expect(back.objects[1].color).toBeUndefined();
		expect(back.objects[1].scale).toEqual([1, 1, 1]);
	});

	it('round-trips DORMANT region aggregates — the WHOLE population survives the link, not just the live near-set', async () => {
		// REGRESSION GUARD for the "2000 objects in the link → only ~500 spawn" bug: a streamed world keeps most of its
		// population in dormant region aggregates (enforceLiveBudget / streamRegions offload the far objects). encode/
		// decode used to drop `w.regions` entirely, silently losing everything beyond the ~LIVE_BUDGET live near-set.
		const w = emptyWorld('p');
		w.objects.push({ id: 'o0', kind: 'rabbit', pos: [1, 0, 1], gene: 1 }); // the live near-set (1 object)
		w.regions = {
			'6,6': { counts: { rabbit: 40, lion: 3 }, gene: 1.2, statics: [{ id: 'h0', kind: 'house', pos: [600, 0, 600] }], lastTick: 900 },
			'-4,2': { counts: { kangaroo: 25 }, gene: 1.1, statics: [], lastTick: 1200 }
		};
		const back = await decodeWorld(await encodeWorld(w));
		expect(back.regions).toBeDefined();
		expect(Object.keys(back.regions!).sort()).toEqual(['-4,2', '6,6']);
		expect(back.regions!['6,6'].counts).toEqual({ rabbit: 40, lion: 3 });
		expect(back.regions!['6,6'].gene).toBeCloseTo(1.2, 1);
		expect(back.regions!['6,6'].lastTick).toBe(900);
		expect(back.regions!['6,6'].statics.length).toBe(1);
		expect(back.regions!['6,6'].statics[0].kind).toBe('house');
		expect(back.regions!['-4,2'].counts).toEqual({ kangaroo: 25 });
		// TOTAL conserved: 1 live + (40+3) + 25 = 69. Pre-fix this collapsed to 1.
		expect(totalPop(back)).toBe(totalPop(w));
		expect(totalPop(back)).toBe(69);
	});

	it('un-streamed world (no regions) encodes regions as nothing — no bloat', async () => {
		const tok = await encodeWorld(demoWorld()); // demoWorld has no .regions
		const back = await decodeWorld(tok);
		expect(back.regions).toBeUndefined();
	});

	it('produces a compact, url-safe token', async () => {
		const tok = await encodeWorld(demoWorld());
		expect(tok).toMatch(/^[A-Za-z0-9_-]+$/);
		expect(tok.length).toBeLessThan(3000); // ~48 objects + a lake, gzipped
	});
});
