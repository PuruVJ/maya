import * as THREE from 'three';

let patched = false;

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
