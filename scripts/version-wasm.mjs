// Append a CONTENT-VERSION query (?v=<hash>) to the wasm URL in the wasm-pack glue, so the FIXED-name worldsim_bg.wasm
// can be cached `immutable` yet auto-bust whenever the engine changes. Runs as part of `build:wasm` — i.e. LOCALLY,
// where cargo/wasm-pack exist (Cloudflare has NO cargo, so the wasm is built + COMMITTED locally and CF only serves
// it). Crucially this does NOT rename any file: worldsim_bg.wasm keeps its name (already git-tracked, served fine by
// CF), and only worldsim.js's URL gets the ?v=. That sidesteps the static/worldsim/.gitignore `*` (which would ignore
// a new hash-named file) and the CF-side build-output renaming that broke serving (500). Commit both files after.
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync, existsSync } from 'node:fs';

const DIR = 'static/worldsim';
const wasmPath = `${DIR}/worldsim_bg.wasm`;
const gluePath = `${DIR}/worldsim.js`;
if (!existsSync(wasmPath) || !existsSync(gluePath)) {
	console.error(`[version-wasm] ${wasmPath} / glue missing — did wasm-pack run?`);
	process.exit(1);
}

const hash = createHash('sha256').update(readFileSync(wasmPath)).digest('hex').slice(0, 10);

// wasm-pack regenerates this line fresh each build (no ?v=); the regex also matches an already-versioned line so a
// re-run just swaps the hash (idempotent).
const re = /new URL\('worldsim_bg\.wasm(?:\?v=[a-f0-9]+)?', import\.meta\.url\)/;
let glue = readFileSync(gluePath, 'utf8');
if (!re.test(glue)) {
	console.error('[version-wasm] could not find the wasm URL in worldsim.js — aborting (would mis-cache the wasm)');
	process.exit(1);
}
glue = glue.replace(re, `new URL('worldsim_bg.wasm?v=${hash}', import.meta.url)`);
writeFileSync(gluePath, glue);

console.log(`[version-wasm] worldsim.js → worldsim_bg.wasm?v=${hash} (fixed-name wasm, immutable-cacheable, auto-busts)`);
