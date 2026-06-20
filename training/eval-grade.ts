// Native-eval step 3: grade the model's outputs with the REAL scenarios.ts predicates + isValidOp.
// Prints the by-tier breakdown + GATED pass rate, directly comparable to the stock numbers.
import { readFileSync } from 'node:fs';
import { isValidOp } from '../src/lib/llm-prompt';
import { SCENARIOS } from '../tests/llm/scenarios';

const out = new Map<number, unknown[]>();
for (const l of readFileSync('training/data/eval-out.jsonl', 'utf8').split('\n').filter(Boolean)) {
	const r = JSON.parse(l);
	out.set(r.i, (r.ops ?? []).filter(isValidOp));
}

const tiers = new Map<string, { p: number; n: number }>();
let gp = 0;
let gt = 0;
const fails: string[] = [];
SCENARIOS.forEach((s, i) => {
	const ops = (out.get(i) ?? []) as never[];
	const ok = s.ok(ops);
	const t = tiers.get(s.tier) ?? { p: 0, n: 0 };
	t.n++;
	if (ok) t.p++;
	tiers.set(s.tier, t);
	if (s.gate) {
		gt++;
		if (ok) gp++;
	}
	if (!ok) fails.push(`  [${s.tier}]${s.gate ? '' : ' (soft)'} ${s.t}  ->  ${JSON.stringify(ops)}`);
});

console.log('\n=== WorldGen-1.5B (tuned) · NATIVE eval ===');
console.log('by tier: ' + [...tiers].map(([k, v]) => `${k} ${v.p}/${v.n}`).join('  '));
console.log(`GATED pass: ${gp}/${gt}   (stock 1.5B=38/49, stock 3B=45/49)`);
if (fails.length) console.log('\nfails:\n' + fails.join('\n'));
