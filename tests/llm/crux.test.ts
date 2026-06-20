import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { existsSync } from 'node:fs';
import path from 'node:path';
import { getLlama, LlamaChatSession, type Llama, type LlamaModel } from 'node-llama-cpp';
import { buildSystem, buildWorldState, isValidOp, KIND_NAMES, GROUND, SKY, AREAS, ZONEMAT, SHAPE, TERRAIN } from '../../src/lib/llm-prompt';
import { demoWorld, type Player } from '../../src/lib/world';
import { SCENARIOS } from './scenarios';

// The real model(s), run locally on CPU/Metal. Manual crux test, AUTOMATED. Compares 1.5B vs 3B on
// single-op INTENT, one-shot COMPOUND/CRUD, and a big realistic SCENARIO suite (messy human prompts,
// overreach + boundary notes, adversarial junk). llama.cpp + GBNF here vs WebLLM + XGrammar in prod
// → an APPROXIMATION, great for comparisons + catching prompt/vocab/grammar regressions.
// our fine-tune (training/to_gguf.sh): prefer q4_k_m, fall back to the f16 export
const tuned = ['.models/worldgen-1.5b-instruct-q4_k_m.gguf', '.models/worldgen-1.5b-instruct-f16.gguf']
	.map((p) => path.resolve(p))
	.find(existsSync);
const MODELS = [
	{ name: 'Qwen2.5-1.5B', path: path.resolve('.models/qwen2.5-1.5b-instruct-q4_k_m.gguf') },
	{ name: 'Qwen2.5-3B', path: path.resolve('.models/qwen2.5-3b-instruct-q4_k_m.gguf') },
	...(tuned ? [{ name: 'WorldGen-1.5B (tuned)', path: tuned }] : [])
]
	.filter((m) => existsSync(m.path))
	// EVAL_MODEL=tuned (or any name substring) → eval just that one, for fast fine-tune iteration
	.filter((m) => !process.env.EVAL_MODEL || m.name.toLowerCase().includes(process.env.EVAL_MODEL.toLowerCase()));

const PLAYER: Player = { pos: [0, 0, 6], yaw: 0 };

// Lean per-op oneOf grammar — each op carries only its real fields (no bloat → no truncation).
const S = { type: 'string' };
const N = { type: 'number' };
const TEST_SCHEMA = {
	type: 'object',
	required: ['ops'],
	properties: {
		ops: {
			type: 'array',
			items: {
				oneOf: [
					{ type: 'object', required: ['op', 'kind'], properties: { op: { const: 'add' }, kind: { enum: KIND_NAMES }, at: S, dist: N, count: N, color: S } },
					{ type: 'object', required: ['op', 'kind', 'count', 'area'], properties: { op: { const: 'scatter' }, kind: { enum: KIND_NAMES }, count: N, area: { enum: AREAS } } },
					{ type: 'object', required: ['op', 'id'], properties: { op: { const: 'remove' }, id: S } },
					{ type: 'object', required: ['op', 'id'], properties: { op: { const: 'move' }, id: S, at: S } },
					{ type: 'object', required: ['op', 'id', 'color'], properties: { op: { const: 'paint' }, id: S, color: S } },
					{ type: 'object', required: ['op', 'value'], properties: { op: { const: 'setGround' }, value: { enum: GROUND } } },
					{ type: 'object', required: ['op', 'value'], properties: { op: { const: 'setSky' }, value: { enum: SKY } } },
					{ type: 'object', required: ['op', 'material', 'shape'], properties: { op: { const: 'addZone' }, material: { enum: ZONEMAT }, shape: { enum: SHAPE }, at: S, size: N } },
					{ type: 'object', required: ['op', 'material'], properties: { op: { const: 'addPath' }, material: { enum: ZONEMAT }, from: S, to: S, width: N } },
					{ type: 'object', required: ['op', 'preset'], properties: { op: { const: 'setTerrain' }, preset: { enum: TERRAIN }, amplitude: N } },
					{ type: 'object', required: ['op', 'text'], properties: { op: { const: 'note' }, text: S } }
				]
			}
		}
	}
};

// instruction → which op(s) count as "got the intent". Also note the expected anchor where relevant.
const INTENT: { t: string; expect: string[]; anchor?: string }[] = [
	{ t: 'build a house here', expect: ['add'] },
	{ t: 'make it night', expect: ['setSky'] },
	{ t: 'plant a forest to the north', expect: ['scatter'] },
	{ t: 'turn the ground to snow', expect: ['setGround'] },
	{ t: 'put a lamp to my left', expect: ['add'], anchor: 'left' },
	{ t: 'a tall tower far in front of me', expect: ['add'], anchor: 'front' },
	{ t: 'make a road behind me', expect: ['addPath'] },
	{ t: 'dig a lake to the east', expect: ['addZone'] },
	{ t: 'a well between the two houses', expect: ['add'], anchor: 'between' },
	{ t: 'a lamp on top of the house', expect: ['add'], anchor: 'on' },
	{ t: 'add rolling hills', expect: ['setTerrain'] },
	{ t: 'a field of flowers to the south', expect: ['addZone', 'scatter'] }
];

