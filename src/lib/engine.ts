// Thin façade over the RUST engine (crates/worldsim/src/engine_bin.rs, via wasm). The LLM emits intent + a symbolic
// anchor; RUST resolves exact, collision-free geometry — the deterministic op→world layer. There is NO JS engine
// anymore: this module only (de)serializes across the wasm boundary. Behaviour + JS-parity are tested in the crate
// (`cargo test -p worldsim engine_bin::`) and in src/lib/engine.test.ts. Callers must have the wasm loaded
// (await math.init()) before applyOps — in the app +page's onMount awaits it; in node tests scenarios.ts top-level-awaits.
//
// THE BOUNDARY IS BINARY (no JSON) — the world + ops cross as parallel string vecs + a flat f64 SoA (mirroring the
// Rust decode_*/decode_ops), the new world rides back the same way. The world is packed minus `regions` (the one
// unbounded field) and minus per-object live-state (dead/asleep/genome/…): only the engine-owned fields cross, and
// `unpackInto` MERGES the result back onto the live objects BY ID, so those snapshot fields are preserved untouched.
import type { World, WorldObject, Player } from './world';
import { math, type PackedApply, type RawApplyResult } from './math';
import { derror } from './debug';

export type Op =
	| { op: 'add'; kind: string; count?: number; pos?: [number, number, number]; at?: string; dist?: number; scale?: [number, number, number]; color?: string; rot?: number; gene?: number }
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

// op-type → numeric tag (mirrors the engine_bin::decode_ops match). `note` / anything unknown → 10 (no-op).
const OP_TAG: Record<string, number> = { add: 0, scatter: 1, remove: 2, move: 3, paint: 4, setGround: 5, setSky: 6, addZone: 7, addPath: 8, setTerrain: 9, note: 10 };

// One op → its slot in the flat streams. `num` stride 19 (NaN = field absent → Rust applies the default), `strs`
// stride 11. Indices MUST match engine_bin::decode_ops:
//   num  [0]tag [1]count [2..5]pos [5]dist [6..9]scale [9]rot [10]size [11..14]fromPos [14..17]toPos [17]width [18]amp
//   strs [0]kind [1]at [2]id [3]color [4]material [5]shape [6]from [7]to [8]value [9]area [10]preset
function packOp(op: Op, num: Float64Array, strs: string[], i: number): void {
	const nb = i * 19;
	const sb = i * 11;
	num[nb] = OP_TAG[op.op] ?? 10;
	const pos = (off: number, p: [number, number, number]) => {
		num[nb + off] = p[0];
		num[nb + off + 1] = p[1];
		num[nb + off + 2] = p[2];
	};
	switch (op.op) {
		case 'add':
			if (op.count != null) num[nb + 1] = op.count;
			if (op.pos) pos(2, op.pos);
			if (op.dist != null) num[nb + 5] = op.dist;
			if (op.scale) pos(6, op.scale);
			if (op.rot != null) num[nb + 9] = op.rot;
			strs[sb] = op.kind;
			strs[sb + 1] = op.at ?? '';
			strs[sb + 3] = op.color ?? '';
			break;
		case 'scatter':
			if (op.count != null) num[nb + 1] = op.count;
			strs[sb] = op.kind;
			strs[sb + 9] = op.area;
			strs[sb + 3] = op.color ?? '';
			break;
		case 'remove':
			strs[sb + 2] = op.id;
			break;
		case 'move':
			if (op.pos) pos(2, op.pos);
			if (op.dist != null) num[nb + 5] = op.dist;
			strs[sb + 2] = op.id;
			strs[sb + 1] = op.at ?? '';
			break;
		case 'paint':
			strs[sb + 2] = op.id;
			strs[sb + 3] = op.color;
			break;
		case 'setGround':
			strs[sb + 8] = op.value;
			break;
		case 'setSky':
			break;
		case 'addZone':
			if (op.pos) pos(2, op.pos);
			if (op.size != null) num[nb + 10] = op.size;
			strs[sb + 4] = op.material;
			strs[sb + 5] = op.shape ?? '';
			strs[sb + 1] = op.at ?? '';
			break;
		case 'addPath':
			if (op.fromPos) pos(11, op.fromPos);
			if (op.toPos) pos(14, op.toPos);
			if (op.width != null) num[nb + 17] = op.width;
			strs[sb + 4] = op.material;
			strs[sb + 6] = op.from ?? '';
			strs[sb + 7] = op.to ?? '';
			break;
		case 'setTerrain':
			if (op.amplitude != null) num[nb + 18] = op.amplitude;
			strs[sb + 10] = op.preset;
			break;
		// note → tag only
	}
}

