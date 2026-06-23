// REGION STREAMING (big-world.md §3) — the near/far sim split that lets the world be huge AND fast. Only regions
// NEAR the player hold INDIVIDUAL creatures (fully simulated by the Rust sim); far regions collapse to a cheap
// aggregate {counts, gene, lastTick} that costs nothing per tick. Crossing a region boundary SLEEPS the regions you
// left (creatures → counts) and WAKES the ones you enter (fast-forward the aggregate via the Rust closed-form
// `ff_targets`, then re-materialise individuals at seeded spots). Static structures (houses/trees/…) are the durable
// delta and are NEVER collapsed — only living creatures stream.
//
// Pure functions over the World (mutate objects + regions); Scene calls `streamRegions` whenever the player crosses
// a region cell. Determinism: positions come from the seeded hash RNG so the same region re-materialises consistently.
import type { World, WorldObject } from './world';
import { worldAreaScale } from './world';
import { math } from './math';
import { heightAt } from './terrain';
import { kindDef } from './kinds';
import { rand } from './rng';

const CREATURE_KINDS = new Set(['rabbit', 'cat', 'kangaroo', 'person', 'lion', 'dinosaur']);
const FF_KINDS = ['rabbit', 'cat', 'kangaroo', 'person', 'lion', 'dinosaur'] as const;

export const REGION_SIZE = 200; // metres per region tile
const ACTIVE_RING = 1; // regions within this Chebyshev ring of the player's stay LIVE (3×3 → ~600 m live span)
const TICK_HZ = 30; // sim ticks per second (matches the Rust clock DT = 1/30)

export const regionKey = (cx: number, cz: number): string => `${cx},${cz}`;
export const regionOf = (x: number, z: number): [number, number] => [Math.floor(x / REGION_SIZE), Math.floor(z / REGION_SIZE)];

/** Region keys that should be LIVE for a player at (px,pz) — its cell + the ACTIVE_RING around it. */
export function activeKeys(px: number, pz: number): Set<string> {
	const [pcx, pcz] = regionOf(px, pz);
	const set = new Set<string>();
	for (let dx = -ACTIVE_RING; dx <= ACTIVE_RING; dx++) for (let dz = -ACTIVE_RING; dz <= ACTIVE_RING; dz++) set.add(regionKey(pcx + dx, pcz + dz));
	return set;
}

/** SLEEP a region: tally its live creatures into the aggregate + drop those creature objects. Static objects stay. */
export function collapseRegion(world: World, key: string, tick: number): void {
	if (!world.regions) world.regions = {};
	const counts: Record<string, number> = {};
	let geneSum = 0;
	let geneN = 0;
	const statics: WorldObject[] = [];
	const keep: WorldObject[] = [];
	for (const o of world.objects) {
		const [cx, cz] = regionOf(o.pos[0], o.pos[2]);
		if (regionKey(cx, cz) !== key) {
			keep.push(o); // not this region → stays live
		} else if (CREATURE_KINDS.has(o.kind)) {
			counts[o.kind] = (counts[o.kind] ?? 0) + 1; // creature → lossy aggregate
			geneSum += o.gene ?? 1;
			geneN++;
		} else {
			statics.push(o); // STATIC structure → kept verbatim in the aggregate (durable), dropped from live objects
		}
	}
	if (geneN === 0 && statics.length === 0) return; // empty region → nothing to collapse
	world.objects = keep;
	const prev = world.regions[key]; // merge if it already had an aggregate (slept twice without waking)
	const merged: Record<string, number> = { ...(prev?.counts ?? {}) };
	for (const k in counts) merged[k] = (merged[k] ?? 0) + counts[k];
	world.regions[key] = {
		counts: merged,
		gene: Math.min(1.6, Math.max(0.6, geneN > 0 ? geneSum / geneN : (prev?.gene ?? 1))),
		statics: [...(prev?.statics ?? []), ...statics],
		lastTick: tick
	};
}

/** WAKE a region: fast-forward its aggregate to `tick` (Rust closed-form), materialise individuals at seeded spots,
 *  clear the aggregate. Returns how many creatures it materialised. No-op (returns 0) if there's no aggregate. */
