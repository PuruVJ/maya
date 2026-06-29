// POST-BUILD: content-hash the worldsim WASM in the deploy output so it can be cached `immutable` forever and still
// auto-bust on a rebuild. Runs AFTER `vite build` (when the SvelteKit Cloudflare adapter has copied static/worldsim/*
// into the output). It renames worldsim_bg.wasm → worldsim_bg.<hash>.wasm and patches the ONE reference in the glue
// (`new URL('worldsim_bg.wasm', import.meta.url)` — wasm-pack's default loader). The glue keeps its fixed name
// (worldsim.js) because the worker imports it by a fixed path; it's cached short/revalidated (see _headers) so a new
// deploy's hashed-wasm reference propagates. Dev (`vite dev`) is untouched — it serves the fixed-name source.
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync, renameSync, existsSync } from 'node:fs';
import { join } from 'node:path';

const DIR = '.svelte-kit/cloudflare/worldsim';
const wasmPath = join(DIR, 'worldsim_bg.wasm');
const gluePath = join(DIR, 'worldsim.js');

if (!existsSync(wasmPath) || !existsSync(gluePath)) {
	// not a Cloudflare build (or the layout changed) → nothing to do, don't fail the build
	console.warn(`[hash-wasm] ${wasmPath} / glue not found — skipping (non-CF build?)`);
	process.exit(0);
}

const REF = "new URL('worldsim_bg.wasm', import.meta.url)"; // wasm-pack's default wasm loader line
let glue = readFileSync(gluePath, 'utf8');
if (!glue.includes(REF)) {
	// fail loud: if we can't patch the reference, marking *.wasm immutable would pin a STALE wasm forever
	console.error(`[hash-wasm] could not find the wasm reference in worldsim.js — aborting so we don't ship a mis-cached wasm`);
	process.exit(1);
}

const hash = createHash('sha256').update(readFileSync(wasmPath)).digest('hex').slice(0, 10);
const hashedName = `worldsim_bg.${hash}.wasm`;

glue = glue.replace(REF, `new URL('${hashedName}', import.meta.url)`);
writeFileSync(gluePath, glue);
renameSync(wasmPath, join(DIR, hashedName));

console.log(`[hash-wasm] worldsim_bg.wasm → ${hashedName} (content-hashed → immutable-cacheable)`);
