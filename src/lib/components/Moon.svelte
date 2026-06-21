<script lang="ts">
	// A procedural MOON for the night/space sky — a shaded disc (light highlands + grey maria + scattered
	// craters, sphere-shaded so it reads round, with a soft terminator) and an additive halo. No textures.
	// Rides WITH the camera (like the stars) so it sits at a fixed sky direction infinitely far off — it
	// drifts across view as you turn but never parallaxes as you walk. Placed roughly along the moonlight.
	import { untrack } from 'svelte';
	import { T, useTask, useThrelte } from '@threlte/core';
	import { Billboard } from '@threlte/extras';
	import * as THREE from 'three';

	let { dim = 1 }: { dim?: number } = $props(); // overall brightness
	const DIM = untrack(() => dim); // constant per mount (SkyDome remounts on sky change) → snapshot for the uniforms

	const { camera } = useThrelte();
	// LOW on the horizon (user: "I wanna see it on the horizon") — same azimuth as the moonlight (x,z kept) but a
	// shallow elevation so it hangs in your forward view, not overhead. The directional light stays high, so the
	// disc no longer sits exactly on the light vector — an accepted trade for a big horizon moon (depthTest:false
	// keeps it clear of the terrain line + fog).
	const DIR = new THREE.Vector3(30, 11, 20).normalize();
	const FAR = 170; // inside the star sphere (so the moon sits in front of the stars) and the camera far plane
	const SIZE = 22; // a prominent, stylised moon

	let group = $state<THREE.Group>();

	const VERT = /* glsl */ `varying vec2 vUv; void main(){ vUv = uv; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }`;

	const moonMat = new THREE.ShaderMaterial({
		transparent: true,
		depthWrite: false,
		depthTest: true, // sits at FAR in the sky → closer terrain + the CHARACTER occlude it (was false → it floated in FRONT of the whole world)
		toneMapped: false, // the night exposure (0.5) was halving it toward invisible against the dark sky
		uniforms: { uDim: { value: DIM } },
		vertexShader: VERT,
		fragmentShader: /* glsl */ `
			precision mediump float;
			varying vec2 vUv;
			uniform float uDim;
			float mh(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
			float mn(vec2 p){
				vec2 i = floor(p), f = fract(p);
				float a = mh(i), b = mh(i + vec2(1.0, 0.0)), c = mh(i + vec2(0.0, 1.0)), d = mh(i + vec2(1.0, 1.0));
				vec2 u = f * f * (3.0 - 2.0 * f);
				return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
			}
			float mf(vec2 p){ float v = 0.0, a = 0.5; for (int i = 0; i < 4; i++) { v += a * mn(p); p *= 2.03; a *= 0.5; } return v; }
			float craters(vec2 p){
				vec2 ip = floor(p), fp = fract(p);
				float f1 = 9.0; vec2 id = vec2(0.0);
				for (int y = -1; y <= 1; y++) for (int x = -1; x <= 1; x++) {
					vec2 g = vec2(float(x), float(y));
					vec2 o = vec2(mh(ip + g), mh(ip + g + 7.3));
					float d = length(g + o - fp);
					if (d < f1) { f1 = d; id = ip + g; }
				}
				float on = step(0.55, mh(id + 2.1));                 // only some cells get a crater
				float rad = 0.16 + 0.12 * mh(id + 5.0);
				return on * (1.0 - smoothstep(rad - 0.05, rad, f1)); // 1 inside the pit
			}
			void main(){
				vec2 p = vUv * 2.0 - 1.0;
				float r = length(p);
				if (r > 1.0) discard;
				float lim = sqrt(max(0.0, 1.0 - r * r));            // sphere normal z → round shading
				vec3 N = vec3(p, lim);
				float maria = mf(p * 2.2 + 4.0);
				vec3 col = mix(vec3(0.93, 0.93, 0.88), vec3(0.58, 0.60, 0.67), smoothstep(0.45, 0.78, maria)); // highlands vs seas
				col *= 1.0 - 0.28 * craters(p * 4.0 + 1.0);          // craters darken
				float sh = clamp(dot(normalize(N), normalize(vec3(0.55, 0.25, 0.79))), 0.0, 1.0);
				col *= 0.5 + 0.6 * sh;                               // sphere shade + soft terminator
				gl_FragColor = vec4(col, smoothstep(1.0, 0.92, r) * uDim);
			}
		`
	});
	const haloMat = new THREE.ShaderMaterial({
		transparent: true,
		depthWrite: false,
		depthTest: true,
		toneMapped: false,
		blending: THREE.AdditiveBlending,
		uniforms: { uDim: { value: DIM } },
		vertexShader: VERT,
		fragmentShader: /* glsl */ `
			precision mediump float;
			varying vec2 vUv;
			uniform float uDim;
			void main(){
				float r = length(vUv * 2.0 - 1.0);
				float g = smoothstep(1.0, 0.0, r);
				gl_FragColor = vec4(vec3(0.72, 0.78, 0.95), g * g * 0.3 * uDim); // soft cool glow
			}
		`
	});

	useTask(() => {
		const cam = camera.current;
		if (group && cam) group.position.copy(cam.position).addScaledVector(DIR, FAR);
	});
</script>

<T.Group bind:ref={group}>
	<Billboard>
		<T.Mesh material={haloMat} renderOrder={1} frustumCulled={false}>
			<T.PlaneGeometry args={[SIZE * 3, SIZE * 3]} />
		</T.Mesh>
		<T.Mesh material={moonMat} renderOrder={2} frustumCulled={false}>
			<T.PlaneGeometry args={[SIZE, SIZE]} />
		</T.Mesh>
	</Billboard>
</T.Group>
