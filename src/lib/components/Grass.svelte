<script lang="ts">
	// GPU procedural grass. Every blade's world position is computed in the VERTEX SHADER from its
	// gl_InstanceID + a player-position uniform, so the CPU never re-places blades. Blades anchor to
	// ABSOLUTE world cells (floor((player+offset)/step)), so they stay put in the world as you walk —
	// instances just hand off cells; only the far rim changes, faded by distance LOD.
	//
	// RANGE comes from concentric LOD TIERS: a dense near grid, then coarser grids (bigger step + bigger
	// blades) covering far annuli, crossfading where they meet. A single uniform grid can't thin out with
	// distance without breaking the cell anchoring, so each tier is its own world-stable grid — together
	// they reach far for a modest blade budget. Terrain height + lake/path carving are reimplemented in
	// GLSL (mirrors terrain.ts / the SKIP set). Material is MeshStandardMaterial patched via
	// onBeforeCompile, so scene lighting, received shadows and fog all still apply. See docs/.
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { playerState } from '$lib/playerState.svelte';
	import { agentManager } from '$lib/agents.svelte';
	import { GROUND_COLOR } from '$lib/kinds';
	import { WIND_GUST } from '$lib/wind';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();

	const MAXF = 12; // GLSL cap on terrain features (hills/mountains/dunes)
	const MAXZ = 16; // GLSL cap on carved-out lake/path zones
	const MAXP = 16; // GLSL cap on carved-out paths (roads/rivers)
	const MAXT = 12; // GLSL cap on grass tramplers (player + nearest creatures/people pressing the grass)
	const MAXWR = 8; // GLSL cap on water zones that grow a REED bed at their bank
	const SKIP = new Set(['water', 'path', 'plaza', 'ice', 'lava']); // no grass on water / paving

	// LOD tiers (inner→outer). step = blade spacing (m); rIn/rOut = the annulus it fills; band = crossfade
	// width at each edge; bmin/bmax = blade height range; w = blade width. Adjacent tiers share an edge
	// (near.rOut ≈ mid.rIn …) so their crossfades sum to ~full density with no seam.
	type Tier = { step: number; rIn: number; rOut: number; band: number; bmin: number; bmax: number; w: number };
	// Normal-height blades, but they grow WIDER (not taller) with distance so far grass stays visible as a
	// textured carpet instead of vanishing to sub-pixel. Only the very outer rim dissolves into the ground
	// colour over a long band, so the field reads as grass all the way out with no hard ring.
	const TIERS: Tier[] = [
		{ step: 0.55, rIn: 0, rOut: 56, band: 22, bmin: 0.28, bmax: 0.68, w: 0.08 }, // near — dense, fine (shorter blades per user)
		{ step: 1.1, rIn: 48, rOut: 130, band: 30, bmin: 0.38, bmax: 0.88, w: 0.2 }, // mid — lush out to ~130 m
		{ step: 2.2, rIn: 120, rOut: 260, band: 70, bmin: 0.56, bmax: 1.06, w: 0.5 } // far — wide short blades read as texture, dissolve 190→260
	];

	// ── shared uniforms (mutated each frame / on world change; the SAME objects feed every tier) ────────
	const uniforms = {
		uTime: { value: 0 },
		uPlayer: { value: new THREE.Vector2() },
		uGround: { value: new THREE.Color(GROUND_COLOR.grass).convertSRGBToLinear() }, // blades take the ground's tone
		uFeatCount: { value: 0 },
		uFeatures: { value: Array.from({ length: MAXF }, () => new THREE.Vector4()) }, // x,z,radius,height
		uSkipCount: { value: 0 },
		uSkip: { value: Array.from({ length: MAXZ }, () => new THREE.Vector3()) }, // x,z,size
		uPathCount: { value: 0 },
		uPaths: { value: Array.from({ length: MAXP }, () => new THREE.Vector4()) }, // fromX,fromZ,toX,toZ
		uPathW: { value: new Array(MAXP).fill(0) }, // half-widths
		uCloud: { value: 1 }, // drifting cloud-shadow strength by sky (matches Terrain) → shadows darken the grass too
		uWet: { value: 0 }, // 1 under the rainy 'fog' sky → wet grass darkens with the wet terrain
		uTrampleCount: { value: 0 },
		uTramplers: { value: Array.from({ length: MAXT }, () => new THREE.Vector3()) }, // x, z, reach — player + nearby agents
		uWaterCount: { value: 0 },
		uWater: { value: Array.from({ length: MAXWR }, () => new THREE.Vector3()) } // x, z, size — reed bed at each pond bank
	};
	// MUST mirror Terrain.svelte's CLOUD_SHADOW so grass + terrain darken by the same amount under a cloud
	const CLOUD_SHADOW: Record<string, number> = { day: 1, sunset: 0.7, fog: 0.3, night: 0.15, space: 0 };

	// terrain + carving in GLSL — keep terrainH() in sync with terrain.ts heightAt().
	const COMMON = /* glsl */ `
		uniform float uTime;
		uniform vec2 uPlayer;
		uniform vec3 uGround;
		uniform int uFeatCount;
		uniform vec4 uFeatures[${MAXF}];
		uniform int uSkipCount;
		uniform vec3 uSkip[${MAXZ}];
		uniform int uPathCount;
		uniform vec4 uPaths[${MAXP}];
		uniform float uPathW[${MAXP}];
		uniform float uCloud;
		uniform float uWet;
		uniform int uTrampleCount;
		uniform vec3 uTramplers[${MAXT}];
		uniform int uWaterCount;
		uniform vec3 uWater[${MAXWR}];
		varying vec3 vGrass;
		float h21(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
		// value-noise fbm IDENTICAL to terrain.ts terNoise/terFbm (h21 == terHash) → the cloud shadow on the
		// grass lines up exactly with the one on the terrain where the field fades out.
		float gNoise(vec2 p){
			vec2 i = floor(p), f = fract(p);
			float a = h21(i), b = h21(i + vec2(1.0, 0.0)), c = h21(i + vec2(0.0, 1.0)), d = h21(i + vec2(1.0, 1.0));
			vec2 u = f * f * (3.0 - 2.0 * f);
			return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
		}
		float gFbm(vec2 p){ float v = 0.0, a = 0.5; for (int i = 0; i < 4; i++) { v += a * gNoise(p); p *= 2.03; a *= 0.5; } return v; }
		float ambientH(vec2 p){
			// MUST stay identical to terrain.ts ambient() or the grass floats off the ground
			float ramp = smoothstep(70.0, 240.0, length(p));
			if (ramp <= 0.0) return 0.0;
			float reg = sin(p.x * 0.0016 + 2.3) * cos(p.y * 0.0014 - 1.1);
			float hilly = smoothstep(-0.35, 0.5, reg);
			float ridged = smoothstep(0.45, 0.95, reg);
			float h = (6.0 * sin(p.x * 0.012 + 1.3) * cos(p.y * 0.011 - 0.7) + 3.0 * sin(p.x * 0.03 - 2.1) * cos(p.y * 0.028 + 1.1)) * (0.4 + hilly);
			float plat = sin(p.x * 0.0021 - 0.6) * cos(p.y * 0.0019 + 2.0);
			h += 13.0 * smoothstep(0.55, 0.82, plat);
			float m = sin(p.x * 0.008 + 4.2) * cos(p.y * 0.0075 - 3.3);
			h += (18.0 + 24.0 * ridged) * max(0.0, m - 0.5);
			return h * ramp;
		}
		float terrainH(vec2 p){
			float h = ambientH(p);
			for (int i = 0; i < ${MAXF}; i++) {
				if (i >= uFeatCount) break;
				vec4 f = uFeatures[i];
				float d = distance(p, f.xy);
				if (d < f.z) h += f.w * 0.5 * (cos(3.14159265 * d / f.z) + 1.0);
			}
			return h;
		}
		bool inSkip(vec2 p){
			for (int i = 0; i < ${MAXZ}; i++) {
				if (i >= uSkipCount) break;
				vec3 z = uSkip[i];
				if (distance(p, z.xy) < z.z) return true;
			}
			return false;
		}
		bool onPath(vec2 p){
			for (int i = 0; i < ${MAXP}; i++) {
				if (i >= uPathCount) break;
				vec4 s = uPaths[i];          // segment a=s.xy → b=s.zw
				vec2 ab = s.zw - s.xy;
				float t = clamp(dot(p - s.xy, ab) / max(dot(ab, ab), 1e-4), 0.0, 1.0);
				if (distance(p, s.xy + ab * t) < uPathW[i] + 0.25) return true; // within the road (+ a lip)
			}
			return false;
		}
	`;

	function tierBody(t: Tier, G: number): string {
		const fin = t.rIn > 0 ? `smoothstep(${t.rIn.toFixed(1)}, ${(t.rIn + t.band).toFixed(1)}, dist)` : `1.0`;
		return /* glsl */ `
			int gz = gl_InstanceID / ${G};
			int gx = gl_InstanceID - gz * ${G};
			vec2 local = (vec2(float(gx), float(gz)) - ${(G / 2).toFixed(1)}) * ${t.step.toFixed(4)};
			vec2 cell = floor((uPlayer + local) / ${t.step.toFixed(4)});      // ABSOLUTE world cell → stays put
			vec2 jit = vec2(h21(cell), h21(cell + 13.7)) - 0.5;
			vec2 wxz = (cell + 0.5 + jit * 0.9) * ${t.step.toFixed(4)};        // blade world XZ
			float dist = distance(wxz, uPlayer);
			float fin = ${fin};                                               // fade IN at the inner edge
			float fout = 1.0 - smoothstep(${(t.rOut - t.band).toFixed(1)}, ${t.rOut.toFixed(1)}, dist); // dissolve OUT
			float lod = fin * fout;

			// CULL FIRST: a blade outside this tier's annulus (the inner disc inner tiers already fill, plus the
			// square's corners) or carved out by a zone/road collapses to a zero-area triangle — so the heavy
			// per-blade work below (terrainH fbm, lean, wind sway, colour) is SKIPPED for it. Big GPU saving on
			// the outer tiers, whose grid mostly covers cells they don't actually draw. No visual change.
			if (lod <= 0.0 || inSkip(wxz) || onPath(wxz)) {
				transformed = vec3(0.0);
				vGrass = vec3(0.0);
			} else {
				float bladeH = mix(${t.bmin.toFixed(3)}, ${t.bmax.toFixed(3)}, h21(cell + 5.3)) * lod;
					float fl = step(0.965, h21(cell + 17.3));                     // ~3.5% of blades are WILDFLOWERS
					bladeH *= 1.0 + fl * 0.45;                                    // ...standing taller so the bloom clears the grass
				// REEDS: in a ~2.4 m band just outside any pond, ~58% of blades shoot up tall (a reed bed framing
				// the lake); patchy via a hash so it reads as reeds, not a hedge. Grass inside water is already
				// culled (inSkip), so this only affects the dry bank.
				float reed = 0.0;
				for (int i = 0; i < ${MAXWR}; i++) {
					if (i >= uWaterCount) break;
					vec3 wz = uWater[i];
					float dwr = distance(wxz, wz.xy);
					reed = max(reed, 1.0 - smoothstep(wz.z, wz.z + 2.4, dwr));
				}
				reed *= step(0.42, h21(cell + 31.7));                         // ~58% of band blades are reeds
				bladeH *= 1.0 + reed * 1.1;                                   // reeds stand ~2× tall
				float tt = position.y;                                        // 0 base .. 1 tip
				float yaw = h21(cell + 3.1) * 6.2831853;
				float cs = cos(yaw), sn = sin(yaw);
				vec2 dir = vec2(sin(yaw), cos(yaw));                          // blade facing / lean direction
				vec3 bp = vec3(position.x, position.y * bladeH, position.z);
				vec3 rp = vec3(bp.x * cs - bp.z * sn, bp.y, bp.x * sn + bp.z * cs);
				float bendT = tt * tt;                                        // bend grows toward the tip
				float lean = mix(0.18, 0.55, h21(cell + 7.7));                // each blade leans (not a rigid spike)
					float gust = (sin(uTime * 1.5 + wxz.x * 0.25 + wxz.y * 0.2) + 0.4 * sin(uTime * 1.27 + wxz.y * 0.3)) * ${WIND_GUST};
				rp.x += (dir.x * lean + gust * 0.22) * bendT * bladeH;        // static lean + wind sway
				rp.z += (dir.y * lean - gust * 0.16) * bendT * bladeH;
				// TRAMPLE: blades within reach bend radially AWAY from each nearby creature/person (and you) and
				// press toward the ground, so a herd parts the grass where it stands and you carve a path as you
				// walk. uTramplers[i] = (worldX, worldZ, reach); springs back instantly (recomputed each frame).
				for (int ti = 0; ti < ${MAXT}; ti++) {
					if (ti >= uTrampleCount) break;
					vec3 tr3 = uTramplers[ti];
					vec2 dv = wxz - tr3.xy;
					float td = length(dv);
					float tramp = 1.0 - smoothstep(0.0, tr3.z, td);
					if (tramp <= 0.0) continue;
					vec2 awayDir = dv / max(td, 1e-3);
					rp.x += awayDir.x * tramp * 0.85 * bendT * bladeH;
					rp.z += awayDir.y * tramp * 0.85 * bendT * bladeH;
					rp.y -= tramp * tramp * 0.5 * bladeH * bendT;             // flatten the tip down (bendT → root stays planted)
				}
				float gh = terrainH(wxz);                                     // blade's ground height (reused for the biome tint)
				transformed = vec3(wxz.x, gh, wxz.y) + rp;

				// blades take the GROUND's tone (so they always blend) — darker at the base, varied per blade in
				// brightness AND a little hue — then fade to the ground colour at the rim so the field melts away.
				float v = mix(0.55, 1.25, h21(cell + 9.4));
				float hue = (h21(cell + 2.3) - 0.5) * 0.16;                   // some blades yellower, some deeper
				vec3 tone = max(uGround * v + vec3(hue, hue * 0.5, -hue * 0.6), 0.0);
				vec3 gcol = mix(uGround, tone * mix(0.55, 1.0, tt), fout);
				// BIOME tint by elevation — MUST match Terrain.svelte so the grass agrees with the ground it grows
				// on: dry/golden on the uplands, lusher in the valleys (was uniformly green over golden hills).
				float gelev = smoothstep(3.0, 16.0, gh);
				gcol = mix(gcol, gcol * vec3(1.16, 1.04, 0.60), 0.45 * gelev);         // hilltops → dry/golden
				gcol = mix(gcol, gcol * vec3(0.82, 1.06, 0.78), 0.20 * (1.0 - gelev)); // valleys → lush
				// wildflower bloom: tint the TIP of flower blades from a small pleasant palette → specks of colour in the meadow
				float fhue = h21(cell + 11.9);
				vec3 bloom = fhue < 0.3 ? vec3(0.96, 0.84, 0.34) : fhue < 0.55 ? vec3(0.93, 0.46, 0.66) : fhue < 0.8 ? vec3(0.97, 0.97, 1.0) : vec3(0.72, 0.56, 0.92);
				gcol = mix(gcol, bloom, fl * smoothstep(0.6, 0.92, tt) * fout);
				// reeds dry to an olive/tan toward their seed-head tips (bases stay green) → a marsh-edge look
				gcol = mix(gcol, vec3(0.46, 0.43, 0.24), reed * smoothstep(0.35, 0.95, tt) * 0.8);
				// drifting cloud shadow — MUST match Terrain's (now aligned to Clouds.svelte: ·0.0022 scale, slow
				// drift, +(86.7,57.8) sun-angle offset) so the shadow on the meadow you stand in lines up with the
				// shadow on the far ground AND with the clouds overhead / their water reflection.
				float gcloud = gFbm((wxz + vec2(86.7, 57.8)) * 0.0022 + vec2(uTime * 0.006, uTime * 0.004));
				gcol *= 1.0 - 0.30 * uCloud * smoothstep(0.55, 0.80, gcloud);
				gcol *= mix(1.0, 0.72, uWet); // wet grass darkens in the rain → matches the wet terrain it grows on
				vGrass = gcol;
			}
		`;
	}

	// one tapered, multi-segment blade of the given width (unit height; the shader scales/bends/places it)
	function bladeGeo(w: number): THREE.BufferGeometry {
		const SEG = 4;
		const pos: number[] = [];
		const idx: number[] = [];
		for (let k = 0; k <= SEG; k++) {
			const t = k / SEG;
			const hw = (w * 0.5) * (1 - t * 0.85);
			pos.push(-hw, t, 0, hw, t, 0);
		}
		for (let k = 0; k < SEG; k++) {
			const a = k * 2;
			idx.push(a, a + 1, a + 3, a, a + 3, a + 2);
		}
		const g = new THREE.BufferGeometry();
		g.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3));
		g.setIndex(idx);
		return g;
	}

	function makeTier(t: Tier): THREE.InstancedMesh {
		const G = Math.ceil((2 * t.rOut + 2) / t.step); // grid wide enough to fill the annulus + margin
		const mat = new THREE.MeshStandardMaterial({ color: 0xffffff, roughness: 0.95, metalness: 0, side: THREE.DoubleSide });
		mat.onBeforeCompile = (shader) => {
			Object.assign(shader.uniforms, uniforms); // share the uniform objects we mutate each frame
			shader.vertexShader = shader.vertexShader
				.replace('#include <common>', '#include <common>\n' + COMMON)
				.replace('#include <beginnormal_vertex>', 'vec3 objectNormal = vec3(0.0, 1.0, 0.0);') // lit from above
				.replace('#include <begin_vertex>', 'vec3 transformed;\n' + tierBody(t, G));
			shader.fragmentShader = shader.fragmentShader
				.replace('#include <common>', '#include <common>\nvarying vec3 vGrass;')
				.replace('#include <color_fragment>', '#include <color_fragment>\ndiffuseColor.rgb *= vGrass;');
		};
		const mesh = new THREE.InstancedMesh(bladeGeo(t.w), mat, G * G);
		mesh.castShadow = false; // thin blades in the shadow pass aren't worth it
		mesh.receiveShadow = true; // but buildings/trees cast onto the grass
		mesh.frustumCulled = false; // real positions live in the shader; CPU bounds are meaningless
		const identity = new THREE.Matrix4();
		for (let i = 0; i < G * G; i++) mesh.setMatrixAt(i, identity); // unused (pos is shader-computed) but must be valid
		mesh.instanceMatrix.needsUpdate = true;
		return mesh;
	}

	const meshes = TIERS.map(makeTier);

	// rebuild the feature/zone uniform arrays only when the world's terrain or zones change
	let lastTerrain = -1;
	let lastZones = -1;
	function syncWorldUniforms() {
		const feats = world.terrain ?? [];
		uniforms.uFeatCount.value = Math.min(feats.length, MAXF);
		for (let i = 0; i < uniforms.uFeatCount.value; i++) {
			const f = feats[i];
			uniforms.uFeatures.value[i].set(f.center[0], f.center[1], f.radius, f.height);
		}
		let n = 0;
		let wr = 0;
		for (const z of world.zones ?? []) {
			if (z.material === 'water' && wr < MAXWR) uniforms.uWater.value[wr++].set(z.pos[0], z.pos[2], z.size); // reed beds
			if (n >= MAXZ || !SKIP.has(z.material)) continue;
			uniforms.uSkip.value[n++].set(z.pos[0], z.pos[2], z.size);
		}
		uniforms.uSkipCount.value = n;
		uniforms.uWaterCount.value = wr;
		// roads/rivers (paths) carve grass too — feed each segment + its half-width
		let pc = 0;
		for (const p of world.paths ?? []) {
			if (pc >= MAXP) break;
			uniforms.uPaths.value[pc].set(p.from[0], p.from[2], p.to[0], p.to[2]);
			uniforms.uPathW.value[pc] = p.width / 2;
			pc++;
		}
		uniforms.uPathCount.value = pc;
	}

	// tramplers: the player + the nearest living agents press the grass. Reused buffers (no per-frame alloc);
	// trampDist mirrors uTramplers so we can keep the NEAREST agents when more than MAXT-1 are in range.
	const TRAMPLE_RANGE = 45; // agents farther than this can't reach the near grass tier → ignore (cheap cull)
	const trampDist = new Array(MAXT).fill(0);
	function gatherTramplers() {
		const tr = uniforms.uTramplers.value;
		tr[0].set(playerState.pos[0], playerState.pos[2], 1.45); // you trample a ~1.45 m circle at your feet
		let tc = 1;
		let maxI = -1; // index (≥1) of the farthest agent currently kept → the next to evict
		agentManager.forEach((m) => {
			if (m.dead || m.dist > TRAMPLE_RANGE) return;
			const reach = Math.max(0.8, m.radius * 2.2); // a big dino flattens a wider patch than a rabbit
			if (tc < MAXT) {
				tr[tc].set(m.agent.x, m.agent.z, reach);
				trampDist[tc] = m.dist;
				if (maxI < 0 || m.dist > trampDist[maxI]) maxI = tc;
				tc++;
			} else if (m.dist < trampDist[maxI]) {
				tr[maxI].set(m.agent.x, m.agent.z, reach);
				trampDist[maxI] = m.dist;
				maxI = 1; // re-find the farthest kept agent (slots 1..MAXT-1; slot 0 is always the player)
				for (let i = 2; i < MAXT; i++) if (trampDist[i] > trampDist[maxI]) maxI = i;
			}
		});
		uniforms.uTrampleCount.value = tc;
	}

	useTask((dt) => {
		const on = world.ground === 'grass';
		for (const m of meshes) m.visible = on;
		if (!on) return;
		uniforms.uTime.value += dt;
		uniforms.uPlayer.value.set(playerState.pos[0], playerState.pos[2]);
		gatherTramplers();
		uniforms.uCloud.value = CLOUD_SHADOW[world.sky] ?? 1; // cloud-shadow strength tracks the sky (matches Terrain)
		uniforms.uWet.value = world.sky === 'fog' ? 1 : 0; // wet grass under the rainy sky (matches Terrain's uWet)
		const tl = world.terrain?.length ?? 0;
		const zl = world.zones?.length ?? 0;
		if (tl !== lastTerrain || zl !== lastZones) {
			lastTerrain = tl;
			lastZones = zl;
			syncWorldUniforms();
		}
	});
</script>

{#each meshes as mesh (mesh.uuid)}
	<T is={mesh} />
{/each}
