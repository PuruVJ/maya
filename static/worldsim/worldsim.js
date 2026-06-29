/* @ts-self-types="./worldsim.d.ts" */

/**
 * The `apply_ops_bin` result: the new world as the SAME parallel arrays JS packs in, plus conflicts. Read once via
 * the getters (each clones — this is a cold per-edit call, not a hot path).
 */
export class ApplyResult {
    static __wrap(ptr) {
        const obj = Object.create(ApplyResult.prototype);
        obj.__wbg_ptr = ptr;
        ApplyResultFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        ApplyResultFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_applyresult_free(ptr, 0);
    }
    /**
     * @returns {string[]}
     */
    get conflict_blockers() {
        const ret = wasm.applyresult_conflict_blockers(this.__wbg_ptr);
        var v1 = getArrayJsValueFromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * @returns {string[]}
     */
    get conflict_labels() {
        const ret = wasm.applyresult_conflict_labels(this.__wbg_ptr);
        var v1 = getArrayJsValueFromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * @returns {string}
     */
    get ground() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.applyresult_ground(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {string[]}
     */
    get obj_colors() {
        const ret = wasm.applyresult_obj_colors(this.__wbg_ptr);
        var v1 = getArrayJsValueFromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * @returns {string[]}
     */
    get obj_ids() {
        const ret = wasm.applyresult_obj_ids(this.__wbg_ptr);
        var v1 = getArrayJsValueFromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * @returns {string[]}
     */
    get obj_kinds() {
        const ret = wasm.applyresult_obj_kinds(this.__wbg_ptr);
        var v1 = getArrayJsValueFromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * @returns {Float64Array}
     */
    get obj_num() {
        const ret = wasm.applyresult_obj_num(this.__wbg_ptr);
        var v1 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
        return v1;
    }
    /**
     * @returns {string[]}
     */
    get path_ids() {
        const ret = wasm.applyresult_path_ids(this.__wbg_ptr);
        var v1 = getArrayJsValueFromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * @returns {string[]}
     */
    get path_materials() {
        const ret = wasm.applyresult_path_materials(this.__wbg_ptr);
        var v1 = getArrayJsValueFromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * @returns {Float64Array}
     */
    get path_num() {
        const ret = wasm.applyresult_path_num(this.__wbg_ptr);
        var v1 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
        return v1;
    }
    /**
     * @returns {string}
     */
    get sky() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.applyresult_sky(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {Float64Array}
     */
    get terrain_num() {
        const ret = wasm.applyresult_terrain_num(this.__wbg_ptr);
        var v1 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
        return v1;
    }
    /**
     * @returns {string[]}
     */
    get zone_ids() {
        const ret = wasm.applyresult_zone_ids(this.__wbg_ptr);
        var v1 = getArrayJsValueFromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * @returns {string[]}
     */
    get zone_materials() {
        const ret = wasm.applyresult_zone_materials(this.__wbg_ptr);
        var v1 = getArrayJsValueFromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * @returns {Float64Array}
     */
    get zone_num() {
        const ret = wasm.applyresult_zone_num(this.__wbg_ptr);
        var v1 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
        return v1;
    }
    /**
     * @returns {string[]}
     */
    get zone_shapes() {
        const ret = wasm.applyresult_zone_shapes(this.__wbg_ptr);
        var v1 = getArrayJsValueFromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
}
if (Symbol.dispose) ApplyResult.prototype[Symbol.dispose] = ApplyResult.prototype.free;

export class Sim {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        SimFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_sim_free(ptr, 0);
    }
    /**
     * Mean age (fraction of lifespan, 0..1) per Kind [rabbit,cat,kangaroo,person,lion,dino]; -1 = none alive.
     * @returns {Float32Array}
     */
    age_means() {
        const ret = wasm.sim_age_means(this.__wbg_ptr);
        var v1 = getArrayF32FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * @returns {number}
     */
    ages_ptr() {
        const ret = wasm.sim_ages_ptr(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {number}
     */
    behaviors_ptr() {
        const ret = wasm.sim_behaviors_ptr(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Newborns from the last step(): count (each is [kc, x, z, gene, motherFam, fatherFam, g0..g4] — 11 floats).
     * @returns {number}
     */
    birth_count() {
        const ret = wasm.sim_birth_count(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Pointer to the flat births buffer [kc, x, z, gene, motherFam, fatherFam, g0..g4, …] (len = birth_count()*11).
     * @returns {number}
     */
    births_ptr() {
        const ret = wasm.sim_births_ptr(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * House-build requests from the last step(): count (each is [x, z]).
     * @returns {number}
     */
    build_count() {
        const ret = wasm.sim_build_count(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Pointer to the flat builds buffer [x, z, …] (length = build_count()*2) for a zero-copy read.
     * @returns {number}
     */
    builds_ptr() {
        const ret = wasm.sim_builds_ptr(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {number}
     */
    count() {
        const ret = wasm.sim_count(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * 0..1 — how imminent a player-hunting predator is (eased; drives the UI danger vignette).
     * @returns {number}
     */
    danger() {
        const ret = wasm.sim_danger(this.__wbg_ptr);
        return ret;
    }
    /**
     * Remove agent `i` (its world-object was deleted / world cleared) → it goes inert, no longer a ghost.
     * @param {number} i
     */
    despawn(i) {
        wasm.sim_despawn(this.__wbg_ptr, i);
    }
    /**
     * Telemetry events from the last step(): count (each is [code, kind, x, z]).
     * @returns {number}
     */
    event_count() {
        const ret = wasm.sim_event_count(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Pointer to the flat events buffer [code, kind, x, z, …] (length = event_count()*4) for a zero-copy read.
     * @returns {number}
     */
    events_ptr() {
        const ret = wasm.sim_events_ptr(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {number}
     */
    flags_ptr() {
        const ret = wasm.sim_flags_ptr(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {number}
     */
    headings_ptr() {
        const ret = wasm.sim_headings_ptr(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {number}
     */
    healths_ptr() {
        const ret = wasm.sim_healths_ptr(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * The cooldown JS should stamp on a newborn.
     * @returns {number}
     */
    juvenile_cd() {
        const ret = wasm.sim_juvenile_cd(this.__wbg_ptr);
        return ret;
    }
    constructor() {
        const ret = wasm.sim_new();
        this.__wbg_ptr = ret;
        SimFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * @returns {number}
     */
    progress_ptr() {
        const ret = wasm.sim_progress_ptr(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Restore a saved agent's exact age (life fraction 0..1) — reload keeps adults adult, not seeded-young.
     * @param {number} i
     * @param {number} frac
     */
    set_age(i, frac) {
        wasm.sim_set_age(this.__wbg_ptr, i, frac);
    }
    /**
     * DROUGHT multiplier on thirst (1 = normal). The director/LLM sets this for a drought event; it stacks on
     * the always-on wet↔dry season cycle. Clamped 0.5‥3.0 internally.
     * @param {number} a
     */
    set_aridity(a) {
        wasm.sim_set_aridity(this.__wbg_ptr, a);
    }
    /**
     * Stamp a newborn (by index) with a maturation breed-cooldown so it can't breed until it grows up.
     * @param {number} i
     * @param {number} cd
     */
    set_breed_cooldown(i, cd) {
        wasm.sim_set_breed_cooldown(this.__wbg_ptr, i, cd);
    }
    /**
     * Mark a spawned agent (by index) as the player's pet → it follows you and won't flee you.
     * @param {number} i
     */
    set_companion(i) {
        wasm.sim_set_companion(this.__wbg_ptr, i);
    }
    /**
     * Replace the lake-fish lure points from a flat [x0,z0,x1,z1,…] buffer.
     * @param {Float64Array} xz
     */
    set_fish(xz) {
        const ptr0 = passArrayF64ToWasm0(xz, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        wasm.sim_set_fish(this.__wbg_ptr, ptr0, len0);
    }
    /**
     * Apply a bred baby's inherited vigor gene (by index) — scales its speed (genetics/evolution).
     * @param {number} i
     * @param {number} gene
     */
    set_gene(i, gene) {
        wasm.sim_set_gene(this.__wbg_ptr, i, gene);
    }
    /**
     * Apply a bred baby's inherited behaviour GENOME (5 utility weights from the births buffer) → emergent
     * strategies evolve across generations.
     * @param {number} i
     * @param {number} food
     * @param {number} safety
     * @param {number} social
     * @param {number} rest
     * @param {number} industry
     */
    set_genome(i, food, safety, social, rest, industry) {
        wasm.sim_set_genome(this.__wbg_ptr, i, food, safety, social, rest, industry);
    }
    /**
     * Stamp a newborn (by index) with its PARENT lineage ids (mother's fam, father's fam) from the births buffer,
     * so the kinship check refuses a future parent/child/sibling pairing (incest avoidance, all kinds).
     * @param {number} i
     * @param {number} pfam_a
     * @param {number} pfam_b
     */
    set_lineage(i, pfam_a, pfam_b) {
        wasm.sim_set_lineage(this.__wbg_ptr, i, pfam_a, pfam_b);
    }
    /**
     * @param {number} n
     */
    set_night(n) {
        wasm.sim_set_night(this.__wbg_ptr, n);
    }
    /**
     * Replace the solid obstacles from a packed [x,z,r,hx,hz,cos,sin] per obstacle (7 f64s each); a CIRCLE
     * is signalled by `hx = NaN`, else it's an oriented box. Agents are pushed out + slide along surfaces.
     * @param {Float64Array} flat
     */
    set_obstacles(flat) {
        const ptr0 = passArrayF64ToWasm0(flat, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        wasm.sim_set_obstacles(this.__wbg_ptr, ptr0, len0);
    }
    /**
     * @param {number} x
     * @param {number} z
     */
    set_player(x, z) {
        wasm.sim_set_player(this.__wbg_ptr, x, z);
    }
    /**
     * Toggle player immunity (1 = no predator hunts/menaces you, danger stays 0 · 0 = you're fair game).
     * @param {number} immune
     */
    set_player_immune(immune) {
        wasm.sim_set_player_immune(this.__wbg_ptr, immune);
    }
    /**
     * @param {number} s
     */
    set_pop_scale(s) {
        wasm.sim_set_pop_scale(this.__wbg_ptr, s);
    }
    /**
     * Replace the REFUGE points (house centres) a threatened woman/child flees toward, flat [x0,z0,x1,z1,…].
     * @param {Float64Array} xz
     */
    set_refuges(xz) {
        const ptr0 = passArrayF64ToWasm0(xz, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        wasm.sim_set_refuges(this.__wbg_ptr, ptr0, len0);
    }
    /**
     * Per-kind breeding vitality from the JS "Mother Nature" director (6 floats, by Kind index).
     * @param {Float64Array} v
     */
    set_vitality(v) {
        const ptr0 = passArrayF64ToWasm0(v, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        wasm.sim_set_vitality(this.__wbg_ptr, ptr0, len0);
    }
    /**
     * Replace the DRINKABLE water sources (thirst) from a flat [x0,z0,r0,x1,z1,r1,…] buffer (pond centre+radius).
     * @param {Float64Array} xzr
     */
    set_water(xzr) {
        const ptr0 = passArrayF64ToWasm0(xzr, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        wasm.sim_set_water(this.__wbg_ptr, ptr0, len0);
    }
    /**
     * Spawn an agent from a kind-code (0 rabbit·1 cat·2 kangaroo·3 person·4 lion·5 dinosaur) + a stable
     * per-individual `seed_id` (its traits/speed key off this). Returns its index = its read-back slot.
     * @param {number} x
     * @param {number} z
     * @param {number} kind_code
     * @param {number} radius
     * @param {number} seed_id
     * @returns {number}
     */
    spawn(x, z, kind_code, radius, seed_id) {
        const ret = wasm.sim_spawn(this.__wbg_ptr, x, z, kind_code, radius, seed_id);
        return ret >>> 0;
    }
    /**
     * Spawn into a stable read-back slot recycled by the worker proxy's free-list.
     * @param {number} slot
     * @param {number} x
     * @param {number} z
     * @param {number} kind_code
     * @param {number} radius
     * @param {number} seed_id
     * @returns {number}
     */
    spawn_at(slot, x, z, kind_code, radius, seed_id) {
        const ret = wasm.sim_spawn_at(this.__wbg_ptr, slot, x, z, kind_code, radius, seed_id);
        return ret >>> 0;
    }
    /**
     * Advance by real elapsed seconds (the clock emits N fixed-DT ticks), then refresh the read-back.
     * @param {number} real_dt
     */
    step(real_dt) {
        wasm.sim_step(this.__wbg_ptr, real_dt);
    }
    /**
     * Well-dig requests from the last step(): count (each is [x, z]).
     * @returns {number}
     */
    well_count() {
        const ret = wasm.sim_well_count(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Pointer to the flat wells buffer [x, z, …] (length = well_count()*2) for a zero-copy read.
     * @returns {number}
     */
    wells_ptr() {
        const ret = wasm.sim_wells_ptr(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {number}
     */
    xs_ptr() {
        const ret = wasm.sim_xs_ptr(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {number}
     */
    zs_ptr() {
        const ret = wasm.sim_zs_ptr(this.__wbg_ptr);
        return ret >>> 0;
    }
}
if (Symbol.dispose) Sim.prototype[Symbol.dispose] = Sim.prototype.free;

export class WorldGen {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        WorldGenFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_worldgen_free(ptr, 0);
    }
    /**
     * @param {Float64Array} reqs
     * @param {Float64Array} zones
     * @returns {Float64Array}
     */
    build(reqs, zones) {
        const ptr0 = passArrayF64ToWasm0(reqs, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passArrayF64ToWasm0(zones, wasm.__wbindgen_malloc);
        const len1 = WASM_VECTOR_LEN;
        const ret = wasm.worldgen_build(this.__wbg_ptr, ptr0, len0, ptr1, len1);
        var v3 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
        return v3;
    }
    /**
     * CITY generator (binary) — reads buildings from the (seeded) store, water from `zones`, and the removable old
     * spokes/plaza from `removables` (`[tag,x,z]×n`; a returned REMOVE slot maps back to a path/plaza id JS-side).
     * @param {Float64Array} zones
     * @param {Float64Array} removables
     * @param {number} px
     * @param {number} pz
     * @param {number} yaw
     * @returns {Float64Array}
     */
    city(zones, removables, px, pz, yaw) {
        const ptr0 = passArrayF64ToWasm0(zones, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passArrayF64ToWasm0(removables, wasm.__wbindgen_malloc);
        const len1 = WASM_VECTOR_LEN;
        const ret = wasm.worldgen_city(this.__wbg_ptr, ptr0, len0, ptr1, len1, px, pz, yaw);
        var v3 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
        return v3;
    }
    /**
     * DEMO GALLERY (binary) — Rust owns the whole multi-town layout (spacing/grid/sizes), packed as `[numSites,
     * numPaths, numObjects, <sites: cx,cz,sizeCode>, <paths×4>, <objects×7>]`. JS just materialises it.
     * @returns {Float64Array}
     */
    demo_gallery() {
        const ret = wasm.worldgen_demo_gallery(this.__wbg_ptr);
        var v1 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
        return v1;
    }
    /**
     * @param {Uint8Array} buf
     */
    deserialize(buf) {
        const ptr0 = passArray8ToWasm0(buf, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        wasm.worldgen_deserialize(this.__wbg_ptr, ptr0, len0);
    }
    /**
     * FOREST generator (binary) — reads trees from the (seeded) store, water from `zones`. Returns the GEN op stream.
     * @param {Float64Array} zones
     * @param {number} px
     * @param {number} pz
     * @param {number} yaw
     * @returns {Float64Array}
     */
    forest(zones, px, pz, yaw) {
        const ptr0 = passArrayF64ToWasm0(zones, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.worldgen_forest(this.__wbg_ptr, ptr0, len0, px, pz, yaw);
        var v2 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
        return v2;
    }
    /**
     * @param {number} dx
     * @param {number} dz
     * @param {Float64Array} zones
     * @returns {Float64Array}
     */
    grave(dx, dz, zones) {
        const ptr0 = passArrayF64ToWasm0(zones, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.worldgen_grave(this.__wbg_ptr, dx, dz, ptr0, len0);
        var v2 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
        return v2;
    }
    /**
     * DORMANT settlement growth (self-sustaining world): grow a FAR cluster's homes via a throwaway store (does
     * NOT touch the live `self.store`). `houses` = the cluster's `[x,z]×n`; returns up to `want` new build ops.
     * @param {Float64Array} houses
     * @param {number} want
     * @param {Float64Array} zones
     * @param {number} seed
     * @returns {Float64Array}
     */
    grow_dormant(houses, want, zones, seed) {
        const ptr0 = passArrayF64ToWasm0(houses, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passArrayF64ToWasm0(zones, wasm.__wbindgen_malloc);
        const len1 = WASM_VECTOR_LEN;
        const ret = wasm.worldgen_grow_dormant(this.__wbg_ptr, ptr0, len0, want, ptr1, len1, seed);
        var v3 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
        return v3;
    }
    /**
     * IMMIGRATION decision (binary) — `counts` = `[n,geneSum]×5` (FLOORS order rabbit,kangaroo,person,cat,lion).
     * Returns a flat `[floorIdx,x,z,gene]×n` add-creature stream (no store needed; JS maps floorIdx → kind).
     * @param {Float64Array} counts
     * @param {number} px
     * @param {number} pz
     * @param {number} global_avg
     * @param {number} seed
     * @returns {Float64Array}
     */
    immigration(counts, px, pz, global_avg, seed) {
        const ptr0 = passArrayF64ToWasm0(counts, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.worldgen_immigration(this.__wbg_ptr, ptr0, len0, px, pz, global_avg, seed);
        var v2 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
        return v2;
    }
    /**
     * LAKE generator (binary) — `zones` = water zones `[px,pz,size,seed]×n`. Returns the GEN op stream (stride 10);
     * a REMOVE references its target zone by slot (its index in `zones`), which JS maps back to the zone id.
     * @param {Float64Array} zones
     * @param {number} px
     * @param {number} pz
     * @param {number} yaw
     * @returns {Float64Array}
     */
    lake(zones, px, pz, yaw) {
        const ptr0 = passArrayF64ToWasm0(zones, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.worldgen_lake(this.__wbg_ptr, ptr0, len0, px, pz, yaw);
        var v2 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
        return v2;
    }
    /**
     * @returns {number}
     */
    len() {
        const ret = wasm.worldgen_len(this.__wbg_ptr);
        return ret >>> 0;
    }
    constructor() {
        const ret = wasm.worldgen_new();
        this.__wbg_ptr = ret;
        WorldGenFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * Replace the store from a flat SoA `[kind, x, z, rot, sx, sy, sz, color, keep]×n`. JS packs world.objects'
     * structures (in array order) once at load + whenever the structure set changes; the slot of each entry = its
     * index here, so a returned REMOVE slot maps back to the object id JS packed at that index.
     * @param {Float64Array} soa
     */
    seed(soa) {
        const ptr0 = passArrayF64ToWasm0(soa, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        wasm.worldgen_seed(this.__wbg_ptr, ptr0, len0);
    }
    /**
     * Binary snapshot of the live structures → IndexedDB stores the bytes (no JSON). Restored via `deserialize`.
     * @returns {Uint8Array}
     */
    serialize() {
        const ret = wasm.worldgen_serialize(this.__wbg_ptr);
        var v1 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
        return v1;
    }
    /**
     * `changed` = positions `[x,z]×n` of structures changed this frame → only those towns' walls re-fit (others
     * stay put). Empty = fit every town (the one-time load reconcile).
     * @param {Float64Array} zones
     * @param {Float64Array} changed
     * @returns {Float64Array}
     */
    settlement(zones, changed) {
        const ptr0 = passArrayF64ToWasm0(zones, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passArrayF64ToWasm0(changed, wasm.__wbindgen_malloc);
        const len1 = WASM_VECTOR_LEN;
        const ret = wasm.worldgen_settlement(this.__wbg_ptr, ptr0, len0, ptr1, len1);
        var v3 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
        return v3;
    }
    /**
     * SETTLEMENT PLAN (binary) — a deterministic town plan packed as `[radius, numPaths, numObjects, <paths×4>,
     * <objects×7>]` (paths then objects; JS rebuilds ids + Path/WorldObject shapes). No store needed.
     * @param {number} cx
     * @param {number} cz
     * @param {string} size
     * @param {number} seed
     * @returns {Float64Array}
     */
    town_plan(cx, cz, size, seed) {
        const ptr0 = passStringToWasm0(size, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.worldgen_town_plan(this.__wbg_ptr, cx, cz, ptr0, len0, seed);
        var v2 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
        return v2;
    }
    /**
     * @param {number} seed
     * @param {Float64Array} zones
     * @returns {Float64Array}
     */
    veg(seed, zones) {
        const ptr0 = passArrayF64ToWasm0(zones, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.worldgen_veg(this.__wbg_ptr, seed, ptr0, len0);
        var v2 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
        return v2;
    }
    /**
     * @param {Float64Array} reqs
     * @param {Float64Array} zones
     * @returns {Float64Array}
     */
    well(reqs, zones) {
        const ptr0 = passArrayF64ToWasm0(reqs, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passArrayF64ToWasm0(zones, wasm.__wbindgen_malloc);
        const len1 = WASM_VECTOR_LEN;
        const ret = wasm.worldgen_well(this.__wbg_ptr, ptr0, len0, ptr1, len1);
        var v3 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
        return v3;
    }
}
if (Symbol.dispose) WorldGen.prototype[Symbol.dispose] = WorldGen.prototype.free;

/**
 * THE BINARY ENGINE (the jzon-drop path, docs/world-data-architecture.md). Same op→world layer as `apply_ops`
 * but NO JSON: the world + ops cross as parallel string vecs + a flat f64 SoA (see engine_bin decode fns), the
 * new world + conflicts ride back in `ApplyResult`. Parity-pinned to `apply_ops` (engine_bin parity test + the
 * JS vitest). `obj_num` stride 9, `zone_num` 4, `path_num` 7, `terrain_num` 5; `op_num` 19, `op_strs` 11.
 * @param {string[]} obj_ids
 * @param {string[]} obj_kinds
 * @param {string[]} obj_colors
 * @param {Float64Array} obj_num
 * @param {string[]} zone_ids
 * @param {string[]} zone_materials
 * @param {string[]} zone_shapes
 * @param {Float64Array} zone_num
 * @param {string[]} path_ids
 * @param {string[]} path_materials
 * @param {Float64Array} path_num
 * @param {Float64Array} terrain_num
 * @param {string} ground
 * @param {string} sky
 * @param {Float64Array} op_num
 * @param {string[]} op_strs
 * @param {number} px
 * @param {number} pz
 * @param {number} yaw
 * @returns {ApplyResult}
 */
export function apply_ops_bin(obj_ids, obj_kinds, obj_colors, obj_num, zone_ids, zone_materials, zone_shapes, zone_num, path_ids, path_materials, path_num, terrain_num, ground, sky, op_num, op_strs, px, pz, yaw) {
    const ptr0 = passArrayJsValueToWasm0(obj_ids, wasm.__wbindgen_malloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passArrayJsValueToWasm0(obj_kinds, wasm.__wbindgen_malloc);
    const len1 = WASM_VECTOR_LEN;
    const ptr2 = passArrayJsValueToWasm0(obj_colors, wasm.__wbindgen_malloc);
    const len2 = WASM_VECTOR_LEN;
    const ptr3 = passArrayF64ToWasm0(obj_num, wasm.__wbindgen_malloc);
    const len3 = WASM_VECTOR_LEN;
    const ptr4 = passArrayJsValueToWasm0(zone_ids, wasm.__wbindgen_malloc);
    const len4 = WASM_VECTOR_LEN;
    const ptr5 = passArrayJsValueToWasm0(zone_materials, wasm.__wbindgen_malloc);
    const len5 = WASM_VECTOR_LEN;
    const ptr6 = passArrayJsValueToWasm0(zone_shapes, wasm.__wbindgen_malloc);
    const len6 = WASM_VECTOR_LEN;
    const ptr7 = passArrayF64ToWasm0(zone_num, wasm.__wbindgen_malloc);
    const len7 = WASM_VECTOR_LEN;
    const ptr8 = passArrayJsValueToWasm0(path_ids, wasm.__wbindgen_malloc);
    const len8 = WASM_VECTOR_LEN;
    const ptr9 = passArrayJsValueToWasm0(path_materials, wasm.__wbindgen_malloc);
    const len9 = WASM_VECTOR_LEN;
    const ptr10 = passArrayF64ToWasm0(path_num, wasm.__wbindgen_malloc);
    const len10 = WASM_VECTOR_LEN;
    const ptr11 = passArrayF64ToWasm0(terrain_num, wasm.__wbindgen_malloc);
    const len11 = WASM_VECTOR_LEN;
    const ptr12 = passStringToWasm0(ground, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len12 = WASM_VECTOR_LEN;
    const ptr13 = passStringToWasm0(sky, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len13 = WASM_VECTOR_LEN;
    const ptr14 = passArrayF64ToWasm0(op_num, wasm.__wbindgen_malloc);
    const len14 = WASM_VECTOR_LEN;
    const ptr15 = passArrayJsValueToWasm0(op_strs, wasm.__wbindgen_malloc);
    const len15 = WASM_VECTOR_LEN;
    const ret = wasm.apply_ops_bin(ptr0, len0, ptr1, len1, ptr2, len2, ptr3, len3, ptr4, len4, ptr5, len5, ptr6, len6, ptr7, len7, ptr8, len8, ptr9, len9, ptr10, len10, ptr11, len11, ptr12, len12, ptr13, len13, ptr14, len14, ptr15, len15, px, pz, yaw);
    return ApplyResult.__wrap(ret);
}

/**
 * Spawn-spread layout for a big creature batch ("100 humans"): BANDS of up to 10 laid on a golden-spiral
 * around the anchor, members loosely clustered within each band, spread wide (~22·√count) so most land BEYOND
 * the mesh-reveal radius → cheap LOD impostors, no mount-storm jank. Returns flat [x,z,…] snapped to 0.5 m.
 * The deterministic op→placement math lives HERE in Rust, not in the JS engine.
 * @param {number} count
 * @param {number} ax
 * @param {number} az
 * @param {number} r
 * @returns {Float64Array}
 */
export function band_spread(count, ax, az, r) {
    const ret = wasm.band_spread(count, ax, az, r);
    var v1 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
    return v1;
}

/**
 * AMBIENT BUSHES near (px,pz). Flat [x, z, scale, rot, colorHash] × n.
 * @param {number} px
 * @param {number} pz
 * @param {number} reach
 * @returns {Float64Array}
 */
export function bushes_near(px, pz, reach) {
    const ret = wasm.bushes_near(px, pz, reach);
    var v1 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
    return v1;
}

/**
 * The RENDER slice of the eco table — [rank, speed_lo, speed_hi] per kind, by Kind order. Rust owns the full
 * canonical eco.rs; the renderer reads ONLY what it needs (gait speed range + rank) from here, no JS copy.
 * @returns {Float64Array}
 */
export function eco_render() {
    const ret = wasm.eco_render();
    var v1 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
    return v1;
}

/**
 * Female fertile WINDOW (seconds) per kind — maturity → menopause/old-age. The SAME numbers the sim breeds by,
 * so the HUD's per-species TFR estimate (births ÷ fertile females × this window) never drifts from the sim.
 * Returns [rabbit, cat, kangaroo, person, lion, dino].
 * @returns {Float64Array}
 */
export function fertile_windows() {
    const ret = wasm.fertile_windows();
    var v1 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
    return v1;
}

/**
 * Closed-form VIGOR drift for a dormant region over `dt` seconds away — evolves the offloaded population's mean
 * gene under predation pressure (no ticking). Lets a dormant region EVOLVE via the clock, not stay frozen.
 * @param {number} gene
 * @param {number} rabbit
 * @param {number} cat
 * @param {number} kangaroo
 * @param {number} person
 * @param {number} lion
 * @param {number} dino
 * @param {number} dt
 * @returns {number}
 */
export function ff_gene(gene, rabbit, cat, kangaroo, person, lion, dino, dt) {
    const ret = wasm.ff_gene(gene, rabbit, cat, kangaroo, person, lion, dino, dt);
    return ret;
}

/**
 * Aggregate fast-forward: advance the 6 populations by `dt` seconds away toward carrying capacity (closed-form
 * logistic). Returns target headcounts [rabbit, cat, kangaroo, person, lion, dino] — JS materialises the deltas.
 * @param {number} rabbit
 * @param {number} cat
 * @param {number} kangaroo
 * @param {number} person
 * @param {number} lion
 * @param {number} dino
 * @param {number} scale
 * @param {number} dt
 * @returns {Uint32Array}
 */
export function ff_targets(rabbit, cat, kangaroo, person, lion, dino, scale, dt) {
    const ret = wasm.ff_targets(rabbit, cat, kangaroo, person, lion, dino, scale, dt);
    var v1 = getArrayU32FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
    return v1;
}

/**
 * The VIGOR gene bounds [GENE_MIN, GENE_MAX] — the sim's source of truth, so the JS clamps that defensively
 * keep a read-back/aggregate gene in range read it from here instead of hard-coding 0.6/1.6 in six places.
 * @returns {Float64Array}
 */
export function gene_bounds() {
    const ret = wasm.gene_bounds();
    var v1 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
    return v1;
}

/**
 * Per-kind GESTATION seconds, by Kind order [rabbit, cat, kangaroo, person, lion, dinosaur] — the sim's source
 * of truth (world::gestation), so the renderer paces the pregnancy belly-grow to the REAL delivery time instead
 * of a duplicated guess (the belly hit full term well before/after delivery when JS hard-coded its own number).
 * @returns {Float64Array}
 */
export function gestation_secs() {
    const ret = wasm.gestation_secs();
    var v1 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
    return v1;
}

/**
 * Kind FOOTPRINT [radius, height] — engine.rs `kind_rh` is the collision source of truth. The JS `KINDS` table
 * keeps its own r/h copy (it also carries render geometry), so a parity test (src/lib/kinds.test.ts) pins the JS
 * numbers to these — a drift would mean placement/collision disagreeing with what's drawn. Unknown → fallback.
 * @param {string} kind
 * @returns {Float64Array}
 */
export function kind_rh(kind) {
    const ptr0 = passStringToWasm0(kind, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.kind_rh(ptr0, len0);
    var v2 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
    return v2;
}

/**
 * Per-kind MIGRATION weight, by Kind order [rabbit, cat, kangaroo, person, lion, dinosaur] — the sim's source
 * of truth (world::migrate_weight), so the HUD reads it from here instead of hard-coding a duplicate copy.
 * @returns {Float64Array}
 */
export function migrate_weights() {
    const ret = wasm.migrate_weights();
    var v1 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
    return v1;
}

/**
 * NATURAL PONDS near (px,pz) within `reach` — Rust owns the world's water (a deterministic, even, infinite
 * pond field); the renderer calls this once per area to DRAW them. Flat [x, z, radius, …]. Cheap + stateless.
 * @param {number} px
 * @param {number} pz
 * @param {number} reach
 * @returns {Float64Array}
 */
export function ponds_near(px, pz, reach) {
    const ret = wasm.ponds_near(px, pz, reach);
    var v1 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
    return v1;
}

/**
 * Carrying caps for the 6 kinds given live counts + world-area scale — the SAME `cap_for` the sim uses, so JS
 * (load-trim / scatter) never re-derives the formula. Returns [rabbit, cat, kangaroo, person, lion, dino].
 * @param {number} rabbit
 * @param {number} cat
 * @param {number} kangaroo
 * @param {number} person
 * @param {number} lion
 * @param {number} dino
 * @param {number} scale
 * @returns {Uint32Array}
 */
export function pop_caps(rabbit, cat, kangaroo, person, lion, dino, scale) {
    const ret = wasm.pop_caps(rabbit, cat, kangaroo, person, lion, dino, scale);
    var v1 = getArrayU32FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
    return v1;
}

/**
 * @param {number} position
 * @param {number} seed
 * @returns {number}
 */
export function rng_hash(position, seed) {
    const ret = wasm.rng_hash(position, seed);
    return ret >>> 0;
}

/**
 * @param {number} seed
 * @param {Int32Array} keys
 * @returns {number}
 */
export function rng_hash_keys(seed, keys) {
    const ptr0 = passArray32ToWasm0(keys, wasm.__wbindgen_malloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.rng_hash_keys(seed, ptr0, len0);
    return ret >>> 0;
}

/**
 * @param {number} seed
 * @param {Int32Array} keys
 * @returns {number}
 */
export function rng_rand(seed, keys) {
    const ptr0 = passArray32ToWasm0(keys, wasm.__wbindgen_malloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.rng_rand(seed, ptr0, len0);
    return ret;
}

/**
 * @param {string} s
 * @returns {number}
 */
export function rng_seed_from(s) {
    const ptr0 = passStringToWasm0(s, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.rng_seed_from(ptr0, len0);
    return ret >>> 0;
}

/**
 * STATELESS settlement wall refit (the jzon-drop for the away-growth / fast-forward path). Builds a THROWAWAY
 * `StructureStore` from `soa` (`[kind,x,z,rot,sx,sy,sz,color,keep]×n`, same layout as `WorldGen.seed`), fits every
 * town's perimeter against `zones` (water `[px,pz,size,seed]×n`), and returns the GEN op stream — WITHOUT touching
 * the persistent live `WorldGen` store, so the renderer's incremental fence state is never clobbered. A REMOVE
 * references its target by slot (index in `soa`), which JS maps back to the object id it packed at that index.
 * @param {Float64Array} soa
 * @param {Float64Array} zones
 * @returns {Float64Array}
 */
export function settlement_ops_bin(soa, zones) {
    const ptr0 = passArrayF64ToWasm0(soa, wasm.__wbindgen_malloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passArrayF64ToWasm0(zones, wasm.__wbindgen_malloc);
    const len1 = WASM_VECTOR_LEN;
    const ret = wasm.settlement_ops_bin(ptr0, len0, ptr1, len1);
    var v3 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
    return v3;
}

/**
 * AMBIENT terrain height at (x,z) with no contained features — the deterministic wilderness relief. The render
 * (terrain.ts heightAt) keeps a native copy (it runs per-frame to ground objects + before the wasm loads), so
 * this exists to PARITY-TEST that copy against Rust (src/lib/terrain.test.ts). Feature patches blend on top in
 * both copies; the ambient field is the shared core most likely to drift on a tweak.
 * @param {number} x
 * @param {number} z
 * @returns {number}
 */
export function terrain_height(x, z) {
    const ret = wasm.terrain_height(x, z);
    return ret;
}

/**
 * Sim ticks per second (1 / DT) — the fixed-timestep rate, so JS region-streaming derives dormant-span seconds
 * from the sim's clock instead of a duplicated `TICK_HZ = 30`.
 * @returns {number}
 */
export function tick_hz() {
    const ret = wasm.tick_hz();
    return ret;
}

/**
 * AMBIENT TREES near (px,pz) — Rust owns the forest field. Flat [x, z, scale, scaleY, rot, colorHash] × n.
 * The renderer + collision read this ONCE per rebuild (cheap); JS culls trees on its own paths/lakes.
 * @param {number} px
 * @param {number} pz
 * @param {number} reach
 * @returns {Float64Array}
 */
export function trees_near(px, pz, reach) {
    const ret = wasm.trees_near(px, pz, reach);
    var v1 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
    return v1;
}

/**
 * Pond SHORELINE radius factor at `ang` for a `seed` — the organic-blob edge. The render keeps a native copy
 * (player wade check runs per frame, pre-wasm-load), so this exists to PARITY-TEST that copy against Rust.
 * @param {number} seed
 * @param {number} ang
 * @returns {number}
 */
export function water_edge_factor(seed, ang) {
    const ret = wasm.water_edge_factor(seed, ang);
    return ret;
}

/**
 * Pond per-id SEED (matches the render's waterSeed) — exposed so a parity test pins the JS copy to Rust.
 * @param {string} id
 * @returns {number}
 */
export function water_seed(id) {
    const ptr0 = passStringToWasm0(id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.water_seed(ptr0, len0);
    return ret;
}

/**
 * World-AREA carrying-capacity multiplier from the built count — the SAME formula the sim + fast-forward use, so
 * the scale JS feeds into `cap_for`/`ff_targets` can never drift. JS counts the buildings; Rust owns the math.
 * @param {number} builds
 * @returns {number}
 */
export function world_area_scale(builds) {
    const ret = wasm.world_area_scale(builds);
    return ret;
}
function __wbg_get_imports() {
    const import0 = {
        __proto__: null,
        __wbg___wbindgen_string_get_71bb4348194e31f0: function(arg0, arg1) {
            const obj = arg1;
            const ret = typeof(obj) === 'string' ? obj : undefined;
            var ptr1 = isLikeNone(ret) ? 0 : passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            var len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg___wbindgen_throw_ea4887a5f8f9a9db: function(arg0, arg1) {
            throw new Error(getStringFromWasm0(arg0, arg1));
        },
        __wbindgen_cast_0000000000000001: function(arg0, arg1) {
            // Cast intrinsic for `Ref(String) -> Externref`.
            const ret = getStringFromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_init_externref_table: function() {
            const table = wasm.__wbindgen_externrefs;
            const offset = table.grow(4);
            table.set(0, undefined);
            table.set(offset + 0, undefined);
            table.set(offset + 1, null);
            table.set(offset + 2, true);
            table.set(offset + 3, false);
        },
    };
    return {
        __proto__: null,
        "./worldsim_bg.js": import0,
    };
}

const ApplyResultFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_applyresult_free(ptr, 1));
const SimFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_sim_free(ptr, 1));
const WorldGenFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_worldgen_free(ptr, 1));

function addToExternrefTable0(obj) {
    const idx = wasm.__externref_table_alloc();
    wasm.__wbindgen_externrefs.set(idx, obj);
    return idx;
}

function getArrayF32FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getFloat32ArrayMemory0().subarray(ptr / 4, ptr / 4 + len);
}

function getArrayF64FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getFloat64ArrayMemory0().subarray(ptr / 8, ptr / 8 + len);
}

function getArrayJsValueFromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    const mem = getDataViewMemory0();
    const result = [];
    for (let i = ptr; i < ptr + 4 * len; i += 4) {
        result.push(wasm.__wbindgen_externrefs.get(mem.getUint32(i, true)));
    }
    wasm.__externref_drop_slice(ptr, len);
    return result;
}

function getArrayU32FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint32ArrayMemory0().subarray(ptr / 4, ptr / 4 + len);
}

function getArrayU8FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint8ArrayMemory0().subarray(ptr / 1, ptr / 1 + len);
}

let cachedDataViewMemory0 = null;
function getDataViewMemory0() {
    if (cachedDataViewMemory0 === null || cachedDataViewMemory0.buffer.detached === true || (cachedDataViewMemory0.buffer.detached === undefined && cachedDataViewMemory0.buffer !== wasm.memory.buffer)) {
        cachedDataViewMemory0 = new DataView(wasm.memory.buffer);
    }
    return cachedDataViewMemory0;
}

let cachedFloat32ArrayMemory0 = null;
function getFloat32ArrayMemory0() {
    if (cachedFloat32ArrayMemory0 === null || cachedFloat32ArrayMemory0.byteLength === 0) {
        cachedFloat32ArrayMemory0 = new Float32Array(wasm.memory.buffer);
    }
    return cachedFloat32ArrayMemory0;
}

let cachedFloat64ArrayMemory0 = null;
function getFloat64ArrayMemory0() {
    if (cachedFloat64ArrayMemory0 === null || cachedFloat64ArrayMemory0.byteLength === 0) {
        cachedFloat64ArrayMemory0 = new Float64Array(wasm.memory.buffer);
    }
    return cachedFloat64ArrayMemory0;
}

function getStringFromWasm0(ptr, len) {
    return decodeText(ptr >>> 0, len);
}

let cachedUint32ArrayMemory0 = null;
function getUint32ArrayMemory0() {
    if (cachedUint32ArrayMemory0 === null || cachedUint32ArrayMemory0.byteLength === 0) {
        cachedUint32ArrayMemory0 = new Uint32Array(wasm.memory.buffer);
    }
    return cachedUint32ArrayMemory0;
}

let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
    if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
        cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8ArrayMemory0;
}

function isLikeNone(x) {
    return x === undefined || x === null;
}

function passArray32ToWasm0(arg, malloc) {
    const ptr = malloc(arg.length * 4, 4) >>> 0;
    getUint32ArrayMemory0().set(arg, ptr / 4);
    WASM_VECTOR_LEN = arg.length;
    return ptr;
}

function passArray8ToWasm0(arg, malloc) {
    const ptr = malloc(arg.length * 1, 1) >>> 0;
    getUint8ArrayMemory0().set(arg, ptr / 1);
    WASM_VECTOR_LEN = arg.length;
    return ptr;
}

function passArrayF64ToWasm0(arg, malloc) {
    const ptr = malloc(arg.length * 8, 8) >>> 0;
    getFloat64ArrayMemory0().set(arg, ptr / 8);
    WASM_VECTOR_LEN = arg.length;
    return ptr;
}

function passArrayJsValueToWasm0(array, malloc) {
    const ptr = malloc(array.length * 4, 4) >>> 0;
    for (let i = 0; i < array.length; i++) {
        const add = addToExternrefTable0(array[i]);
        getDataViewMemory0().setUint32(ptr + 4 * i, add, true);
    }
    WASM_VECTOR_LEN = array.length;
    return ptr;
}

function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length, 1) >>> 0;
        getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;

    const mem = getUint8ArrayMemory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }
    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
        const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
        const ret = cachedTextEncoder.encodeInto(arg, view);

        offset += ret.written;
        ptr = realloc(ptr, len, offset, 1) >>> 0;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}

let cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
cachedTextDecoder.decode();
const MAX_SAFARI_DECODE_BYTES = 2146435072;
let numBytesDecoded = 0;
function decodeText(ptr, len) {
    numBytesDecoded += len;
    if (numBytesDecoded >= MAX_SAFARI_DECODE_BYTES) {
        cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
        cachedTextDecoder.decode();
        numBytesDecoded = len;
    }
    return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}

const cachedTextEncoder = new TextEncoder();

if (!('encodeInto' in cachedTextEncoder)) {
    cachedTextEncoder.encodeInto = function (arg, view) {
        const buf = cachedTextEncoder.encode(arg);
        view.set(buf);
        return {
            read: arg.length,
            written: buf.length
        };
    };
}

let WASM_VECTOR_LEN = 0;

let wasmModule, wasmInstance, wasm;
function __wbg_finalize_init(instance, module) {
    wasmInstance = instance;
    wasm = instance.exports;
    wasmModule = module;
    cachedDataViewMemory0 = null;
    cachedFloat32ArrayMemory0 = null;
    cachedFloat64ArrayMemory0 = null;
    cachedUint32ArrayMemory0 = null;
    cachedUint8ArrayMemory0 = null;
    wasm.__wbindgen_start();
    return wasm;
}

async function __wbg_load(module, imports) {
    if (typeof Response === 'function' && module instanceof Response) {
        if (typeof WebAssembly.instantiateStreaming === 'function') {
            try {
                return await WebAssembly.instantiateStreaming(module, imports);
            } catch (e) {
                const validResponse = module.ok && expectedResponseType(module.type);

                if (validResponse && module.headers.get('Content-Type') !== 'application/wasm') {
                    console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", e);

                } else { throw e; }
            }
        }

        const bytes = await module.arrayBuffer();
        return await WebAssembly.instantiate(bytes, imports);
    } else {
        const instance = await WebAssembly.instantiate(module, imports);

        if (instance instanceof WebAssembly.Instance) {
            return { instance, module };
        } else {
            return instance;
        }
    }

    function expectedResponseType(type) {
        switch (type) {
            case 'basic': case 'cors': case 'default': return true;
        }
        return false;
    }
}

function initSync(module) {
    if (wasm !== undefined) return wasm;


    if (module !== undefined) {
        if (Object.getPrototypeOf(module) === Object.prototype) {
            ({module} = module)
        } else {
            console.warn('using deprecated parameters for `initSync()`; pass a single object instead')
        }
    }

    const imports = __wbg_get_imports();
    if (!(module instanceof WebAssembly.Module)) {
        module = new WebAssembly.Module(module);
    }
    const instance = new WebAssembly.Instance(module, imports);
    return __wbg_finalize_init(instance, module);
}

async function __wbg_init(module_or_path) {
    if (wasm !== undefined) return wasm;


    if (module_or_path !== undefined) {
        if (Object.getPrototypeOf(module_or_path) === Object.prototype) {
            ({module_or_path} = module_or_path)
        } else {
            console.warn('using deprecated parameters for the initialization function; pass a single object instead')
        }
    }

    if (module_or_path === undefined) {
        module_or_path = new URL('worldsim_bg.wasm?v=9f7ce3405a', import.meta.url);
    }
    const imports = __wbg_get_imports();

    if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL)) {
        module_or_path = fetch(module_or_path);
    }

    const { instance, module } = await __wbg_load(await module_or_path, imports);

    return __wbg_finalize_init(instance, module);
}

export { initSync, __wbg_init as default };