export function wakeRegion(world: World, key: string, tick: number, idPrefix: string): number {
	const agg = world.regions?.[key];
	if (!agg) return 0;
	const [cx, cz] = key.split(',').map(Number);
	const dtSec = Math.max(0, (tick - agg.lastTick) / TICK_HZ);
	const c = agg.counts;
	const scale = worldAreaScale(world.objects);
	// fast-forward the dormant population toward carrying capacity (Rust). Wasm not loaded / no time → keep the counts.
	const adv = dtSec > 0 ? math.ffTargets(c.rabbit ?? 0, c.cat ?? 0, c.kangaroo ?? 0, c.person ?? 0, c.lion ?? 0, c.dinosaur ?? 0, scale, dtSec) : null;
	const final: Record<string, number> = {};
	if (adv) FF_KINDS.forEach((k, i) => (final[k] = adv[i]));
	else for (const k in c) final[k] = c[k];
	// EVOLVE the dormant region's vigor over the away span (Rust closed-form) — dormant regions evolve via the clock,
	// they don't freeze. Under predation the mean gene climbs; with no predators it holds.
	const gene = dtSec > 0 ? math.ffGene(agg.gene, c, dtSec) : agg.gene;
	// restore the durable STATIC structures verbatim (exact ids/positions — they're the persistent delta).
	// `?? []` tolerates aggregates persisted before the statics field existed (older saved worlds).
	for (const s of agg.statics ?? []) world.objects.push(s);
	let made = 0;
	for (const kind of FF_KINDS) {
		const want = final[kind] ?? 0;
		for (let i = 0; i < want; i++) {
			const sx = rand(cx * 73856093 + cz * 19349663, kind.charCodeAt(0), i, 1); // seeded → deterministic re-materialise
			const sz = rand(cx * 19349663 + cz * 83492791, kind.charCodeAt(0), i, 2);
			const x = (cx + sx) * REGION_SIZE;
			const z = (cz + sz) * REGION_SIZE;
			world.objects.push({ id: `${idPrefix}${made.toString(36)}`, kind, pos: [x, heightAt(x, z, world.terrain), z], gene, scale: [1, 1, 1] });
			made++;
		}
	}
	if (world.regions) delete world.regions[key];
	return made;
}

/** WORLD PULSE (user) — fast-forward EVERY dormant region's aggregate to `tick` WITHOUT waking it, so the far
 *  world keeps LIVING (populations relax toward carrying capacity + vigor evolves) instead of freezing until you
 *  visit. Pure closed-form (Rust ff_targets/ff_gene), O(1) per region → microseconds for dozens of regions, and it
 *  runs on the main thread between frames so it never blocks the worker's sim ticks. Scene calls it ~every 10 s. */
export function fastForwardDormant(world: World, tick: number): void {
	if (!world.regions) return;
	const scale = worldAreaScale(world.objects);
	for (const key in world.regions) {
		const agg = world.regions[key];
		const dtSec = (tick - agg.lastTick) / TICK_HZ;
		if (dtSec <= 0) continue;
		const c = agg.counts;
		const adv = math.ffTargets(c.rabbit ?? 0, c.cat ?? 0, c.kangaroo ?? 0, c.person ?? 0, c.lion ?? 0, c.dinosaur ?? 0, scale, dtSec);
		if (adv) {
			const next: Record<string, number> = {};
			FF_KINDS.forEach((k, i) => (next[k] = adv[i]));
			// evolve vigor over the span (BEFORE overwriting counts). CLAMP defensively to the gene band: ff_gene
			// should already, but a stale/mismatched main-thread wasm must never let agg.gene drift past 1.6 — that
			// was inflating the HUD "vigor" readout over time (and region-dependent, as dormant regions came/went).
			agg.gene = Math.min(1.6, Math.max(0.6, math.ffGene(agg.gene, c, dtSec)));
			agg.counts = next;
		}
		agg.lastTick = tick;
	}
}

/** Per-cell streaming step (call when the player crosses a region): WAKE regions that just entered the active set,
 *  SLEEP regions with live creatures that just left it. Returns counts for diagnostics (0/0 if nothing changed). */
export function streamRegions(world: World, px: number, pz: number, tick: number, idPrefix = 'rg'): { slept: number; woken: number } {
	const active = activeKeys(px, pz);
	let slept = 0;
	let woken = 0;
	if (world.regions) {
		for (const key of Object.keys(world.regions)) {
			if (active.has(key)) {
				wakeRegion(world, key, tick, `${idPrefix}${key.replace(',', '_')}-${tick.toString(36)}-`);
				woken++;
			}
		}
	}
	const live = new Set<string>();
	for (const o of world.objects) {
		const [cx, cz] = regionOf(o.pos[0], o.pos[2]);
		const k = regionKey(cx, cz);
		if (!active.has(k)) live.add(k); // ANY object (creature OR static) in a non-active region → that region sleeps
	}
	for (const k of live) {
		collapseRegion(world, k, tick);
		slept++;
	}
	return { slept, woken };
}
