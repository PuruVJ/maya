<script lang="ts">
	// Procedural WEATHER — drifting SNOW (Points, ground='snow'), slanted RAIN streaks (LineSegments, sky='fog'),
	// and low BLOWING SAND (Points, ground='sand' on a non-rainy sky) so the desert isn't dead air either. All
	// wrap in a player-following box so the atmosphere fills wherever you go. Pure GPU shaders, no assets;
	// decorative, not saved/shared. See the alive-world atmosphere set: clouds, fireflies, sky lighting, weather.
	import { T, useTask } from '@threlte/core';
	import * as THREE from 'three';
	import { playerState } from '$lib/playerState.svelte';
	import { wind } from '$lib/wind';

	let { ground = 'grass', sky = 'day' }: { ground?: string; sky?: string } = $props();

	const BOX = 56; // horizontal spread around the player
	const TOP = 16; // particles fall through 0..TOP m, then wrap to the top
	// uWind = the SHARED wind clock (not the local uTime) → rain/snow gust in sync with the grass/trees/water
	const uniforms = { uTime: { value: 0 }, uPlayer: { value: new THREE.Vector2() }, uWind: wind.uTime };

	// shared player-following wrap; `slope` slants the fall (rain leans, snow doesn't)
	const wrapGlsl = (fallSpeed: number, drift: string) => /* glsl */ `
		vec3 b = position;
		float y = mod(b.y - uTime * ${fallSpeed.toFixed(1)}, ${TOP.toFixed(1)});
		${drift}
		vec2 origin = uPlayer - vec2(${(BOX / 2).toFixed(1)});
		vec2 wxz = origin + mod(b.xz + drift2 - origin, vec2(${BOX.toFixed(1)}));
		float vis = smoothstep(0.0, 0.8, y) * smoothstep(${TOP.toFixed(1)}, ${(TOP - 3).toFixed(1)}, y);
	`;

	// ── SNOW (soft white Points) ───────────────────────────────────────────────────────────────────────
	const SNOW = 700;
	const sp = new Float32Array(SNOW * 3);
	const sph = new Float32Array(SNOW);
	for (let i = 0; i < SNOW; i++) {
		sp[i * 3] = Math.random() * BOX;
		sp[i * 3 + 1] = Math.random() * TOP;
		sp[i * 3 + 2] = Math.random() * BOX;
		sph[i] = Math.random() * 100;
	}
	const snowGeo = new THREE.BufferGeometry();
	snowGeo.setAttribute('position', new THREE.Float32BufferAttribute(sp, 3));
	snowGeo.setAttribute('aPhase', new THREE.Float32BufferAttribute(sph, 1));
	const snowMat = new THREE.ShaderMaterial({
		uniforms,
		transparent: true,
		depthWrite: false,
		vertexShader: /* glsl */ `
			attribute float aPhase;
			uniform vec2 uPlayer; uniform float uTime; uniform float uWind;
			varying float vV;
			void main() {
				float gust = 1.0 + 0.4 * sin(uWind * 0.23) + 0.25 * sin(uWind * 0.07 + 1.7); // shared WIND_GUST curve
				${wrapGlsl(1.7, 'vec2 drift2 = gust * vec2(sin(uTime*0.7+aPhase)*0.9 + sin(uTime*1.9+aPhase*2.0)*0.3, cos(uTime*0.6+aPhase*1.3)*0.9);')}
				vec4 mv = modelViewMatrix * vec4(wxz.x, y, wxz.y, 1.0);
				gl_Position = projectionMatrix * mv;
				gl_PointSize = (2.0 + aPhase * 0.02) * (300.0 / max(-mv.z, 1.0));
				vV = vis;
			}
		`,
		fragmentShader: /* glsl */ `
			varying float vV;
			void main() {
				float a = smoothstep(0.5, 0.0, length(gl_PointCoord - 0.5)) * vV * 0.85;
				if (a < 0.01) discard;
				gl_FragColor = vec4(1.0, 1.0, 1.0, a); // soft white flake
			}
		`
	});
	const snow = new THREE.Points(snowGeo, snowMat);
	snow.frustumCulled = false;

	// ── RAIN (slanted streaks via LineSegments — 2 verts per drop, the bottom dropped + leaned) ──────────
	const RAIN = 600;
	const rp = new Float32Array(RAIN * 2 * 3);
	const rs = new Float32Array(RAIN * 2); // aSide: 0 = streak top, 1 = streak bottom
	for (let i = 0; i < RAIN; i++) {
		const x = Math.random() * BOX;
		const y = Math.random() * TOP;
		const z = Math.random() * BOX;
		for (let k = 0; k < 2; k++) {
			rp[(i * 2 + k) * 3] = x;
			rp[(i * 2 + k) * 3 + 1] = y;
			rp[(i * 2 + k) * 3 + 2] = z;
			rs[i * 2 + k] = k;
		}
	}
	const rainGeo = new THREE.BufferGeometry();
	rainGeo.setAttribute('position', new THREE.Float32BufferAttribute(rp, 3));
	rainGeo.setAttribute('aSide', new THREE.Float32BufferAttribute(rs, 1));
	const rainMat = new THREE.ShaderMaterial({
		uniforms,
		transparent: true,
		depthWrite: false,
		vertexShader: /* glsl */ `
			attribute float aSide;
			uniform vec2 uPlayer; uniform float uTime; uniform float uWind;
			varying float vV;
			void main() {
				float gust = 1.0 + 0.4 * sin(uWind * 0.23) + 0.25 * sin(uWind * 0.07 + 1.7); // shared WIND_GUST curve
				${wrapGlsl(11.0, 'vec2 drift2 = vec2(0.0);')}
				float drop = aSide * 1.4;                          // streak length
				vec3 world = vec3(wxz.x - drop * 0.45 * gust, y - drop, wxz.y); // streak leans HARDER in a gust (wind)
				vec4 mv = modelViewMatrix * vec4(world, 1.0);
				gl_Position = projectionMatrix * mv;
				vV = vis;
			}
		`,
		fragmentShader: /* glsl */ `
			varying float vV;
			void main() {
				float a = vV * 0.32;
				if (a < 0.01) discard;
				gl_FragColor = vec4(0.62, 0.7, 0.82, a); // cool grey rain
			}
		`
	});
	const rain = new THREE.LineSegments(rainGeo, rainMat);
	rain.frustumCulled = false;

	// ── BLOWING SAND (low tan Points streaming downwind) — the desert's atmosphere, like snow/rain are for ──
	// theirs. Hugs the ground, streams in the wind and kicks up in gusts (shared clock), so a sand world isn't
	// dead air. Lives in a LOW band (not the tall fall column), so it gets its own buffer.
	const SAND = 520;
	const ap = new Float32Array(SAND * 3);
	const aph = new Float32Array(SAND);
	for (let i = 0; i < SAND; i++) {
		ap[i * 3] = Math.random() * BOX;
		ap[i * 3 + 1] = Math.random() * 2.5; // low band: 0–2.5 m
		ap[i * 3 + 2] = Math.random() * BOX;
		aph[i] = Math.random() * 100;
	}
	const sandGeo = new THREE.BufferGeometry();
	sandGeo.setAttribute('position', new THREE.Float32BufferAttribute(ap, 3));
	sandGeo.setAttribute('aPhase', new THREE.Float32BufferAttribute(aph, 1));
	const sandMat = new THREE.ShaderMaterial({
		uniforms,
		transparent: true,
		depthWrite: false,
		vertexShader: /* glsl */ `
			attribute float aPhase;
			uniform vec2 uPlayer; uniform float uTime; uniform float uWind;
			varying float vV;
			void main() {
				float gust = 1.0 + 0.4 * sin(uWind * 0.23) + 0.25 * sin(uWind * 0.07 + 1.7); // shared WIND_GUST curve
				vec3 b = position;
				float dx = uTime * 2.2 + sin(uTime * 0.5 + aPhase) * 1.5 * gust; // net downwind stream + gusty wobble
				vec2 origin = uPlayer - vec2(${(BOX / 2).toFixed(1)});
				vec2 wxz = origin + mod(b.xz + vec2(dx, dx * 0.35) - origin, vec2(${BOX.toFixed(1)})); // stream + wrap
				float yy = max(0.05, b.y + (0.2 + 0.6 * gust) * sin(uTime * 1.6 + aPhase * 3.0) * 0.4); // low, kicks up in gusts
				vec4 mv = modelViewMatrix * vec4(wxz.x, yy, wxz.y, 1.0);
				gl_Position = projectionMatrix * mv;
				gl_PointSize = (2.4 + aPhase * 0.02) * (300.0 / max(-mv.z, 1.0));
				vV = smoothstep(2.8, 0.0, yy); // hugs the ground — fades out higher up
			}
		`,
		fragmentShader: /* glsl */ `
			varying float vV;
			void main() {
				float a = smoothstep(0.5, 0.0, length(gl_PointCoord - 0.5)) * vV * 0.5;
				if (a < 0.01) discard;
				gl_FragColor = vec4(0.78, 0.68, 0.5, a); // warm tan grit
			}
		`
	});
	const sand = new THREE.Points(sandGeo, sandMat);
	sand.frustumCulled = false;

	useTask((dt) => {
		const snowing = ground === 'snow';
		const raining = !snowing && sky === 'fog';
		const sanding = ground === 'sand' && sky !== 'fog'; // dry desert wind; rain suppresses the dust
		snow.visible = snowing;
		rain.visible = raining;
		sand.visible = sanding;
		if (!snowing && !raining && !sanding) return;
		uniforms.uTime.value += dt;
		uniforms.uPlayer.value.set(playerState.pos[0], playerState.pos[2]);
	});
</script>

<T is={snow} />
<T is={rain} />
<T is={sand} />
