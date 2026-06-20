<script lang="ts">
	// Occasional SHOOTING STAR for the night/space sky — a bright cool-white head with a fading tail streaks
	// across the upper sky every ~10–28 s, then goes quiet. Pure GPU Points (a short trail of points sampled
	// along the streak path), additive, camera-anchored like the Moon so it rides the sky without parallax as
	// you roam. No assets. Decorative; not saved. Mounted only for night/space (SkyDome), so it's inherently
	// night-only. A rare "did you see that?" beat to match the Moon + stars.
	import { T, useTask, useThrelte } from '@threlte/core';
	import * as THREE from 'three';

	const { camera } = useThrelte();

	const N = 14; // points along the trail (head → tail); additive overlap reads as a continuous streak
	const R = 150; // sky radius the streak sits at (inside the star sphere → in front of the stars)
	const LIFE = 0.85; // seconds a streak lasts
	const TRAIL = 0.14; // how much of the path the tail lags behind the head (fraction of the sweep)

	// aTrail = 0 at the head → 1 at the tail; positions are rewritten each active frame
	const posArr = new Float32Array(N * 3).fill(0);
	const trailArr = new Float32Array(N);
	for (let i = 0; i < N; i++) trailArr[i] = i / (N - 1);
	const geo = new THREE.BufferGeometry();
	geo.setAttribute('position', new THREE.BufferAttribute(posArr, 3));
	geo.setAttribute('aTrail', new THREE.BufferAttribute(trailArr, 1));

	const uniforms = { uAlpha: { value: 0 } };
	const mat = new THREE.ShaderMaterial({
		uniforms,
		transparent: true,
		depthWrite: false,
		blending: THREE.AdditiveBlending,
		vertexShader: /* glsl */ `
			attribute float aTrail;
			uniform float uAlpha;
			varying float vA;
			void main() {
				vec4 mv = modelViewMatrix * vec4(position, 1.0);
				gl_PointSize = mix(5.0, 1.0, aTrail) * (300.0 / max(-mv.z, 1.0)); // big bright head → thin tail
				gl_Position = projectionMatrix * mv;
				vA = (1.0 - aTrail) * uAlpha;                                    // brightest at the head, dies along the tail
			}
		`,
		fragmentShader: /* glsl */ `
			precision mediump float;
			varying float vA;
			void main() {
				if (vA < 0.01) discard;
				float d = length(gl_PointCoord - 0.5);
				float soft = smoothstep(0.5, 0.0, d);
				gl_FragColor = vec4(vec3(0.82, 0.90, 1.0), soft * vA); // cool white-blue
			}
		`
	});
	const points = new THREE.Points(geo, mat);
	points.frustumCulled = false; // camera-anchored + positions written in JS

	// streak state
	let wait = 3 + Math.random() * 6; // first one comes a few seconds in
	let life = -1; // <0 = idle; otherwise seconds elapsed in the current streak
	const startDir = new THREE.Vector3();
	const endDir = new THREE.Vector3();
	const tmp = new THREE.Vector3();

	function beginStreak() {
		// head starts somewhere in the UPPER sky (elevation 30–75°, any bearing)
		const az = Math.random() * Math.PI * 2;
		const el = 0.52 + Math.random() * 0.78; // ~30°..75° in radians
		const ce = Math.cos(el);
		startDir.set(ce * Math.sin(az), Math.sin(el), ce * Math.cos(az));
		// and streaks DOWN + sideways across the sky to its end direction
		endDir
			.copy(startDir)
			.add(tmp.set(Math.random() * 0.8 - 0.4, -(0.2 + Math.random() * 0.4), Math.random() * 0.8 - 0.4))
			.normalize();
		life = 0;
	}

	useTask((dt) => {
		const cam = camera.current;
		if (cam) points.position.copy(cam.position); // anchor to the camera → sky-locked, no parallax

		if (life < 0) {
			wait -= dt;
			if (wait <= 0) beginStreak();
			return;
		}

		life += dt;
		const lp = life / LIFE; // 0 → 1 across the streak
		if (lp >= 1) {
			life = -1;
			wait = 10 + Math.random() * 18; // quiet for a while before the next
			uniforms.uAlpha.value = 0;
			return;
		}
		// fade in at the start, out at the end
		uniforms.uAlpha.value = Math.min(1, lp / 0.12) * Math.min(1, (1 - lp) / 0.18);
		// sweep the head from start→end; each trail point samples an earlier point on the path
		for (let i = 0; i < N; i++) {
			const t = Math.max(0, Math.min(1, lp - trailArr[i] * TRAIL));
			tmp.copy(startDir).lerp(endDir, t).normalize().multiplyScalar(R);
			posArr[i * 3] = tmp.x;
			posArr[i * 3 + 1] = tmp.y;
			posArr[i * 3 + 2] = tmp.z;
		}
		geo.attributes.position.needsUpdate = true;
	});
</script>

<T is={points} />
