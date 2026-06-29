<script lang="ts">
	// All near/mid creatures of the INSTANCED species drawn as ONE InstancedMesh PER species (rabbit, kangaroo, cat),
	// instead of an articulated Critter each — the big draw-call + per-frame win for dense crowds. Each species gets a
	// low-poly merged body + a VERTEX-SHADER gait keyed to actual ground travel (stride-locked, no skating): rabbits +
	// kangaroos BOUND (a whole-body arc + nose-dip — their real gait, so no skeleton needed); cats TROT (a subtle
	// grounded 2-beat bob + shoulder sway). Material = the same `creatureMat` recipe (soft coat + pale belly + moonlit
	// night-rim) with per-instance colour, so an instanced creature matches the articulated one. Idle → it sits/stands
	// still (gait gated by gaitRate). The companion cat KEEPS its articulated Critter (it follows you) → skipped here.
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { mergeGeometries } from 'three/addons/utils/BufferGeometryUtils.js';
	import { groundYCached } from '$lib/terrain';
	import { agentManager, type ManagedAgent } from '$lib/agents.svelte';
	import { creatureNight } from '$lib/sharedAssets';
	import { clock } from '$lib/clock';
	import type { World } from '$lib/world';

	let { world }: { world: World } = $props();
	const MAX = 1024;

	// ── geometry — built from the EXACT primitives the articulated Critter uses (its per-species body snippets), with
	//    the group transforms flattened in, so an instanced rabbit/kangaroo is the SAME shape as the original (just no
	//    per-leg articulation). PRIM bases: box 1³, sphere r0.5, cone r0.5/h1. `P` = one part: base × scale, rotated +
	//    positioned, optionally inside a group (its pos+rot). Built at the 0.35-radius base → `radius/0.35` recovers size.
	type V3 = [number, number, number];
	const baseGeo = (b: 'box' | 'sphere' | 'cone' | 'cyl') =>
		b === 'box' ? new THREE.BoxGeometry(1, 1, 1) : b === 'sphere' ? new THREE.SphereGeometry(0.5, 12, 10) : b === 'cone' ? new THREE.ConeGeometry(0.5, 1, 8) : new THREE.CylinderGeometry(0.5, 0.5, 1, 10);
	const _g = new THREE.Vector3();
	const _q = new THREE.Quaternion();
	const _s = new THREE.Vector3();
	const _e = new THREE.Euler();
	// a LEG part also carries, per vertex, the gait PHASE (diagonal trot: paired legs share a phase) + its HIP pivot
	// (Y,Z) so the WALK shader can swing the whole leg around the hip. Non-legs get legPh≥50 (sentinel → no swing).
	// `part` tags which per-instance COLOUR a vertex uses (humans only: 0=shirt, 1=pants, 2=skin, 3=hair); the animal
	// material ignores aPart and tints the whole instance via instanceColor, so this is purely additive.
	type Leg = { ph: number; hipY: number; hipZ: number };
	const P = (b: 'box' | 'sphere' | 'cone' | 'cyl', s: V3, p: V3, r: V3 = [0, 0, 0], gp: V3 = [0, 0, 0], gr: V3 = [0, 0, 0], leg?: Leg, mane = false, part = 0) => {
		const g = baseGeo(b);
		const mesh = new THREE.Matrix4().compose(_g.set(...p), _q.setFromEuler(_e.set(...r)), _s.set(...s));
		const grp = new THREE.Matrix4().compose(_g.set(...gp), _q.setFromEuler(_e.set(...gr)), new THREE.Vector3(1, 1, 1));
		g.applyMatrix4(grp.multiply(mesh));
		const ng = g.toNonIndexed();
		const n = ng.attributes.position.count;
		ng.setAttribute('aLegPh', new THREE.BufferAttribute(new Float32Array(n).fill(leg ? leg.ph : 99), 1));
		ng.setAttribute('aHipY', new THREE.BufferAttribute(new Float32Array(n).fill(leg ? leg.hipY : 0), 1));
		ng.setAttribute('aHipZ', new THREE.BufferAttribute(new Float32Array(n).fill(leg ? leg.hipZ : 0), 1));
		ng.setAttribute('aMane', new THREE.BufferAttribute(new Float32Array(n).fill(mane ? 1 : 0), 1)); // mane verts → collapse for females
		ng.setAttribute('aPart', new THREE.BufferAttribute(new Float32Array(n).fill(part), 1)); // 0=shirt 1=pants 2=skin 3=hair (humans)
		return ng;
	};
	const merge = (parts: THREE.BufferGeometry[]) => {
		const g = mergeGeometries(parts, false)!;
		g.computeVertexNormals();
		return g;
	};
	// rabbitBody snippet, verbatim (eyes omitted — they're a separate dark material instancing can't carry per-part)
	const buildRabbit = () =>
		merge([
			P('sphere', [0.36, 0.34, 0.5], [0, 0.26, 0]), // body
			P('sphere', [0.3, 0.3, 0.3], [0, 0, 0], [0, 0, 0], [0, 0.42, 0.28]), // head
			P('box', [0.08, 0.42, 0.04], [0.08, 0.32, 0], [0, 0, -0.12], [0, 0.42, 0.28]), // ears
			P('box', [0.08, 0.42, 0.04], [-0.08, 0.32, 0], [0, 0, 0.12], [0, 0.42, 0.28]),
			P('sphere', [0.16, 0.16, 0.16], [0, 0, 0], [0, 0, 0], [0, 0.3, -0.32]), // tail
			P('box', [0.08, 0.14, 0.08], [0, -0.07, 0], [0, 0, 0], [0.1, 0.14, 0.2]), // front legs
			P('box', [0.08, 0.14, 0.08], [0, -0.07, 0], [0, 0, 0], [-0.1, 0.14, 0.2]),
			P('box', [0.11, 0.16, 0.24], [0, -0.08, 0], [0, 0, 0], [0.13, 0.16, -0.16]), // hind legs
			P('box', [0.11, 0.16, 0.24], [0, -0.08, 0], [0, 0, 0], [-0.13, 0.16, -0.16])
		]);
	// kangarooBody snippet, verbatim
	const buildKangaroo = () =>
		merge([
			P('box', [0.36, 0.5, 0.34], [0, 0.95, 0.02]), // upper body
			P('box', [0.42, 0.4, 0.4], [0, 0.62, 0]), // lower body
			P('sphere', [0.26, 0.3, 0.34], [0, 0, 0], [0, 0, 0], [0, 1.3, 0.08]), // head
			P('cone', [0.09, 0.26, 0.09], [0.1, 0.26, 0], [0, 0, 0], [0, 1.3, 0.08]), // ears
			P('cone', [0.09, 0.26, 0.09], [-0.1, 0.26, 0], [0, 0, 0], [0, 1.3, 0.08]),
			P('box', [0.18, 0.18, 0.95], [0, 0, 0.42], [0, 0, 0], [0, 0.5, -0.18], [-0.9, 0, 0]), // heavy tail (rotated group)
			P('box', [0.07, 0.3, 0.07], [0, -0.15, 0], [0, 0, 0], [0.2, 1.0, 0.18]), // little arms
			P('box', [0.07, 0.3, 0.07], [0, -0.15, 0], [0, 0, 0], [-0.2, 1.0, 0.18]),
			P('box', [0.17, 0.5, 0.36], [0, -0.25, 0.06], [0, 0, 0], [0.18, 0.5, 0.04]), // big hind legs
			P('box', [0.17, 0.5, 0.36], [0, -0.25, 0.06], [0, 0, 0], [-0.18, 0.5, 0.04])
		]);
	// lionBody snippet (mane included for all — the per-instance male/female mane is a later refinement). The 4 legs are
	// tagged for a DIAGONAL TROT: FL+BR swing together (phase 0), FR+BL together (phase π); each swings around its hip.
	const PI = Math.PI;
	const buildLion = () =>
		merge([
			P('box', [0.6, 0.5, 1.3], [0, 0.52, 0]), // body
			P('sphere', [1.0, 0.95, 0.82], [0, 0.03, -0.22], [0, 0, 0], [0, 0.66, 0.82]), // mane
			P('sphere', [0.5, 0.5, 0.5], [0, 0, 0], [0, 0, 0], [0, 0.66, 0.82]), // head
			P('sphere', [0.26, 0.22, 0.3], [0, -0.04, 0.28], [0, 0, 0], [0, 0.66, 0.82]), // muzzle
			P('sphere', [0.08, 0.06, 0.08], [0, -0.05, 0.42], [0, 0, 0], [0, 0.66, 0.82]), // nose
			P('cone', [0.13, 0.16, 0.13], [0.18, 0.34, 0], [0, 0, 0], [0, 0.66, 0.82]), // ears
			P('cone', [0.13, 0.16, 0.13], [-0.18, 0.34, 0], [0, 0, 0], [0, 0.66, 0.82]),
			P('cyl', [0.07, 0.7, 0.07], [0, 0.3, 0], [0, 0, 0], [0, 0.68, -0.78], [0.8, 0, 0]), // tail
			P('sphere', [0.18, 0.2, 0.18], [0, 0.62, 0], [0, 0, 0], [0, 0.68, -0.78], [0.8, 0, 0]),
			P('box', [0.17, 0.42, 0.17], [0, -0.21, 0], [0, 0, 0], [0.22, 0.4, 0.46], [0, 0, 0], { ph: PI, hipY: 0.4, hipZ: 0.46 }), // FR
			P('box', [0.17, 0.42, 0.17], [0, -0.21, 0], [0, 0, 0], [-0.22, 0.4, 0.46], [0, 0, 0], { ph: 0, hipY: 0.4, hipZ: 0.46 }), // FL
			P('box', [0.17, 0.42, 0.17], [0, -0.21, 0], [0, 0, 0], [0.22, 0.4, -0.46], [0, 0, 0], { ph: 0, hipY: 0.4, hipZ: -0.46 }), // BR
			P('box', [0.17, 0.42, 0.17], [0, -0.21, 0], [0, 0, 0], [-0.22, 0.4, -0.46], [0, 0, 0], { ph: PI, hipY: 0.4, hipZ: -0.46 }) // BL
		]);
	// humanBody — the EXACT Npc.svelte model (NPC.* parts), group transforms flattened in like the animal builders.
	// 4 swinging limbs tagged as legs (CONTRALATERAL: each arm shares its OPPOSITE leg's phase → arm swings against
	// its same-side leg). part tags the per-instance colour: torso/arms=shirt(0), legs=pants(1), head=skin(2),
	// hair=hair(3). The hair is `mane`-tagged → it collapses into the head for MALES (aMale=1). Eyes are omitted
	// (a separate dark material instancing can't carry — same as the animal builders).
	const buildHuman = () =>
		merge([
			P('cyl', [0.52, 0.85, 0.52], [0, 1.05, 0], [0, 0, 0], [0, 0, 0], [0, 0, 0], undefined, false, 0), // torso = SHIRT
			P('sphere', [0.48, 0.48, 0.48], [0, 1.62, 0], [0, 0, 0], [0, 0, 0], [0, 0, 0], undefined, false, 2), // head = SKIN
			P('box', [0.18, 0.7, 0.18], [0, -0.35, 0], [0, 0, 0], [0.14, 0.7, 0], [0, 0, 0], { ph: 0, hipY: 0.7, hipZ: 0 }, false, 1), // legL = PANTS
			P('box', [0.18, 0.7, 0.18], [0, -0.35, 0], [0, 0, 0], [-0.14, 0.7, 0], [0, 0, 0], { ph: PI, hipY: 0.7, hipZ: 0 }, false, 1), // legR
			P('box', [0.12, 0.62, 0.12], [0, -0.3, 0], [0, 0, 0], [0.34, 1.4, 0], [0, 0, 0], { ph: PI, hipY: 1.4, hipZ: 0 }, false, 0), // armL = SHIRT, opposite legL
			P('box', [0.12, 0.62, 0.12], [0, -0.3, 0], [0, 0, 0], [-0.34, 1.4, 0], [0, 0, 0], { ph: 0, hipY: 1.4, hipZ: 0 }, false, 0), // armR, opposite legR
			P('sphere', [0.46, 0.42, 0.46], [0, 1.67, -0.04], [0, 0, 0], [0, 0, 0], [0, 0, 0], undefined, true, 3), // hair crown (head-rel y0.05) = HAIR
			P('sphere', [0.17, 0.4, 0.2], [0.27, 1.4, -0.04], [0, 0, 0], [0, 0, 0], [0, 0, 0], undefined, true, 3), // hair side-lock (head-rel y-0.22)
			P('sphere', [0.17, 0.4, 0.2], [-0.27, 1.4, -0.04], [0, 0, 0], [0, 0, 0], [0, 0, 0], undefined, true, 3)
		]);
	// GAIT (the begin_vertex displacement) — a whole-body BOUND: an arc up + a forward nose-dip on every stride.
	const BOUND = (amp: number, lean: number) => /* glsl */ `
		float arc = sin(fract(aPhase) * 3.14159);
		transformed.y += aHop * arc * ${amp.toFixed(2)};
		float lean = aHop * arc * ${lean.toFixed(2)};
		float cl = cos(lean), sl = sin(lean);
		transformed.yz = mat2(cl, sl, -sl, cl) * transformed.yz;`;
	// WALK/TROT (quadruped): the BODY stays level (no bob — a bob flutters into a jitter at chase speed); only each LEG
	// swings forward/back around its own hip by the stride phase (tagged diagonal pairs out of phase) → the legs move,
	// the body glides, no skating (the cat bug) and no jumping. `mane` verts collapse to the head for females (aMale=0).
	const WALK = (swing: number) => /* glsl */ `
		float ph = fract(aPhase) * 6.28318;
		if (aLegPh < 50.0) {
			float ang = aHop * ${swing.toFixed(2)} * sin(ph + aLegPh);
			float ry = transformed.y - aHipY, rz = transformed.z - aHipZ;
			float cs = cos(ang), sn = sin(ang);
			transformed.y = aHipY + ry * cs - rz * sn;
			transformed.z = aHipZ + ry * sn + rz * cs;
		}
		if (aMane > 0.5) transformed = mix(vec3(0.0, 0.69, 0.6), transformed, aMale);`;

	const makeMat = (gait: string) => {
		const m = new THREE.MeshStandardMaterial({ color: 0xffffff, flatShading: true });
		m.onBeforeCompile = (shader) => {
			shader.uniforms.uNight = creatureNight;
			shader.vertexShader = shader.vertexShader
				.replace('#include <common>', '#include <common>\nattribute float aHop;\nattribute float aPhase;\nattribute float aLegPh;\nattribute float aHipY;\nattribute float aHipZ;\nattribute float aMane;\nattribute float aMale;\nvarying vec3 vCreatureN;\nvarying vec3 vCreatureP;')
				.replace('#include <begin_vertex>', `#include <begin_vertex>\nvCreatureN = normalize(mat3(modelMatrix) * normal);\nvCreatureP = position;${gait}`);
			shader.fragmentShader = shader.fragmentShader
				.replace(
					'#include <common>',
					/* glsl */ `#include <common>
					uniform float uNight;
					varying vec3 vCreatureN;
					varying vec3 vCreatureP;
					float crHash(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
					float crNoise(vec2 p){ vec2 i = floor(p), f = fract(p); float a = crHash(i), b = crHash(i + vec2(1.0, 0.0)), c = crHash(i + vec2(0.0, 1.0)), d = crHash(i + vec2(1.0, 1.0)); vec2 u = f * f * (3.0 - 2.0 * f); return mix(mix(a, b, u.x), mix(c, d, u.x), u.y); }`
				)
				.replace(
					'#include <color_fragment>',
					/* glsl */ `#include <color_fragment>
					float crAO = 0.72 + 0.28 * (vCreatureN.y * 0.5 + 0.5);
					diffuseColor.rgb *= crAO * (0.93 + 0.12 * crNoise(vCreatureP.xz * 5.0 + vCreatureP.y * 2.0));
					diffuseColor.rgb *= 1.0 + 0.16 * smoothstep(0.1, -0.5, vCreatureN.y);`
				)
				.replace(
					'#include <emissivemap_fragment>',
					/* glsl */ `#include <emissivemap_fragment>
					float crRim = pow(1.0 - clamp(dot(normalize(normal), normalize(vViewPosition)), 0.0, 1.0), 2.5);
					totalEmissiveRadiance += vec3(0.40, 0.50, 0.72) * (crRim * uNight * 0.55);`
				);
		};
		return m;
	};

	// WALK_HUMAN — the biped gait: same leg-swing-around-the-hip as WALK, but the `mane` (hair) verts collapse into
	// the HEAD (y≈1.62) for MALES so they read bare-headed (females aMale=0 → keep the hair; males aMale=1 → collapse).
	const WALK_HUMAN = (swing: number) => /* glsl */ `
		float ph = fract(aPhase) * 6.28318;
		if (aLegPh < 50.0) {
			float ang = aHop * ${swing.toFixed(2)} * sin(ph + aLegPh);
			float ry = transformed.y - aHipY, rz = transformed.z - aHipZ;
			float cs = cos(ang), sn = sin(ang);
			transformed.y = aHipY + ry * cs - rz * sn;
			transformed.z = aHipZ + ry * sn + rz * cs;
		}
		if (aMane > 0.5) transformed = mix(vec3(0.0, 1.62, 0.0), transformed, 1.0 - aMale);`;

	// makeHumanMat — like makeMat, but each vertex is tinted by its PART's per-instance colour (shirt/pants/skin/hair)
	// instead of one whole-instance tint: the vertex picks vBase from aPart + the four per-instance vec3 attributes and
	// the fragment multiplies the white diffuse by it. The AO/noise/night-rim recipe is otherwise identical.
	const makeHumanMat = (gait: string) => {
		const m = new THREE.MeshStandardMaterial({ color: 0xffffff, flatShading: true });
		m.onBeforeCompile = (shader) => {
			shader.uniforms.uNight = creatureNight;
			shader.vertexShader = shader.vertexShader
				.replace(
					'#include <common>',
					'#include <common>\nattribute float aHop;\nattribute float aPhase;\nattribute float aLegPh;\nattribute float aHipY;\nattribute float aHipZ;\nattribute float aMane;\nattribute float aMale;\nattribute float aPart;\nattribute vec4 aCol4;\nvarying vec3 vCreatureN;\nvarying vec3 vCreatureP;\nvarying vec3 vBase;\nvec3 unpackCol(float f){ return vec3(floor(f / 65536.0), floor(mod(f, 65536.0) / 256.0), mod(f, 256.0)) / 255.0; }'
				)
				.replace(
					'#include <begin_vertex>',
					`#include <begin_vertex>\nvCreatureN = normalize(mat3(modelMatrix) * normal);\nvCreatureP = position;\nvBase = unpackCol(aPart < 0.5 ? aCol4.x : aPart < 1.5 ? aCol4.y : aPart < 2.5 ? aCol4.z : aCol4.w);${gait}`
				);
			shader.fragmentShader = shader.fragmentShader
				.replace(
					'#include <common>',
					/* glsl */ `#include <common>
					uniform float uNight;
					varying vec3 vCreatureN;
					varying vec3 vCreatureP;
					varying vec3 vBase;
					float crHash(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
					float crNoise(vec2 p){ vec2 i = floor(p), f = fract(p); float a = crHash(i), b = crHash(i + vec2(1.0, 0.0)), c = crHash(i + vec2(0.0, 1.0)), d = crHash(i + vec2(1.0, 1.0)); vec2 u = f * f * (3.0 - 2.0 * f); return mix(mix(a, b, u.x), mix(c, d, u.x), u.y); }`
				)
				.replace(
					'#include <color_fragment>',
					/* glsl */ `#include <color_fragment>
					diffuseColor.rgb *= vBase;
					float crAO = 0.72 + 0.28 * (vCreatureN.y * 0.5 + 0.5);
					diffuseColor.rgb *= crAO * (0.93 + 0.12 * crNoise(vCreatureP.xz * 5.0 + vCreatureP.y * 2.0));
					diffuseColor.rgb *= 1.0 + 0.16 * smoothstep(0.1, -0.5, vCreatureN.y);`
				)
				.replace(
					'#include <emissivemap_fragment>',
					/* glsl */ `#include <emissivemap_fragment>
					float crRim = pow(1.0 - clamp(dot(normalize(normal), normalize(vViewPosition)), 0.0, 1.0), 2.5);
					totalEmissiveRadiance += vec3(0.40, 0.50, 0.72) * (crRim * uNight * 0.55);`
				);
		};
		return m;
	};

	// per-species: kind · geometry · gait shader · stride RATE (hop cycles per metre travelled) · fallback tone.
	type Sp = { kind: string; geo: THREE.BufferGeometry; gait: string; rate: number; tone: string };
	// ONLY the HOPPERS — their whole-body bound reads perfectly instanced. Trotters (cat/lion/dino) stay articulated
	// Critters (a trot needs real legs to frolic; predators are rare → instancing them costs the look for no perf).
	const SPECIES: Sp[] = [
		{ kind: 'rabbit', geo: buildRabbit(), gait: BOUND(0.75, 0.36), rate: 0.7, tone: '#eceae3' },
		{ kind: 'kangaroo', geo: buildKangaroo(), gait: BOUND(1.0, 0.3), rate: 0.45, tone: '#b07a4a' } // big, long bounds
		// LION REVERTED to the articulated Critter (2026-06-27): the instanced WALK gait read as a persistent vertical
		// JUMP that I couldn't pin (body is level in the shader + grounding is smooth, yet it jumped "like hell"). The
		// articulated trot is proven (same as the cat), so the lion draws via Critter again until the cause is found.
	];

	type Bank = { kind: string; mesh: THREE.InstancedMesh; aHop: Float32Array; aPhase: Float32Array; aMale: Float32Array; hop: Map<number, { px: number; pz: number; ph: number }>; count: number };
	const banks: Bank[] = SPECIES.map(({ kind, geo, gait, tone }) => {
		const mesh = new THREE.InstancedMesh(geo, makeMat(gait), MAX);
		mesh.castShadow = false;
		mesh.receiveShadow = false;
		mesh.frustumCulled = false;
		mesh.count = 0;
		mesh.userData.tone = tone;
		const aHop = new Float32Array(MAX);
		const aPhase = new Float32Array(MAX);
		const aMale = new Float32Array(MAX);
		geo.setAttribute('aHop', new THREE.InstancedBufferAttribute(aHop, 1));
		geo.setAttribute('aPhase', new THREE.InstancedBufferAttribute(aPhase, 1));
		geo.setAttribute('aMale', new THREE.InstancedBufferAttribute(aMale, 1));
		return { kind, mesh, aHop, aPhase, aMale, hop: new Map(), count: 0 };
	});
	const byKind = new Map(banks.map((b) => [b.kind, b]));
	const rate = new Map(SPECIES.map((s) => [s.kind, s.rate]));
	const dummy = new THREE.Object3D();

	// ── the PERSON bank — a HYBRID with the articulated Npc: this instanced biped draws a person only while NORMAL
	//    (walking/idle); the moment they enter a nuanced state (corpse / asleep / pregnant belly / guardian machete /
	//    drinking crouch — or a companion) the Npc takes over (it filters on the SAME predicate, so exactly one draws).
	//    It diverges from the animal banks: instead of ONE instanceColor tint it carries FOUR per-instance tints packed
	//    into one vec4 (shirt/pants/skin/hair) the part-tinted material reads, and people are radius=0.4·scale (so `radius/0.4` recovers
	//    size, vs the animals' 0.35). Kept as a small dedicated bank + loop rather than forced into the animal path.
	const PERSON_RATE = 1.0; // stride phase per metre travelled → a natural walking cadence
	const personGeo = buildHuman();
	const personMat = makeHumanMat(WALK_HUMAN(0.5)); // 0.5 amp = the Npc's contralateral leg/arm swing
	const personMesh = new THREE.InstancedMesh(personGeo, personMat, MAX);
	personMesh.castShadow = false;
	personMesh.receiveShadow = false;
	personMesh.frustumCulled = false;
	personMesh.count = 0;
	const pHop = new Float32Array(MAX);
	const pPhase = new Float32Array(MAX);
	const pMale = new Float32Array(MAX);
	// FOUR per-part tints PACKED into ONE vec4 (x=shirt y=pants z=skin w=hair), each component a bit-packed RGB float
	// (r·65536+g·256+b, 8-bit/channel → exact in float32). WHY: four separate vec3 colour attributes pushed the person
	// mesh to 18 vertex attributes — OVER the GPU's 16 `MAX_VERTEX_ATTRIBS` limit, so the shader failed to link ("Too
	// many attributes (aSkin)") and NO humans drew at all. Packing 4 slots → 1 brings it to 15 (safe margin). Per-part
	// tints are SOLID (no gradient within a part), so 8-bit quantisation is imperceptible here.
	const pCol4 = new Float32Array(MAX * 4);
	personGeo.setAttribute('aHop', new THREE.InstancedBufferAttribute(pHop, 1));
	personGeo.setAttribute('aPhase', new THREE.InstancedBufferAttribute(pPhase, 1));
	personGeo.setAttribute('aMale', new THREE.InstancedBufferAttribute(pMale, 1));
	personGeo.setAttribute('aCol4', new THREE.InstancedBufferAttribute(pCol4, 4));
	const personHopState = new Map<number, { px: number; pz: number; ph: number }>();
	// sensible fallbacks when a person's colour is undefined (e.g. one frame before the Npc's $effect stamps it).
	const P_FALLBACK = { shirt: '#4a73c4', skin: '#e8b894', pants: '#3a3a42', hair: '#3a2817' };
	// a NORMAL person is drawn here; the rest (companion/corpse/asleep/pregnant/guardian/drinking) draw via the Npc.
	const personNormal = (m: ManagedAgent) => !m.companion && !m.dead && !m.asleep && !m.pregnant && !m.guardian && !m.drinking;
	// parsed-colour cache: setColorAt is called per agent per frame, but tints almost never change and parsing a hex
	// string (`new Color('#eceae3')`) every time is pure CPU churn. Cache by string → ~3 parses total, not ~250/frame.
	const colorCache = new Map<string, THREE.Color>();
	const tintColor = (key: string) => {
		let c = colorCache.get(key);
		if (!c) colorCache.set(key, (c = new THREE.Color(key)));
		return c;
	};
	// per-part tint PACKER for the person bank — parses each hex ONCE (cached, like tintColor) then BIT-PACKS the linear
	// rgb into a single float (r·65536+g·256+b, 8-bit/channel) written into aCol4 at instance i, component `comp`
	// (0=x/shirt 1=y/pants 2=z/skin 3=w/hair). Avoids re-parsing four hex strings per person per frame; the shader
	// unpacks via unpackCol(). Linear round-trips correctly (diffuseColor is linear working space).
	const packPart = (i: number, comp: number, key: string) => {
		const c = tintColor(key);
		pCol4[i * 4 + comp] = Math.round(c.r * 255) * 65536 + Math.round(c.g * 255) * 256 + Math.round(c.b * 255);
	};

	// JUVENILE GROWTH (matches Npc/Critter BABY_SCALE=0.45): a newborn is born small and grows to adult size by the
	// sim's maturity age (JUVENILE_FRAC ≈ 0.12 of life). The instanced banks read it off ageFrac (mirrored from the
	// sim each frame) so a baby drawn HERE scales up like the articulated one. WITHOUT this an instanced newborn drew
	// at full adult size — the articulated growth only scaled the (hidden) Npc/Critter mesh (user: "a pregnant woman
	// gave birth to a full-grown woman"). Adults (ageFrac ≥ 0.12, or unset) → 1; a juvenile with ageFrac not yet
	// mirrored falls back to 0 (baby) so there's no full-size flash on the first frames.
	const BABY_SCALE = 0.45;
	const MATURE_FRAC = 0.12;
	const growthOf = (m: ManagedAgent) => {
		const af = m.ageFrac ?? (m.juvenile ? 0 : 1);
		return BABY_SCALE + (1 - BABY_SCALE) * Math.min(1, af / MATURE_FRAC);
	};

	useTask(() => {
		for (const b of banks) b.count = 0; // running per-bank fill index (was a fresh Map + per-agent string-hash each frame)
		let pi = 0; // running fill index for the PERSON bank (separate from the animal banks)
		agentManager.forEach((m) => {
			// PEOPLE: a NORMAL walker draws in the dedicated instanced human bank; nuanced states fall to the Npc.
			if (m.kind === 'person') {
				if (m.lod === 2 || !personNormal(m) || pi >= MAX) return; // far → impostor; nuanced → articulated Npc
				const a = m.agent;
				a.interpolate(clock.alpha);
				let h = personHopState.get(m.seedId);
				if (!h) ((h = { px: a.rx, pz: a.rz, ph: (m.seedId & 255) * 0.013 }), personHopState.set(m.seedId, h));
				h.ph += Math.hypot(a.rx - h.px, a.rz - h.pz) * PERSON_RATE;
				h.px = a.rx;
				h.pz = a.rz;
				dummy.position.set(a.rx, groundYCached(m, a.rx, a.rz, world.terrain), a.rz);
				dummy.rotation.set(0, a.rh, 0); // never a corpse here (corpses draw via the Npc)
				dummy.scale.setScalar((m.radius / 0.4) * growthOf(m)); // people are radius=0.4·scale; × juvenile growth
				dummy.updateMatrix();
				personMesh.setMatrixAt(pi, dummy.matrix);
				packPart(pi, 0, m.tint ?? P_FALLBACK.shirt); // x = shirt
				packPart(pi, 1, m.pants ?? P_FALLBACK.pants); // y = pants
				packPart(pi, 2, m.skin ?? P_FALLBACK.skin); // z = skin
				packPart(pi, 3, m.hair ?? P_FALLBACK.hair); // w = hair
				pHop[pi] = Math.min(1, a.gaitRate() * 2.2);
				pPhase[pi] = h.ph;
				pMale[pi] = m.seedId & 1; // odd seed = male → the hair collapses into the head (females keep it)
				pi++;
				return;
			}
			const b = byKind.get(m.kind);
			if (!b || m.lod === 2 || m.companion) return; // far → impostor; companion keeps its articulated Critter
			const i = b.count;
			if (i >= MAX) return;
			const a = m.agent;
			a.interpolate(clock.alpha);
			let h = b.hop.get(m.seedId);
			if (!h) ((h = { px: a.rx, pz: a.rz, ph: (m.seedId & 255) * 0.013 }), b.hop.set(m.seedId, h));
			h.ph += Math.hypot(a.rx - h.px, a.rz - h.pz) * rate.get(m.kind)!;
			h.px = a.rx;
			h.pz = a.rz;
			dummy.position.set(a.rx, groundYCached(m, a.rx, a.rz, world.terrain), a.rz);
			dummy.rotation.set(0, a.rh, m.dead ? Math.PI / 2 : 0);
			dummy.scale.setScalar((m.radius / 0.35) * growthOf(m)); // × juvenile growth (instanced babies grow up too)
			dummy.updateMatrix();
			b.mesh.setMatrixAt(i, dummy.matrix);
			b.mesh.setColorAt(i, tintColor(m.tint ?? (b.mesh.userData.tone as string)));
			b.aHop[i] = m.dead || m.asleep ? 0 : Math.min(1, a.gaitRate() * 2.2);
			b.aPhase[i] = h.ph;
			b.aMale[i] = m.seedId & 1; // odd seed = male (matches the Critter's `female = (seedId & 1) === 0`) → mane only on males
			b.count = i + 1;
		});
		for (const b of banks) {
			b.mesh.count = b.count;
			b.mesh.instanceMatrix.needsUpdate = true;
			if (b.mesh.instanceColor) b.mesh.instanceColor.needsUpdate = true;
			(b.mesh.geometry.getAttribute('aHop') as THREE.InstancedBufferAttribute).needsUpdate = true;
			(b.mesh.geometry.getAttribute('aPhase') as THREE.InstancedBufferAttribute).needsUpdate = true;
			(b.mesh.geometry.getAttribute('aMale') as THREE.InstancedBufferAttribute).needsUpdate = true;
		}
		// flush the PERSON bank — its four per-instance colour attributes need an upload too (no instanceColor here)
		personMesh.count = pi;
		personMesh.instanceMatrix.needsUpdate = true;
		for (const name of ['aHop', 'aPhase', 'aMale', 'aCol4']) {
			(personGeo.getAttribute(name) as THREE.InstancedBufferAttribute).needsUpdate = true;
		}
	});
</script>

{#each banks as b (b.kind)}
	<T is={b.mesh} />
{/each}
<T is={personMesh} />