/** Pack `world` (minus regions + live-state) + `ops` + player into the flat arrays the wasm `apply_ops_bin` reads. */
function packApply(world: World, ops: Op[], player: Player): PackedApply {
	const objs = world.objects;
	const objIds: string[] = [];
	const objKinds: string[] = [];
	const objColors: string[] = [];
	const objNum = new Float64Array(objs.length * 9);
	for (let i = 0; i < objs.length; i++) {
		const o = objs[i];
		objIds.push(o.id);
		objKinds.push(o.kind);
		objColors.push(o.color ?? '');
		const p = o.pos;
		const s = o.scale ?? [1, 1, 1];
		const b = i * 9;
		objNum[b] = p[0];
		objNum[b + 1] = p[1];
		objNum[b + 2] = p[2];
		objNum[b + 3] = s[0];
		objNum[b + 4] = s[1];
		objNum[b + 5] = s[2];
		objNum[b + 6] = o.rot ?? 0;
		objNum[b + 7] = o.keep ? 1 : 0;
		objNum[b + 8] = o.gene ?? 0;
	}
	const zones = world.zones ?? [];
	const zoneIds: string[] = [];
	const zoneMaterials: string[] = [];
	const zoneShapes: string[] = [];
	const zoneNum = new Float64Array(zones.length * 4);
	for (let i = 0; i < zones.length; i++) {
		const z = zones[i];
		zoneIds.push(z.id);
		zoneMaterials.push(z.material);
		zoneShapes.push(z.shape ?? '');
		const b = i * 4;
		zoneNum[b] = z.pos[0];
		zoneNum[b + 1] = z.pos[1];
		zoneNum[b + 2] = z.pos[2];
		zoneNum[b + 3] = z.size;
	}
	const paths = world.paths ?? [];
	const pathIds: string[] = [];
	const pathMaterials: string[] = [];
	const pathNum = new Float64Array(paths.length * 7);
	for (let i = 0; i < paths.length; i++) {
		const pa = paths[i];
		pathIds.push(pa.id);
		pathMaterials.push(pa.material);
		const b = i * 7;
		pathNum[b] = pa.from[0];
		pathNum[b + 1] = pa.from[1];
		pathNum[b + 2] = pa.from[2];
		pathNum[b + 3] = pa.to[0];
		pathNum[b + 4] = pa.to[1];
		pathNum[b + 5] = pa.to[2];
		pathNum[b + 6] = pa.width;
	}
	const terrain = world.terrain ?? [];
	const terrainNum = new Float64Array(terrain.length * 5);
	for (let i = 0; i < terrain.length; i++) {
		const f = terrain[i];
		const b = i * 5;
		terrainNum[b] = f.center[0];
		terrainNum[b + 1] = f.center[1];
		terrainNum[b + 2] = f.radius;
		terrainNum[b + 3] = f.height;
		terrainNum[b + 4] = f.rough;
	}
	const opNum = new Float64Array(ops.length * 19).fill(NaN);
	const opStrs: string[] = new Array(ops.length * 11).fill('');
	for (let i = 0; i < ops.length; i++) packOp(ops[i], opNum, opStrs, i);
	return { objIds, objKinds, objColors, objNum, zoneIds, zoneMaterials, zoneShapes, zoneNum, pathIds, pathMaterials, pathNum, terrainNum, ground: world.ground ?? '', sky: world.sky ?? '', opNum, opStrs, px: player.pos[0], pz: player.pos[2], yaw: player.yaw };
}

/** Write the binary result back into `world` (engine-owned fields only). Objects MERGE by id — an existing object
 *  keeps every field (dead/asleep/genome/scale/…) and only its engine-changed fields (pos always; color when set;
 *  keep when flagged) are overlaid; new objects are built from the SoA; removed ids drop out. */
function unpackInto(world: World, r: RawApplyResult, out?: { conflicts: PlacementConflict[] }): World {
	const byId = new Map(world.objects.map((o) => [o.id, o]));
	const objs: WorldObject[] = [];
	for (let i = 0; i < r.objIds.length; i++) {
		const id = r.objIds[i];
		const b = i * 9;
		const pos: [number, number, number] = [r.objNum[b], r.objNum[b + 1], r.objNum[b + 2]];
		const base = byId.get(id);
		const o: WorldObject = base ? { ...base, pos } : { id, kind: r.objKinds[i], pos, scale: [r.objNum[b + 3], r.objNum[b + 4], r.objNum[b + 5]], rot: r.objNum[b + 6] };
		const col = r.objColors[i];
		if (col) o.color = col;
		if (r.objNum[b + 7]) o.keep = true;
		objs.push(o);
	}
	world.objects = objs;
	world.zones = r.zoneIds.map((id, i) => {
		const b = i * 4;
		return { id, material: r.zoneMaterials[i], shape: r.zoneShapes[i], pos: [r.zoneNum[b], r.zoneNum[b + 1], r.zoneNum[b + 2]] as [number, number, number], size: r.zoneNum[b + 3] };
	});
	world.paths = r.pathIds.map((id, i) => {
		const b = i * 7;
		return { id, material: r.pathMaterials[i], from: [r.pathNum[b], r.pathNum[b + 1], r.pathNum[b + 2]] as [number, number, number], to: [r.pathNum[b + 3], r.pathNum[b + 4], r.pathNum[b + 5]] as [number, number, number], width: r.pathNum[b + 6] };
	});
	world.terrain = [];
	for (let i = 0; i < r.terrainNum.length; i += 5) world.terrain.push({ center: [r.terrainNum[i], r.terrainNum[i + 1]], radius: r.terrainNum[i + 2], height: r.terrainNum[i + 3], rough: r.terrainNum[i + 4] });
	if (r.ground) world.ground = r.ground;
	if (r.sky) world.sky = r.sky;
	if (out) for (let i = 0; i < r.conflictLabels.length; i++) out.conflicts.push({ label: r.conflictLabels[i], blockers: r.conflictBlockers[i] ? r.conflictBlockers[i].split(',') : [] });
	return world;
}

/**
 * Apply `ops` to `world` for `player`, in place (the engine-owned fields — objects/zones/paths/terrain/ground/sky
 * — are replaced from the Rust result; every other object/world field is preserved). The actual op resolution
 * (anchors, collision-free placement, terrain/water, scatter, CRUD) runs in Rust (the binary engine). `out.conflicts`
 * collects big-footprint placement conflicts. Returns the same `world` for convenience.
 */
export function applyOps(world: World, ops: Op[], player: Player = { pos: [0, 0, 0], yaw: 0 }, out?: { conflicts: PlacementConflict[] }): World {
	const raw = math.applyOpsBin(packApply(world, ops, player));
	if (!raw) {
		derror('engine', 'apply_ops called before the wasm engine loaded — world unchanged', { ops });
		return world;
	}
	return unpackInto(world, raw, out);
}
