<script lang="ts">
	// A placed building (house / cabin / tower) with PROCEDURAL WINDOWS — a shader, no textures (the
	// shader-first direction). The wall part (parts[0]) gets a window grid tiled across its faces (box) or
	// around its circumference (cylinder); windows are dark glass by day and glow warm at night (uNight,
	// from the sky), a random ~half of them lit. Roof/door parts stay plain. Scene routes house/cabin/tower
	// here; everything else stays on Prop. Pop-in matches Prop.
	import { untrack } from 'svelte';
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { kindDef } from '$lib/kinds';
	import { partGeo, litMat } from '$lib/sharedAssets';
	import { weather } from '$lib/weather';
	import type { World, WorldObject } from '$lib/world';

	let { obj, world }: { obj: WorldObject; world: World } = $props();
	// snapshot the static build inputs once; read pos/rot REACTIVELY in the markup so move ops relocate it
	const O = untrack(() => ({ kind: obj.kind, color: obj.color, sx: obj.scale?.[0] ?? 1, sy: obj.scale?.[1] ?? 1, sz: obj.scale?.[2] ?? 1 }));
	const rot = $derived(((obj.rot ?? 0) * Math.PI) / 180);
	const def = kindDef(O.kind);

	// uNight: 0 (full day) → 1 (windows fully lit). Reactive on the sky.
	const NIGHT: Record<string, number> = { day: 0, sunset: 0.5, fog: 0.18, night: 1, space: 1 };
	const uNight = { value: 0 };
	// uWet: 1 under the rainy 'fog' sky → walls + roof darken and pick up a grazing wet sheen, so a building
	// gets soaked in the rain like the ground + roads already do (same emissive-Fresnel trick as Path/Terrain).
	const uWet = { value: 0 };
	$effect(() => {
		uNight.value = NIGHT[world.sky] ?? 0;
		uWet.value = world.sky === 'fog' ? 1 : 0;
	});

	function wallMat(color: string, halfY: number, cyl: boolean): THREE.MeshStandardMaterial {
		const m = new THREE.MeshStandardMaterial({ color, flatShading: true });
		m.onBeforeCompile = (shader) => {
			shader.uniforms.uNight = uNight;
			shader.uniforms.uWet = uWet;
			shader.uniforms.uSnow = weather.uSnow;
			shader.uniforms.uHalfY = { value: halfY };
			shader.uniforms.uCyl = { value: cyl ? 1 : 0 };
			shader.vertexShader = shader.vertexShader
				.replace('#include <common>', '#include <common>\nvarying vec3 vLocalPos;\nvarying vec3 vNrm;\nvarying vec3 vWPos;\nvarying vec3 vWN;')
				.replace('#include <begin_vertex>', '#include <begin_vertex>\nvLocalPos = position;\nvNrm = normal;\nvWPos = (modelMatrix * vec4(position, 1.0)).xyz;\nvWN = normalize(mat3(modelMatrix) * normal);');
			shader.fragmentShader = shader.fragmentShader
				.replace(
					'#include <common>',
					/* glsl */ `#include <common>
					varying vec3 vLocalPos;
					varying vec3 vNrm;
					varying vec3 vWPos;
					varying vec3 vWN;
					uniform float uNight;
					uniform float uWet;
					uniform float uSnow;
					uniform float uHalfY;
					uniform float uCyl;
					float bHash(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
					float winMask(out float litOut, out float horizOut){
						float side = 1.0 - step(0.5, abs(vNrm.y));          // 0 on roof/floor faces
						float horiz = uCyl > 0.5
							? atan(vLocalPos.x, vLocalPos.z) * length(vLocalPos.xz) // arc length around the tower
							: (abs(vNrm.x) > abs(vNrm.z) ? vLocalPos.z : vLocalPos.x); // along whichever wall we're on
						horizOut = horiz;
						vec2 cell = vec2(horiz / 1.15, vLocalPos.y / 1.05);
						vec2 fc = fract(cell + 0.5) - 0.5;
						float win = step(abs(fc.x), 0.32) * step(abs(fc.y), 0.36);
						float band = step(-uHalfY + 0.45, vLocalPos.y) * step(vLocalPos.y, uHalfY - 0.35); // off ground/eaves
						win *= side * band;
						litOut = step(0.45, bHash(floor(cell + 0.5) + vNrm.xz * 7.3)); // ~55% of windows lit at night
						return win;
					}`
				)
				.replace(
					'#include <color_fragment>',
					/* glsl */ `#include <color_fragment>
					float winLit;
					float wHoriz;
					float win = winMask(winLit, wHoriz);
					// TOWER doorway (cyl walls only — box houses/cabins use the panelled door PART): an arched stone
					// opening at the base front (the +Z arc, where wHoriz≈0). Windows are suppressed inside it; it's
					// carved dark after the glass below.
					float door = 0.0;
					if (uCyl > 0.5) {
						float dy = vLocalPos.y + uHalfY;                                     // height above the foot
						float dSide = 1.0 - step(0.5, abs(vNrm.y));                           // wall faces only
						float rect = step(abs(wHoriz), 0.5) * step(0.0, dy) * step(dy, 1.8);  // door body
						float arch = step(length(vec2(wHoriz, dy - 1.8)), 0.5) * step(1.8, dy); // rounded top
						door = max(rect, arch) * dSide;
						win *= 1.0 - door;                                                   // no window/glow in the doorway
					}
					// GLASS: each pane reflects the sky — a vertical gradient (dark sill → bright sky at the top of
					// the pane) plus a Fresnel grazing GLINT off the wall, so windows catch the light and read as
					// glass instead of flat black holes. (The ~55% lit ones still glow warm at night, added below.)
					vec3 vView = normalize(cameraPosition - vWPos);
					float winFres = pow(1.0 - clamp(dot(normalize(vWN), vView), 0.0, 1.0), 4.0);
					float paneY = fract(vLocalPos.y / 1.05 + 0.5) - 0.5;                  // -0.5 sill .. 0.5 pane top
					vec3 glass = mix(vec3(0.06, 0.09, 0.14), vec3(0.40, 0.50, 0.64), clamp(paneY + 0.5, 0.0, 1.0));
					glass = mix(glass, vec3(0.62, 0.72, 0.85), winFres * 0.7);           // grazing-angle sky sheen
					glass *= 1.0 - 0.5 * uNight;                                         // reflects a darker sky after dusk
					diffuseColor.rgb = mix(diffuseColor.rgb, glass, win);
					diffuseColor.rgb = mix(diffuseColor.rgb, vec3(0.09, 0.07, 0.10), door); // dark recessed tower doorway
					// WEATHERING: contact grime + AO creeping up from the ground (base darker), broken into faint
					// vertical streaks per wall column, so buildings read as planted & aged, not clean floating boxes.
					float wSide = 1.0 - step(0.5, abs(vNrm.y));                          // walls only (not roof/floor faces)
					float baseAO = smoothstep(-uHalfY + 1.8, -uHalfY + 0.1, vLocalPos.y); // 0 up high → 1 down at the footing
					float streak = 0.55 + 0.45 * bHash(vec2(floor(wHoriz * 2.5), 3.0));   // dirtier in some columns
					float grime = baseAO * wSide * streak;
					diffuseColor.rgb *= 1.0 - 0.34 * grime;                              // darken toward the base
					diffuseColor.rgb = mix(diffuseColor.rgb, diffuseColor.rgb * vec3(0.92, 0.89, 0.82), grime * 0.6); // earthy tone
					diffuseColor.rgb *= 1.0 - 0.26 * uWet;                               // rain-soaked walls darken
					diffuseColor.rgb = mix(diffuseColor.rgb, vec3(0.93, 0.95, 0.99), uSnow * smoothstep(0.42, 0.74, vWN.y)); // snow on ledges/sills`
				)
				.replace(
					'#include <emissivemap_fragment>',
					/* glsl */ `#include <emissivemap_fragment>
					totalEmissiveRadiance += win * winLit * uNight * vec3(1.0, 0.8, 0.45) * 1.7; // warm glow at night
					if (uWet > 0.01) {
						// wet sheen: the rain-slicked wall mirrors the overcast sky at grazing angles, added as EMISSIVE
						// so it survives the weak rain sun (same trick as Path/Terrain's wet ground)
						float wFres = pow(1.0 - clamp(dot(normalize(vWN), normalize(cameraPosition - vWPos)), 0.0, 1.0), 4.0);
						totalEmissiveRadiance += uWet * wFres * vec3(0.34, 0.38, 0.45) * 0.5;
					}`
				);
		};
		return m;
	}

	// procedural SHINGLE roof — overlapping tile rows up the slope with a shadow lip at each row's bottom
	// edge, staggered (brick-bond) columns with seam gaps, and per-tile tone variation. A shader, no textures.
	function roofMat(color: string): THREE.MeshStandardMaterial {
		const m = new THREE.MeshStandardMaterial({ color, flatShading: true });
		m.onBeforeCompile = (shader) => {
			shader.uniforms.uWet = uWet;
			shader.uniforms.uSnow = weather.uSnow;
			shader.vertexShader = shader.vertexShader
				.replace('#include <common>', '#include <common>\nvarying vec3 vRoofP;\nvarying vec3 vRoofWP;\nvarying vec3 vRoofWN;')
				.replace('#include <begin_vertex>', '#include <begin_vertex>\nvRoofP = position;\nvRoofWP = (modelMatrix * vec4(position, 1.0)).xyz;\nvRoofWN = normalize(mat3(modelMatrix) * normal);');
			shader.fragmentShader = shader.fragmentShader
				.replace(
					'#include <common>',
					/* glsl */ `#include <common>
					varying vec3 vRoofP;
					varying vec3 vRoofWP;
					varying vec3 vRoofWN;
					uniform float uWet;
					uniform float uSnow;
					float rfHash(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }`
				)
				.replace(
					'#include <color_fragment>',
					/* glsl */ `#include <color_fragment>
					float row = vRoofP.y * 3.2;                              // shingle rows climbing the slope
					float rowF = fract(row);
					float col = atan(vRoofP.x, vRoofP.z) * 3.2;              // tiles around the roof
					col += step(0.5, fract(row * 0.5)) * 0.5;                // stagger alternate rows (brick bond)
					float colF = fract(col);
					float lip = smoothstep(0.0, 0.16, rowF);                 // dark shadow at each shingle's lower edge
					float seam = smoothstep(0.045, 0.0, min(colF, 1.0 - colF)); // gaps between tiles
					float tone = 0.84 + 0.30 * rfHash(floor(vec2(col, row)));   // each tile a touch different
					diffuseColor.rgb *= (0.62 + 0.38 * lip) * tone;
					diffuseColor.rgb *= 1.0 - 0.25 * seam;
					diffuseColor.rgb *= 1.0 - 0.3 * uWet;                    // rain-soaked shingles darken
					diffuseColor.rgb = mix(diffuseColor.rgb, vec3(0.94, 0.96, 1.0), uSnow * smoothstep(0.3, 0.7, vRoofWN.y)); // snow blankets the roof`
				)
				.replace(
					'#include <emissivemap_fragment>',
					/* glsl */ `#include <emissivemap_fragment>
					if (uWet > 0.01) {
						// wet roof glistens: mirrors the overcast sky at grazing angles (emissive so it survives the
						// weak rain sun) — the classic rain-slicked rooftop, matching Path/Terrain's wet sheen
						float rFres = pow(1.0 - clamp(dot(normalize(vRoofWN), normalize(cameraPosition - vRoofWP)), 0.0, 1.0), 4.0);
						totalEmissiveRadiance += uWet * rFres * vec3(0.34, 0.38, 0.45) * 0.6;
					}`
				);
		};
		return m;
	}

	// procedural PANELLED door — on the outward face (local +Z) a raised frame + mid-rail bound two recessed
	// panels, with a brass knob. uHW/uHH are the door's half-extents so the layout is right at any door size.
	function doorMat(color: string, hw: number, hh: number): THREE.MeshStandardMaterial {
		const m = new THREE.MeshStandardMaterial({ color, flatShading: true });
		m.onBeforeCompile = (shader) => {
			shader.uniforms.uHW = { value: hw };
			shader.uniforms.uHH = { value: hh };
			shader.vertexShader = shader.vertexShader
				.replace('#include <common>', '#include <common>\nvarying vec3 vDoorP;\nvarying vec3 vDoorN;')
				.replace('#include <begin_vertex>', '#include <begin_vertex>\nvDoorP = position;\nvDoorN = normal;');
			shader.fragmentShader = shader.fragmentShader
				.replace(
					'#include <common>',
					/* glsl */ `#include <common>
					varying vec3 vDoorP;
					varying vec3 vDoorN;
					uniform float uHW;
					uniform float uHH;`
				)
				.replace(
					'#include <color_fragment>',
					/* glsl */ `#include <color_fragment>
					if (vDoorN.z > 0.5) {                                    // the outward-facing front of the door
						float nx = vDoorP.x / uHW;                           // -1..1 across
						float ny = vDoorP.y / uHH;                           // -1..1 up
						float frame = step(0.80, max(abs(nx), abs(ny)));     // raised outer frame
						float rail = 1.0 - step(0.10, abs(ny));              // horizontal mid-rail between the panels
						float panel = (1.0 - frame) * (1.0 - rail);          // the two recessed panes
						diffuseColor.rgb *= 1.0 - 0.32 * panel;              // inset → darker
						float knob = step(length(vec2(nx - 0.62, ny - 0.05)), 0.09);
						diffuseColor.rgb = mix(diffuseColor.rgb, vec3(0.74, 0.6, 0.26), knob); // brass knob
					}`
				);
		};
		return m;
	}

	const parts = untrack(() =>
		def.parts.map((p, i) => {
			const isWall = i === 0 && (p.geo === 'box' || p.geo === 'cyl');
			const isRoof = p.geo === 'pyramid' || p.geo === 'cone';
			const isDoor = !isWall && p.geo === 'box'; // the only non-wall box part is the door (house/cabin); towers have none
			// paint tints the WALLS only; roof/door keep their own colours (realistic, and avoids mutating the
			// shared litMat cache). Wall + roof + door use OWN materials (wall recolours reactively below).
			return {
				geo: partGeo(p),
				pos: p.pos,
				mat: isWall
					? wallMat(obj.color ?? p.color, p.args[1] / 2, p.geo === 'cyl')
					: isRoof
						? roofMat(p.color)
						: isDoor
							? doorMat(p.color, p.args[0] / 2, p.args[1] / 2)
							: litMat(p.color, p.emissive),
				isWall,
				base: p.color
			};
		})
	);
	$effect(() => {
		const c = obj.color; // react to paint ("make the house blue") → re-tint the walls
		for (const part of parts) if (part.isWall) part.mat.color.set(c ?? part.base);
	});

	// pop-in (easeOutBack), same as Prop
	let model = $state<THREE.Group>();
	let t = 0;
	const eob = (x: number) => {
		const c1 = 1.70158;
		const c3 = c1 + 1;
		return 1 + c3 * (x - 1) ** 3 + c1 * (x - 1) ** 2;
	};
	// settle, then STOP the task — a finished pop-in needn't keep a per-frame callback alive (see Prop)
	const { stop } = useTask((dt) => {
		if (!model) return;
		t = Math.min(1, t + dt * 3.5);
		const s = eob(t);
		model.scale.set(O.sx * s, O.sy * s, O.sz * s);
		if (t >= 1) stop();
	});
</script>

<T.Group position={[obj.pos[0], obj.pos[1], obj.pos[2]]} rotation={[0, rot, 0]} userData={{ objectId: obj.id }}>
	<T.Group bind:ref={model} scale={[0, 0, 0]}>
		{#each parts as part, i (i)}
			<T.Mesh geometry={part.geo} position={part.pos} material={part.mat} castShadow receiveShadow />
		{/each}
	</T.Group>
</T.Group>
