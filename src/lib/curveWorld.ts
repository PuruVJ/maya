import * as THREE from 'three';

let patched = false;
let curveRadius = 0; // the active fold radius (0 = flat) — needed to INVERT the fold for cursor→ground picking

/** The active world-fold radius (0 = flat). Other shaders (e.g. distant settlement glows) read this to apply the
 *  SAME inception-fold so their world-space points ride up the curve instead of floating below the reared terrain. */
export function worldCurveRadius(): number {
	return curveRadius;
}

/**
 * INCEPTION-FOLD / valley world curve. Globally patches the standard vertex shader so the ground
 * rears up and curls toward the sky in the FORWARD/BACK direction (world-Z), like the inside of a
 * vast valley — the sides (world-X) stay flat and the whole upper sky stays open. Walk toward the
 * rising horizon and it forever rears up and recedes (it re-centers on the camera).
 *
 * The forward axis (world-Z, relative to the camera) wraps onto a circle of radius `radius` via
 * sin/cos: ground `d` metres ahead/behind sits at arc-angle d/radius, lifted (1−cos)·radius up. Done
 * in WORLD space so the axis is stable whichever way you look; you always stand on flat ground at the
 * valley floor. It only reaches straight overhead at d = π·radius — kept far out in fog, so the sky
 * stays open above. Terrain must be DoubleSide so the reared-over far ground still renders.
 *
 * `radius` (metres): smaller = tighter, nearer, steeper valley walls; larger = gentler, more distant.
 * Cheap (a few lines in the vertex shader, affects every MeshStandardMaterial). Call ONCE before any
 * material compiles. Set `radius` to 0 to disable.
 */
export function enableWorldCurvature(radius = 450): void {
	if (patched || radius <= 0 || typeof window === 'undefined') return;
	patched = true;
	curveRadius = radius;
	// already patched in a prior HMR cycle (THREE is a singleton) → don't re-run/warn
	if (THREE.ShaderChunk.project_vertex.includes('_wp')) return;
	const src = THREE.ShaderChunk.project_vertex;
	// At this point in the chunk, `mvPosition` is the post-instance OBJECT-space position (model
	// matrix not yet applied), so modelMatrix * mvPosition gives true world position.
	const bent = src.replace(
		'mvPosition = modelViewMatrix * mvPosition;',
		[
			'vec4 _wp = modelMatrix * mvPosition;',
			`float _ang = (_wp.z - cameraPosition.z) / float(${radius});`, // arc-angle along the fold
			`_wp.z = cameraPosition.z + sin(_ang) * float(${radius});`, // wrap forward/back onto the circle
			`_wp.y += (1.0 - cos(_ang)) * float(${radius});`, // …rearing the far ground up into a valley wall
			'mvPosition = viewMatrix * _wp;'
		].join('\n\t')
	);
	if (bent === src) {
		console.warn('[curveWorld] project_vertex shape changed; fold not applied');
		return;
	}
	THREE.ShaderChunk.project_vertex = bent;
}

/**
 * INVERSE of the fold, for cursor→ground PICKING. The ground is rendered reared-up (the shader lifts far ground
 * along world-Z), so a naive raycast against the flat y=0 plane overshoots and drops things "far back". Given the
 * camera ray (world space), this returns the FLAT world (x,z) of the ground point the ray VISUALLY meets — i.e.
 * the point that, after the fold, lies on the ray. Returns null if the ray never meets the curved ground (sky).
 *
 * Render of a ground point (x,0,z): X=x, Z=camZ+sin(a)·R, Y=(1−cos(a))·R, with a=(z−camZ)/R. Setting that equal
 * to the ray O+tD and using sin²+cos²=1 gives a quadratic in t (camZ = O.z since the ray starts at the camera).
 */
export function curvedGroundXZ(ray: THREE.Ray): { x: number; z: number } | null {
	const O = ray.origin;
	const D = ray.direction;
	const R = curveRadius;
	if (R <= 0) {
		// no fold → plain y=0 plane
		const t = -O.y / D.y;
		return t > 0 ? { x: O.x + t * D.x, z: O.z + t * D.z } : null;
	}
	const c = 1 - O.y / R;
	const A = (D.y * D.y + D.z * D.z) / (R * R);
	const B = (-2 * c * D.y) / R;
	const C = c * c - 1;
	const disc = B * B - 4 * A * C;
	if (A === 0 || disc < 0) return null;
	const s = Math.sqrt(disc);
	let t = (-B - s) / (2 * A); // the nearer ground hit
	if (t <= 0) t = (-B + s) / (2 * A);
	if (t <= 0) return null;
	const ang = Math.atan2((t * D.z) / R, c - (t * D.y) / R);
	return { x: O.x + t * D.x, z: O.z + ang * R };
}
