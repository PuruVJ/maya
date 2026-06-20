// Canonical world-state types + builders. This is what gets gzip+base64'd into the URL.
import { applyOps, type Op } from './engine';

export interface WorldObject {
	id: string;
	kind: string;
	pos: [number, number, number];
	scale?: [number, number, number];
	color?: string;
	rot?: number;
	// live-state snapshot (animals): captured into the share link so a wandered/dead creature reopens that
	// way. `pos` already holds the live position at encode time; these flag its condition.
	dead?: boolean;
	asleep?: boolean;
}

export interface Zone {
	id: string;
	material: string;
	shape: string; // blob | rect | ring
	pos: [number, number, number];
	size: number;
}

export interface Path {
	id: string;
	material: string;
	from: [number, number, number];
	to: [number, number, number];
	width: number;
}

// A contained terrain bump (hill/mountain/dune patch). The world is flat outside all features.
export interface TerrainFeature {
	center: [number, number]; // x, z
	radius: number;
	height: number; // peak height (negative = a valley/depression)
	rough: number; // 0 = smooth mound, >0 = rolling ripple
}

export interface World {
	v: number;
	name: string;
	ground: string;
	sky: string;
	spawn: [number, number, number];
	objects: WorldObject[];
	zones: Zone[];
	paths: Path[];
	terrain: TerrainFeature[];
	/** Where the player was when the link was made (decoded from the URL) → reopen standing there. Not part
	 *  of the world proper; set only by share-link decode, read once by Player to place you. */
	start?: { x: number; z: number; yaw: number };
}

export interface Player {
	pos: [number, number, number];
	yaw: number;
}

export function emptyWorld(name = 'Untitled'): World {
	return {
		v: 1,
		name,
		ground: 'grass',
		sky: 'day',
		spawn: [0, 0, 0],
		objects: [],
		zones: [],
		paths: [],
		terrain: []
	};
}

// A populated scene to walk around in before the LLM is wired up. Also dogfoods the engine.
export function demoWorld(): World {
	const w = emptyWorld('Hello World');
	w.sky = 'night';
	const ops: Op[] = [
		{ op: 'add', kind: 'house', pos: [0, 0, -6] },
		{ op: 'add', kind: 'cabin', pos: [-11, 0, -13] },
		{ op: 'add', kind: 'tower', pos: [10, 0, -13] },
		{ op: 'add', kind: 'well', pos: [-5, 0, -3] },
		{ op: 'add', kind: 'lamp', pos: [2.5, 0, -2] },
		{ op: 'add', kind: 'lamp', pos: [-2.5, 0, -2] },
		{ op: 'scatter', kind: 'tree', count: 20, area: 'north' },
		{ op: 'scatter', kind: 'flower', count: 16, area: 'center' },
		{ op: 'scatter', kind: 'rock', count: 6, area: 'west' },
		{ op: 'addZone', material: 'water', shape: 'blob', at: 'east', size: 20 },
		// a LIVING world on load — wildlife + villagers (the game's core, absent from the old demo): a couple of
		// cats and rabbits wander the hamlet, a kangaroo hops by the lake (it'll come down to drink), two
		// villagers mill about, and a lone dinosaur roams the far treeline → the food chain emerges as you watch.
		{ op: 'scatter', kind: 'cat', count: 2, area: 'center' },
		{ op: 'scatter', kind: 'rabbit', count: 3, area: 'center' },
		{ op: 'add', kind: 'kangaroo', pos: [9, 0, 5] },
		{ op: 'add', kind: 'person', pos: [-4, 0, -5] },
		{ op: 'add', kind: 'person', pos: [5, 0, -8] },
		{ op: 'add', kind: 'dinosaur', pos: [-16, 0, -34] }
	];
	applyOps(w, ops, { pos: [0, 0, 6], yaw: 0 });
	return w;
}