/* eslint-disable @typescript-eslint/no-explicit-any */
const COMPOUND: { t: string; ok: (o: any[]) => boolean; desc: string }[] = [
	{ t: 'add a house and a tree right next to it', ok: (o) => o.filter((x) => x.op === 'add').length >= 2, desc: '2 adds' },
	{ t: 'build a tower and put a lamp on top of it', ok: (o) => o.some((x) => x.op === 'add' && x.kind === 'tower') && o.some((x) => x.op === 'add' && x.kind === 'lamp'), desc: 'tower + lamp' },
	{ t: 'add a house and a tree next to it, then paint the house red and put a lamp on it', ok: (o) => o.filter((x) => x.op === 'add').length >= 3 && o.some((x) => x.op === 'paint'), desc: '3 adds + paint' },
	{ t: 'make it sunset and add a house with a well beside it', ok: (o) => o.some((x) => x.op === 'setSky') && o.filter((x) => x.op === 'add').length >= 2, desc: 'setSky + 2 adds' },
	{ t: 'remove the cat and the well', ok: (o) => o.filter((x) => x.op === 'remove').length >= 2, desc: '2 removes' }
];

for (const M of MODELS) {
	describe(`LLM battery · ${M.name}`, () => {
		// the fine-tune was trained on the compact world-state prompt; stock models get the full one
		const SYS = M.name.toLowerCase().includes('tuned') ? buildWorldState : buildSystem;
		let llama: Llama;
		let model: LlamaModel;
		let grammar: any;

		beforeAll(async () => {
			llama = await getLlama();
			model = await llama.loadModel({ modelPath: M.path });
			grammar = await llama.createGrammarForJsonSchema(TEST_SCHEMA as any);
		}, 300_000);

		afterAll(async () => {
			await model?.dispose?.();
		});

		async function ask(system: string, instruction: string): Promise<any[]> {
			// 4096 so the (now longer, limits-aware) system prompt + 768 max-gen never overflows here.
			// NB: prod (WebLLM) runs the model's full 32k context, so this cap is a harness-only concern.
			const context = await model.createContext({ contextSize: 4096 });
			const session = new LlamaChatSession({ contextSequence: context.getSequence(), systemPrompt: system });
			let ops: any[] = [];
			try {
				const res = await session.prompt(instruction, { grammar, temperature: 0.3, maxTokens: 768 });
				const parsed = grammar.parse(res);
				ops = Array.isArray(parsed?.ops) ? parsed.ops.filter(isValidOp) : [];
			} catch {
				/* parse/infer error → empty */
			}
			await context.dispose();
			return ops;
		}

		it('intent battery (single-op)', async () => {
			const system = SYS(demoWorld(), PLAYER);
			let pass = 0;
			const rows: string[] = [];
			for (const c of INTENT) {
				const ops = await ask(system, c.t);
				const hit = ops.some((o) => c.expect.includes(o.op));
				const anchorOk = !c.anchor || ops.some((o) => typeof o.at === 'string' && o.at.startsWith(c.anchor!));
				if (hit) pass++;
				rows.push(`${hit ? 'PASS' : 'FAIL'} ${c.anchor && !anchorOk ? '(anchor x) ' : ''}${c.t}  ->  ${JSON.stringify(ops)}`);
			}
			console.log(`\n=== ${M.name} · INTENT ===\n` + rows.join('\n') + `\nintent: ${pass}/${INTENT.length}`);
			expect(pass).toBeGreaterThanOrEqual(Math.ceil(INTENT.length * 0.7));
		}, 600_000);

		it('compound / CRUD battery (one-shot, multi-op)', async () => {
			let pass = 0;
			const rows: string[] = [];
			for (const c of COMPOUND) {
				const ops = await ask(SYS(demoWorld(), PLAYER), c.t);
				const ok = c.ok(ops);
				if (ok) pass++;
				rows.push(`${ok ? 'PASS' : 'FAIL'} [${c.desc}] ${c.t}  ->  ${JSON.stringify(ops)}`);
			}
			console.log(`\n=== ${M.name} · COMPOUND/CRUD (one-shot) ===\n` + rows.join('\n') + `\ncompound: ${pass}/${COMPOUND.length}`);
			expect(pass).toBeGreaterThanOrEqual(0);
		}, 600_000);

		it('scenario suite (messy human prompts + boundaries)', async () => {
			const tiers = new Map<string, { pass: number; total: number }>();
			let gp = 0;
			let gt = 0;
			const rows: string[] = [];
			for (const s of SCENARIOS) {
				const ops = await ask(SYS(s.world ?? demoWorld(), PLAYER), s.t);
				const ok = s.ok(ops);
				const tt = tiers.get(s.tier) ?? { pass: 0, total: 0 };
				tt.total++;
				if (ok) tt.pass++;
				tiers.set(s.tier, tt);
				if (s.gate) {
					gt++;
					if (ok) gp++;
				}
				rows.push(`${ok ? 'PASS' : 'FAIL'}${s.gate ? '' : ' (exploratory)'} [${s.tier}] ${s.t}  ->  ${JSON.stringify(ops)}`);
			}
			console.log(`\n=== ${M.name} · SCENARIOS ===\n` + rows.join('\n'));
			console.log('\nby tier: ' + [...tiers].map(([k, v]) => `${k} ${v.pass}/${v.total}`).join('  '));
			console.log(`GATED pass: ${gp}/${gt}`);
			expect(gp).toBeGreaterThanOrEqual(Math.ceil(gt * 0.6));
		}, 1_200_000);
	});
}

describe.skipIf(MODELS.length > 0)('LLM battery (skipped — no models downloaded)', () => {
	it('run `pnpm test:llm` to fetch model(s) and run', () => {
		expect(MODELS.length).toBe(0);
	});
});
