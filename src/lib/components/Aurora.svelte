<script lang="ts">
	// AURORA for a winter night — shimmering green→violet ribbons high overhead, a reward for a snowy NIGHT
	// world (mounted by SkyDome only when sky=night AND ground=snow). One big horizontal plane that follows the
	// camera (like Clouds), with a pure procedural fragment shader: fbm-wobbled ribbons drifting across the sky
	// + fine vertical rays shimmering through them, the classic aurora palette. Additive glow, no assets.
	import { T, useTask, useThrelte } from '@threlte/core';
	import * as THREE from 'three';

	const { camera } = useThrelte();
	const ALT = 150; // high in the sky, below the star sphere (radius 180) so it glows in front of the stars
	const geo = new THREE.PlaneGeometry(2400, 2400);

	const uniforms = { uTime: { value: 0 } };
	const mat = new THREE.ShaderMaterial({
		uniforms,
		transparent: true,
		depthWrite: false,
		blending: THREE.AdditiveBlending,
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
			varying vec2 vWorld;
			varying vec2 vLocal;
			float hash(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
			float noise(vec2 p){
				vec2 i = floor(p), f = fract(p);
				float a = hash(i), b = hash(i + vec2(1.0, 0.0)), c = hash(i + vec2(0.0, 1.0)), d = hash(i + vec2(1.0, 1.0));
				vec2 u = f * f * (3.0 - 2.0 * f);
				return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
			}
			float fbm(vec2 p){ float v = 0.0, a = 0.5; for (int i = 0; i < 4; i++) { v += a * noise(p); p *= 2.03; a *= 0.5; } return v; }
			void main() {
				vec2 p = vWorld * 0.003;
				float drift = uTime * 0.03;
				// ribbons: a sine sheet whose path is wobbled by fbm so it snakes like a real curtain, drifting slowly
				float wob = fbm(vec2(p.x * 0.6 + drift, p.y * 0.6));
				float ribbon = sin((p.x + wob * 1.6) * 2.2 + drift);
				float curtain = smoothstep(0.2, 0.95, ribbon);
				// rays: fine vertical striations shimmering across the curtain (the aurora's flickering filaments)
				float rays = 0.55 + 0.45 * sin(p.y * 9.0 + uTime * 0.9 + wob * 5.0);
				float glow = curtain * rays;
				// classic palette: green cores fringing to magenta/violet at the bright crests
				vec3 col = mix(vec3(0.15, 0.95, 0.55), vec3(0.55, 0.20, 0.85), smoothstep(0.45, 0.95, ribbon));
				float edge = smoothstep(0.5, 0.22, distance(vLocal, vec2(0.5))); // radial fade hides the plane's edge
				float a = glow * edge * 0.42;
				if (a < 0.01) discard;
				gl_FragColor = vec4(col, a);
			}
		`
	});
	mat.toneMapped = false;

	const mesh = new THREE.Mesh(geo, mat);
	mesh.rotation.x = -Math.PI / 2; // lie flat overhead
	mesh.frustumCulled = false; // huge + camera-following
	mesh.renderOrder = 2; // over the stars

	useTask((dt) => {
		uniforms.uTime.value += dt;
		const cam = camera.current;
		if (cam) mesh.position.set(cam.position.x, ALT, cam.position.z); // follow the camera horizontally
	});
</script>

<T is={mesh} />
