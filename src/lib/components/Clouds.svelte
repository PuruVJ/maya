<script lang="ts">
	// Procedural drifting clouds — a single big horizontal plane high in the sky that follows the camera
	// (so it never ends), with an fbm-noise fragment shader generating soft cloud cover that scrolls over
	// WORLD space (parallax-correct: clouds stay put in the world as you roam, they don't slide with you).
	// A radial fade hides the plane's square edge. Pure shader, no assets ([[shader-first-direction]]).
	// Only mounted for the cloudy skies (day/sunset/fog); night/space use Stars instead.
	import { untrack } from 'svelte';
	import { T, useTask, useThrelte } from '@threlte/core';
	import * as THREE from 'three';
	import { weather } from '$lib/weather';

	let { tint = '#ffffff', opacity = 0.55, cover = 0.5 }: { tint?: string; opacity?: number; cover?: number } = $props();

	const { camera } = useThrelte();
	const ALT = 130; // cloud-deck altitude
	const geo = new THREE.PlaneGeometry(2400, 2400);

	// snapshot props once — SkyDome remounts Clouds when the sky enum changes, so they're constant per mount
	const P = untrack(() => ({ tint, opacity, cover }));
	const uniforms = {
		uTime: { value: 0 },
		uColor: { value: new THREE.Color(P.tint) },
		uOpacity: { value: P.opacity },
		uCover: { value: P.cover }, // 0..1 → how much of the sky is clouded
		uFlash: weather.uFlash // shared lightning pulse → the deck flashes white during a rain-storm strike
	};

	const mat = new THREE.ShaderMaterial({
		uniforms,
		transparent: true,
		depthWrite: false,
		side: THREE.DoubleSide,
		vertexShader: /* glsl */ `
			varying vec2 vWorld;
			varying vec2 vLocal;
			void main() {
				vLocal = uv;
				vWorld = (modelMatrix * vec4(position, 1.0)).xz;
				gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
			}
		`,
		fragmentShader: /* glsl */ `
			uniform float uTime;
			uniform vec3 uColor;
			uniform float uOpacity;
			uniform float uCover;
			uniform float uFlash;
			varying vec2 vWorld;
			varying vec2 vLocal;
			float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
			float noise(vec2 p) {
				vec2 i = floor(p), f = fract(p);
				float a = hash(i), b = hash(i + vec2(1.0, 0.0)), c = hash(i + vec2(0.0, 1.0)), d = hash(i + vec2(1.0, 1.0));
				vec2 u = f * f * (3.0 - 2.0 * f);
				return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
			}
			float fbm(vec2 p) {
				float v = 0.0, amp = 0.5;
				for (int i = 0; i < 5; i++) { v += amp * noise(p); p *= 2.02; amp *= 0.5; }
				return v;
			}
			void main() {
				vec2 p = vWorld * 0.0022 + vec2(uTime * 0.006, uTime * 0.004); // big soft clouds, slow drift
				float n = fbm(p);
				float lo = mix(0.58, 0.28, uCover);                            // more cover → lower threshold
				float cloud = smoothstep(lo, lo + 0.30, n);
				// gentle radial fade only near the rim → clouds fill most of the visible sky (not just a tight
				// disc straight overhead, which is why they were never seen) while the square corners still fade out
				float edge = smoothstep(0.5, 0.30, distance(vLocal, vec2(0.5)));
				float a = cloud * uOpacity * edge;
				if (a < 0.01) discard;
				// FAKE VOLUMETRIC SELF-SHADOW: compare density a step toward the sun. Clearer toward the sun → this
				// bit faces the light (bright); denser toward the sun → it sits under cloud, in shade. Turns the flat
				// deck into fluffy forms lit from the same side as the scene's directional sun (DIR's horizontal dir),
				// instead of a uniform brightness-by-density. (Water's cloud reflection samples COVER, not this, so
				// it's unaffected.)
				float toward = fbm(p + vec2(0.83, 0.55) * 0.05);              // density one step toward the sun
				float shade = smoothstep(-0.12, 0.14, n - toward);           // 0 self-shadowed underside → 1 sunlit side
				vec3 col = mix(uColor * 0.6, uColor, shade);                  // shadowed grey base → sunlit top
				col += uColor * smoothstep(0.55, 0.95, shade) * cloud * 0.12; // crisp sunlit rim where it turns to the light
				col = mix(col, vec3(1.0), clamp(uFlash * 0.45, 0.0, 0.85) * cloud); // LIGHTNING — the deck flashes white
				gl_FragColor = vec4(col, a);
			}
		`
	});
	mat.toneMapped = false;

	const mesh = new THREE.Mesh(geo, mat);
	mesh.rotation.x = -Math.PI / 2; // lie flat
	mesh.frustumCulled = false; // huge + camera-following; never cull
	mesh.renderOrder = 1;

	useTask((dt) => {
		uniforms.uTime.value += dt;
		const cam = camera.current;
		if (cam) mesh.position.set(cam.position.x, ALT, cam.position.z); // follow the camera horizontally
	});
</script>

<T is={mesh} />
