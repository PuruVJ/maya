// In-browser LLM (WebLLM + WebGPU) that turns a natural-language instruction into a
// grammar-constrained array of world ops. 100% local — no key, no server.
// Prompt/schema/validation live in ./llm-prompt (shared with the node test suite).
import type { Op } from './engine';
import type { World, Player } from './world';
import { dlog, derror } from './debug';
import { buildSystem, buildWorldState, SCHEMA_STR, isValidOp } from './llm-prompt';

export type ModelKey = 'fast' | 'smart' | 'tuned' | 'tuned-sm';

// Our domain-specialised fine-tunes (see training/README.md). Each shares an architecture + quant
// (q4f16_1) with a stock WebLLM model → it REUSES that stock model lib (WASM); only the weights
// differ, so there's no custom WASM to compile. `url` is where the converted MLC weights live —
// dev serves them from static/models/<id>/ (origin-relative; made absolute at load). For prod, swap
// to a Hugging Face resolve URL (WebLLM fetches MLC models from HF natively — free CDN + CORS), e.g.
// 'https://huggingface.co/puruvj/WorldGen-1.5B/resolve/main/' (TRAILING SLASH). Empty map = hidden.
type TunedDef = { id: string; url: string; stockId: string };
const TUNED: Partial<Record<ModelKey, TunedDef>> = {
	tuned: { id: 'WorldGen-1.5B', url: '/models/WorldGen-1.5B/', stockId: 'Qwen2.5-1.5B-Instruct-q4f16_1-MLC' },
	'tuned-sm': { id: 'WorldGen-0.5B', url: '/models/WorldGen-0.5B/', stockId: 'Qwen2.5-0.5B-Instruct-q4f16_1-MLC' }
};
const isTuned = (k: ModelKey | null): k is ModelKey => k != null && k in TUNED;

// LOCAL choices (all WebLLM/WebGPU, all free): Fast runs anywhere; Smart is sharper on big, messy,
// multi-step builds (44/49 vs 37/49 on the scenario battery); the Tuned pair are our fine-tunes —
// lighter AND sharper on our grammar (Tuned = 1.5B, Tuned Mini = 0.5B, ~280 MB for low-end devices).
export const MODELS: Partial<Record<ModelKey, { id: string; label: string; sub: string }>> = {
	fast: { id: 'Qwen2.5-1.5B-Instruct-q4f16_1-MLC', label: 'Fast', sub: 'Qwen 1.5B · ~1 GB · snappy, runs on any GPU' },
	smart: { id: 'Qwen2.5-3B-Instruct-q4f16_1-MLC', label: 'Smart', sub: 'Qwen 3B · ~2 GB · sharper on big, detailed builds' },
	...(TUNED.tuned ? { tuned: { id: TUNED.tuned.id, label: 'Tuned', sub: 'WorldGen 1.5B · fine-tuned for this game · sharpest' } } : {}),
	...(TUNED['tuned-sm'] ? { 'tuned-sm': { id: TUNED['tuned-sm'].id, label: 'Tuned Mini', sub: 'WorldGen 0.5B · fine-tuned · ~280 MB · lightest, low-end devices' } } : {})
};

const STORE_KEY = 'worldgen:model';
function readStored(): ModelKey | null {
	if (typeof localStorage === 'undefined') return null;
	const v = localStorage.getItem(STORE_KEY);
	return v && v in MODELS ? (v as ModelKey) : null;
}

type Phase = 'idle' | 'loading' | 'ready' | 'error';

/** Reactive, lazily-loaded local LLM. The UI reads the $state fields; BuildBar calls load()/generate(). */
export class WorldLLM {
	phase = $state('idle' as Phase);
	progress = $state(0);
	text = $state('AI not loaded');
	busy = $state(false);
	// default to the fine-tune when it's available (TUNED_URL set); else null → first-run picker
	selected = $state((readStored() ?? ('tuned' in MODELS ? 'tuned' : null)) as ModelKey | null);

	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	#engine: any = null;
	#worker: Worker | null = null;
	#loadPromise: Promise<void> | null = null;

	get model() {
		return this.selected ? (MODELS[this.selected] ?? null) : null;
	}

