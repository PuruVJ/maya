// Thin façade over the RUST engine (crates/worldsim/src/engine.rs, via wasm). The LLM emits intent + a symbolic
// anchor; RUST resolves exact, collision-free geometry — the deterministic op→world layer. There is NO JS engine
// anymore: this module only (de)serializes across the wasm boundary. Behaviour + JS-parity are tested in the crate
// (`cargo test -p worldsim engine::`). Callers must have the wasm loaded (await initRustMath()) before applyOps —
// in the app +page's onMount awaits it before any build; in node tests scenarios.ts top-level-awaits it.
import type { World, Player } from './world';
import { rustApplyOps } from './rustMath';
import { derror } from './debug';

export type Op =
	| { op: 'add'; kind: string; count?: number; pos?: [number, number, number]; at?: string; dist?: number; scale?: [number, number, number]; color?: string; rot?: number }
	| { op: 'scatter'; kind: string; count: number; area: string; color?: string }
	| { op: 'remove'; id: string }
	| { op: 'move'; id: string; pos?: [number, number, number]; at?: string; dist?: number }
	| { op: 'paint'; id: string; color: string }
	| { op: 'setGround'; value: string }
	| { op: 'setSky'; value: string }
	// scenery ops — stored for the renderer to interpret later (not applied to objects yet)
	| { op: 'addZone'; material: string; shape: string; at?: string; pos?: [number, number, number]; size?: number }
	| { op: 'addPath'; material: string; from?: string; to?: string; fromPos?: [number, number, number]; toPos?: [number, number, number]; width?: number }
	| { op: 'setTerrain'; preset: string; amplitude?: number }
	// pure UI feedback — the model emits this to tell the user a limit; the engine ignores it
	| { op: 'note'; text: string };

// reported by applyOps when a big-footprint op (lake/terrain) couldn't find fully clear space
export interface PlacementConflict {
	label: string;
	blockers: string[]; // object ids sitting under the placed area
}

/**
 * Apply `ops` to `world` for `player`, in place (the engine-owned fields — objects/zones/paths/terrain/ground/sky
 * — are replaced from the Rust result; any other world fields are left untouched). The actual op resolution
 * (anchors, collision-free placement, terrain/water, scatter, CRUD) runs in Rust. `out.conflicts` collects
 * big-footprint placement conflicts. Returns the same `world` for convenience.
 */
export function applyOps(world: World, ops: Op[], player: Player = { pos: [0, 0, 0], yaw: 0 }, out?: { conflicts: PlacementConflict[] }): World {
	const res = rustApplyOps(JSON.stringify(world), JSON.stringify(ops), player.pos[0], player.pos[2], player.yaw);
	if (!res) {
		derror('engine', 'apply_ops called before the wasm engine loaded — world unchanged', { ops });
		return world;
	}
	const nw = res.world as Partial<World>;
	// write back only the engine-owned fields (unknown world fields the Rust DOM round-tripped stay as they are)
	if (nw.objects) world.objects = nw.objects;
	world.zones = nw.zones ?? [];
	world.paths = nw.paths ?? [];
	world.terrain = nw.terrain ?? [];
	if (typeof nw.ground === 'string') world.ground = nw.ground;
	if (typeof nw.sky === 'string') world.sky = nw.sky;
	if (out && Array.isArray(res.conflicts)) out.conflicts.push(...(res.conflicts as PlacementConflict[]));
	return world;
}
