// Shared "look + footprint" registry — the single source of truth for the placement engine
// (footprint radius `r`, height `h`) and the renderer (a composed low-poly `parts` model + the
// collider shape `col`). Swapping these procedural parts for CC0 GLB meshes later changes ONLY
// this file + the small Prop renderer.

export type PartGeo = 'box' | 'cyl' | 'cone' | 'pyramid' | 'sphere';

export interface Part {
	geo: PartGeo;
	args: number[]; // box:[w,h,d] · cyl/cone/pyramid:[radius,height] · sphere:[radius]
	pos: [number, number, number]; // centre offset from the model base (y up from the ground)
	color: string;
	emissive?: boolean;
}

export interface KindDef {
	r: number; // footprint radius (collision-free placement)
	h: number; // height (collision + terrain grounding)
	col: 'box' | 'cyl' | 'ball'; // collider shape
	color: string; // fallback / paint tint base
	parts: Part[]; // composed low-poly model
}

// palette
const TRUNK = '#6b4a2b';
const BARK = '#7c5230';
const LEAF = '#3f8f4a';
const PINE = '#2f6f3f';
const STONE = '#b3b3bd';
const ROOF = '#5a3b30';
const ROOFRED = '#9c4a3a';

export const KINDS: Record<string, KindDef> = {
	tree: {
		r: 0.8, h: 3, col: 'cyl', color: LEAF,
		parts: [
			{ geo: 'cyl', args: [0.16, 1.3], pos: [0, 0.65, 0], color: TRUNK },
			{ geo: 'sphere', args: [1.0], pos: [0, 2.2, 0], color: LEAF }
		]
	},
	pine: {
		r: 0.8, h: 4, col: 'cyl', color: PINE,
		parts: [
			{ geo: 'cyl', args: [0.14, 1.0], pos: [0, 0.5, 0], color: TRUNK },
			{ geo: 'cone', args: [1.0, 1.6], pos: [0, 1.6, 0], color: PINE },
			{ geo: 'cone', args: [0.75, 1.4], pos: [0, 2.5, 0], color: PINE },
			{ geo: 'cone', args: [0.5, 1.2], pos: [0, 3.3, 0], color: PINE }
		]
	},
	bush: {
		r: 0.6, h: 1, col: 'ball', color: LEAF,
		parts: [
			{ geo: 'sphere', args: [0.6], pos: [0, 0.5, 0], color: LEAF },
			{ geo: 'sphere', args: [0.45], pos: [0.35, 0.4, 0.1], color: '#459a52' },
			{ geo: 'sphere', args: [0.4], pos: [-0.3, 0.45, -0.1], color: '#3a8748' }
		]
	},
	flower: {
		r: 0.3, h: 0.6, col: 'cyl', color: '#e26d9c',
		parts: [
			{ geo: 'cyl', args: [0.05, 0.5], pos: [0, 0.25, 0], color: LEAF },
			{ geo: 'sphere', args: [0.18], pos: [0, 0.55, 0], color: '#e26d9c' }
		]
	},
	rock: {
		r: 0.9, h: 1, col: 'ball', color: '#8c8c92',
		parts: [
			{ geo: 'sphere', args: [0.85], pos: [0, 0.4, 0], color: '#8c8c92' },
			{ geo: 'sphere', args: [0.5], pos: [0.5, 0.3, 0.2], color: '#7e7e86' }
		]
	},
	house: {
		r: 3, h: 3, col: 'box', color: '#d2b48c',
		parts: [
			{ geo: 'box', args: [4.6, 2.2, 4.6], pos: [0, 1.1, 0], color: '#d2b48c' },
			{ geo: 'pyramid', args: [3.4, 1.6], pos: [0, 3.0, 0], color: ROOFRED },
			{ geo: 'box', args: [0.9, 1.4, 0.2], pos: [0, 0.7, 2.35], color: TRUNK }
		]
	},
	cabin: {
		r: 2.5, h: 3, col: 'box', color: '#a9763f',
		parts: [
			{ geo: 'box', args: [3.8, 2.0, 3.4], pos: [0, 1.0, 0], color: '#a9763f' },
			{ geo: 'pyramid', args: [3.0, 1.4], pos: [0, 2.7, 0], color: ROOF },
			{ geo: 'box', args: [0.8, 1.2, 0.2], pos: [0, 0.6, 1.75], color: BARK }
		]
	},
	tower: {
		r: 1.8, h: 8, col: 'cyl', color: STONE,
		parts: [
			{ geo: 'cyl', args: [1.4, 6.5], pos: [0, 3.25, 0], color: STONE },
			{ geo: 'cyl', args: [1.55, 0.6], pos: [0, 6.6, 0], color: '#9a9aa6' },
			{ geo: 'cone', args: [1.7, 2.0], pos: [0, 7.6, 0], color: ROOFRED }
		]
	},
	well: {
		r: 1.2, h: 1.5, col: 'cyl', color: '#9a9aa2',
		parts: [
			{ geo: 'cyl', args: [1.0, 1.0], pos: [0, 0.5, 0], color: '#9a9aa2' },
			{ geo: 'box', args: [0.12, 1.3, 0.12], pos: [0.8, 1.4, 0], color: BARK },
			{ geo: 'box', args: [0.12, 1.3, 0.12], pos: [-0.8, 1.4, 0], color: BARK },
			{ geo: 'pyramid', args: [1.3, 0.7], pos: [0, 2.3, 0], color: ROOF }
		]
	},
	lamp: {
		r: 0.4, h: 3, col: 'cyl', color: '#ffd27a',
		parts: [
			{ geo: 'cyl', args: [0.08, 2.6], pos: [0, 1.3, 0], color: '#3a3a40' },
			{ geo: 'sphere', args: [0.28], pos: [0, 2.7, 0], color: '#ffd27a', emissive: true }
		]
	},
	fence: {
		r: 0.6, h: 1, col: 'box', color: BARK,
		parts: [
			{ geo: 'box', args: [0.16, 1.0, 0.16], pos: [0.5, 0.5, 0], color: BARK },
			{ geo: 'box', args: [0.16, 1.0, 0.16], pos: [-0.5, 0.5, 0], color: BARK },
			{ geo: 'box', args: [1.4, 0.16, 0.1], pos: [0, 0.7, 0], color: '#8a5a2b' },
			{ geo: 'box', args: [1.4, 0.16, 0.1], pos: [0, 0.35, 0], color: '#8a5a2b' }
		]
	},
	bridge: {
		r: 2, h: 0.6, col: 'box', color: '#8a5a2b',
		parts: [
			{ geo: 'box', args: [3.6, 0.25, 1.6], pos: [0, 0.2, 0], color: '#8a5a2b' },
			{ geo: 'box', args: [3.6, 0.4, 0.12], pos: [0, 0.5, 0.74], color: BARK },
			{ geo: 'box', args: [3.6, 0.4, 0.12], pos: [0, 0.5, -0.74], color: BARK }
		]
	},
	person: {
		r: 0.5, h: 1.8, col: 'cyl', color: '#4a73c4',
		parts: [
			{ geo: 'box', args: [0.18, 0.7, 0.18], pos: [0.14, 0.35, 0], color: '#34507f' },
			{ geo: 'box', args: [0.18, 0.7, 0.18], pos: [-0.14, 0.35, 0], color: '#34507f' },
			{ geo: 'cyl', args: [0.26, 0.85], pos: [0, 1.05, 0], color: '#4a73c4' },
			{ geo: 'sphere', args: [0.24], pos: [0, 1.62, 0], color: '#e8b894' }
		]
	},
	// Footprint only — the live, animated cat is rendered by Cat.svelte (these parts are a fallback).
	cat: {
		r: 0.6, h: 0.7, col: 'box', color: '#e8924a',
		parts: [
			{ geo: 'box', args: [0.42, 0.34, 0.95], pos: [0, 0.32, 0], color: '#e8924a' },
			{ geo: 'sphere', args: [0.26], pos: [0, 0.46, 0.55], color: '#e8924a' },
			{ geo: 'cyl', args: [0.05, 0.55], pos: [0, 0.55, -0.55], color: '#d9823c' }
		]
	},
	// Footprints only — the live animals are rendered + animated by Critter.svelte (these are fallbacks).
	lion: {
		r: 0.85, h: 0.95, col: 'box', color: '#c79a4b',
		parts: [
			{ geo: 'box', args: [0.6, 0.5, 1.3], pos: [0, 0.45, 0], color: '#c79a4b' },
			{ geo: 'sphere', args: [0.36], pos: [0, 0.62, 0.78], color: '#a07a35' }
		]
	},
	rabbit: {
		r: 0.4, h: 0.55, col: 'box', color: '#e7e3da',
		parts: [
			{ geo: 'box', args: [0.26, 0.24, 0.5], pos: [0, 0.2, 0], color: '#e7e3da' },
			{ geo: 'sphere', args: [0.16], pos: [0, 0.32, 0.3], color: '#e7e3da' }
		]
	},
	kangaroo: {
		r: 0.6, h: 1.4, col: 'cyl', color: '#b07a4a',
		parts: [
			{ geo: 'box', args: [0.4, 0.8, 0.5], pos: [0, 0.6, 0], color: '#b07a4a' },
			{ geo: 'sphere', args: [0.22], pos: [0, 1.1, 0.2], color: '#b07a4a' }
		]
	},
	// Footprint only — the live dinosaur is rendered + animated by Critter.svelte (this is a fallback).
	dinosaur: {
		r: 1.4, h: 2.6, col: 'box', color: '#5f7d4a',
		parts: [
			{ geo: 'box', args: [0.9, 1.0, 2.4], pos: [0, 1.0, 0], color: '#5f7d4a' },
			{ geo: 'sphere', args: [0.5], pos: [0, 1.5, 1.3], color: '#5f7d4a' }
		]
	}
};