	/** Pick (or switch) the local model, remember it, and (re)load. */
	choose(which: ModelKey): Promise<void> {
		const changed = this.selected !== which;
		if (typeof localStorage !== 'undefined') localStorage.setItem(STORE_KEY, which);
		this.selected = which;
		if (changed) {
			// tear down any previous engine + worker so load() rebuilds with the new weights
			this.#worker?.terminate();
			this.#worker = null;
			this.#engine = null;
			this.#loadPromise = null;
			this.phase = 'idle';
			this.progress = 0;
		}
		return this.load();
	}

	load(): Promise<void> {
		if (!this.selected) return Promise.resolve(); // wait for a first-run choice
		if (this.#loadPromise) return this.#loadPromise;
		const sel = this.selected;
		const modelId = MODELS[sel]!.id;
		this.#loadPromise = (async () => {
			try {
				if (typeof navigator === 'undefined' || !('gpu' in navigator)) {
					throw new Error('WebGPU not available — use Chrome or Edge');
				}
				this.phase = 'loading';
				this.text = 'Loading AI…';
				// make the cached weights non-evictable so later visits are instant
				if (navigator.storage?.persist) {
					try {
						await navigator.storage.persist();
					} catch {
						/* ignore */
					}
				}
				const webllm = await import('@mlc-ai/web-llm');
				// run inference in a Web Worker so token generation never freezes the 3D render loop
				this.#worker = new Worker(new URL('./llm-worker.ts', import.meta.url), { type: 'module' });
				// eslint-disable-next-line @typescript-eslint/no-explicit-any
				const engineConfig: any = {
					initProgressCallback: (r: { progress?: number; text?: string }) => {
						this.progress = r.progress ?? 0;
						this.text = r.text ?? '';
					}
				};
				const tdef = TUNED[sel];
				if (tdef) {
					// register our hosted weights against the matching STOCK model lib (same architecture +
					// quant) — no custom WASM needed, only the fine-tuned weights are ours
					const stock = webllm.prebuiltAppConfig.model_list.find((m) => m.model_id === tdef.stockId);
					// origin-relative dev path → absolute URL WebLLM can fetch; HF/absolute URLs pass through
					const modelUrl = tdef.url.startsWith('http') ? tdef.url : new URL(tdef.url, location.origin).href;
					engineConfig.appConfig = {
						...webllm.prebuiltAppConfig,
						model_list: [
							...webllm.prebuiltAppConfig.model_list,
							{ model: modelUrl, model_id: tdef.id, model_lib: stock?.model_lib }
						]
					};
				}
				this.#engine = await webllm.CreateWebWorkerMLCEngine(this.#worker, modelId, engineConfig);
				this.phase = 'ready';
				this.text = 'AI ready';
				dlog('llm', 'engine ready', { model: modelId, engine: this.#engine?.constructor?.name });
			} catch (e) {
				this.phase = 'error';
				this.text = e instanceof Error ? e.message : 'AI failed to load';
				derror('llm', 'load failed', { error: String(e) });
				this.#loadPromise = null; // allow retry
				throw e;
			}
		})();
		return this.#loadPromise;
	}

	async generate(instruction: string, world: World, player: Player, temperature = 0.3): Promise<Op[]> {
		await this.load();
		this.busy = true;
		const t0 = performance.now();
		try {
			// the fine-tunes were trained on the compact world-state prompt; stock models need the full one
			const system = isTuned(this.selected) ? buildWorldState(world, player) : buildSystem(world, player);
			const reply = await this.#engine.chat.completions.create({
				messages: [
					{ role: 'system', content: system },
					{ role: 'user', content: instruction }
				],
				response_format: { type: 'json_object', schema: SCHEMA_STR },
				temperature,
				// headroom for multi-op compound replies (the model emits whole chains in one shot)
				max_tokens: 768
			});
			const raw = reply.choices?.[0]?.message?.content ?? '{}';
			const parsed = JSON.parse(raw);
			const ops: Op[] = Array.isArray(parsed.ops) ? parsed.ops.filter(isValidOp) : [];
			const ms = Math.round(performance.now() - t0);
			dlog('llm', `generated ${ops.length} ops in ${ms}ms`, { instruction, ops });
			// dedicated prompt→result log (→ prompts.log) to spot failures + decide next training data
			dlog('prompt', instruction, { model: this.selected, ops, ms, raw });
			return ops;
		} catch (e) {
			derror('llm', 'generate failed', { instruction, error: String(e) });
			dlog('prompt', instruction, { model: this.selected, ops: [], error: String(e) });
			return [];
		} finally {
			this.busy = false;
		}
	}
}

// app-wide singleton
export const llm = new WorldLLM();
