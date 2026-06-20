// Downloads the GGUF model(s) for the LLM test suite into .models/ (gitignored, fetched once,
// skipped if present). The 1.5B is the current prod model; the 3B is for the compound-prompt
// reliability comparison. Pass a name substring to fetch just one, e.g. `node fetch-test-model.mjs 3b`.
import { createWriteStream, existsSync, mkdirSync, statSync } from 'node:fs';
import { Readable } from 'node:stream';
import { pipeline } from 'node:stream/promises';

const MODELS = [
	{
		dest: '.models/qwen2.5-1.5b-instruct-q4_k_m.gguf',
		url: 'https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf',
		label: 'Qwen2.5-1.5B-Instruct Q4_K_M (~1.1GB)'
	},
	{
		dest: '.models/qwen2.5-3b-instruct-q4_k_m.gguf',
		url: 'https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf',
		label: 'Qwen2.5-3B-Instruct Q4_K_M (~1.9GB)'
	}
];

const filter = process.argv[2]?.toLowerCase();
mkdirSync('.models', { recursive: true });

for (const m of MODELS) {
	if (filter && !m.dest.toLowerCase().includes(filter)) continue;
	if (existsSync(m.dest) && statSync(m.dest).size > 500_000_000) {
		console.log('✓ present:', m.dest);
		continue;
	}
	console.log('Downloading ' + m.label + ' → ' + m.dest + ' …');
	const res = await fetch(m.url);
	if (!res.ok || !res.body) {
		console.error('download failed:', m.url, res.status, res.statusText);
		process.exit(1);
	}
	await pipeline(Readable.fromWeb(res.body), createWriteStream(m.dest));
	console.log('✓ saved', m.dest, '(' + (statSync(m.dest).size / 1e9).toFixed(2) + ' GB)');
}
