import { describe, it, expect } from 'vitest';
import { encodeWorld, decodeWorld } from './share';
import { demoWorld, emptyWorld } from './world';
import { heightAt } from './terrain';

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

	it('produces a compact, url-safe token', async () => {
		const tok = await encodeWorld(demoWorld());
		expect(tok).toMatch(/^[A-Za-z0-9_-]+$/);
		expect(tok.length).toBeLessThan(3000); // ~48 objects + a lake, gzipped
	});
});