export const FALLBACK: KindDef = {
	r: 1, h: 2, col: 'box', color: '#aa66cc',
	parts: [{ geo: 'box', args: [1.6, 1.8, 1.6], pos: [0, 0.9, 0], color: '#aa66cc' }]
};
export const kindDef = (k: string): KindDef => KINDS[k] ?? FALLBACK;

// Ground material → flat colour (placeholder for textured/zoned ground later).
export const GROUND_COLOR: Record<string, string> = {
	grass: '#5a8f3c',
	sand: '#d8c388',
	stone: '#8a8a90',
	snow: '#eef2f5',
	water: '#3b6fb0',
	void: '#0b0d12'
};

// Sky enum → CSS background (canvas is transparent; robust + reactive, no three bg API).
export const SKY_BG: Record<string, string> = {
	day: 'linear-gradient(#9cc6ff, #dff0ff)',
	night: 'linear-gradient(#0a1430, #1b2a4a)',
	sunset: 'linear-gradient(#ff9a5a, #ffd6a0)',
	space: 'radial-gradient(circle at 50% 30%, #161a2b, #04050a)',
	fog: 'linear-gradient(#c8ccd2, #e6e9ee)'
};

// Zone/path material → colour. Some render translucent (water/ice/lava sheen).
export const ZONE_COLOR: Record<string, string> = {
	water: '#2f6fb0',
	path: '#9a9a9a',
	plaza: '#cabfa6',
	sand: '#d8c388',
	flowers: '#d96ba0',
	ice: '#cfe8f5',
	lava: '#d6391b',
	grass: '#5a8f3c'
};
export const ZONE_TRANSLUCENT = new Set(['water', 'ice', 'lava']);

// Distance fog per sky — kept light so you can see clear across the valley to the far side / city;
// just enough haze that the far folded-over edge still dissolves softly near the top of the fold.
export const SKY_FOG: Record<string, { color: string; density: number }> = {
	// Denser than before → a sense of dread + distance: far buildings dissolve into the murk, so a settlement
	// reads from afar only as its lamp-GLOWS (SettlementGlows.svelte) until you walk in and the blocks reveal.
	day: { color: '#cfe0f2', density: 0.0018 },
	night: { color: '#0c1830', density: 0.0025 },
	sunset: { color: '#f0b07a', density: 0.0021 },
	space: { color: '#05060c', density: 0.0019 },
	fog: { color: '#d6dade', density: 0.005 }
};
