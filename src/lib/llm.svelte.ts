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
// THE model — our 0.5B WorldGen fine-tune, the only one (the 1.5B is gone; the mini is enough). WebLLM fetches the
// converted MLC weights straight from Hugging Face (free CDN + CORS), so the ~280 MB doesn't ship in the app/deploy.
// (The training pipeline still produces BOTH sizes — see training/README.md — we just ship the mini.)
const TUNED: Partial<Record<ModelKey, TunedDef>> = {
	'tuned-sm': { id: 'WorldGen-0.5B', url: 'https://huggingface.co/puruvj/WorldGen-0.5B/resolve/main/', stockId: 'Qwen2.5-0.5B-Instruct-q4f16_1-MLC' }
};
const isTuned = (k: ModelKey | null): k is ModelKey => k != null && k in TUNED;

// ONE model, no picker (user: "the mini is enough, no options"). Reuses the stock Qwen2.5-0.5B MLC model lib (WASM);
// only the fine-tuned weights differ, fetched from HF.
export const MODELS: Partial<Record<ModelKey, { id: string; label: string; sub: string }>> = {
	...(TUNED['tuned-sm'] ? { 'tuned-sm': { id: TUNED['tuned-sm'].id, label: 'WorldGen', sub: 'WorldGen 0.5B · fine-tuned · ~280 MB · 100% local' } } : {})
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
	// ONE model now — always the WorldGen mini (no picker). readStored only ever returns it or null.
	selected = $state((readStored() ?? 'tuned-sm') as ModelKey | null);

	#worker: Worker | null = null;
	#loadPromise: Promise<void> | null = null;
	// tiny RPC over the worker: each request gets an id; the worker replies {id, ok, ...} or streams {type:'progress'}
	#rpcId = 0;
	#pending = new Map<number, { resolve: (v: { content?: string }) => void; reject: (e: Error) => void }>();

	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	#call(req: Record<string, any>): Promise<{ content?: string }> {
		const id = ++this.#rpcId;
		return new Promise((resolve, reject) => {
			this.#pending.set(id, { resolve, reject });
			this.#worker!.postMessage({ ...req, id });
		});
	}

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
			this.#pending.clear();
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
				// run inference in a Web Worker so token generation never freezes the 3D render loop. The worker hosts
				// the FULL WebLLM engine; we talk to it over the tiny RPC below (so @mlc-ai/web-llm isn't bundled on the
				// main thread too — see llm-worker.ts). Progress messages stream in via the 'progress' branch.
				this.#worker = new Worker(new URL('./llm-worker.ts', import.meta.url), { type: 'module' });
				this.#worker.onmessage = (e: MessageEvent) => {
					const d = e.data;
					if (d?.type === 'progress') {
						this.progress = d.progress ?? 0;
						this.text = d.text ?? '';
						return;
					}
					const p = this.#pending.get(d?.id);
					if (!p) return;
					this.#pending.delete(d.id);
					if (d.ok) p.resolve(d);
					else p.reject(new Error(d.error ?? 'worker error'));
				};
				this.#worker.onerror = (ev) => {
					const err = new Error(ev.message || 'LLM worker crashed');
					for (const [, p] of this.#pending) p.reject(err);
					this.#pending.clear();
				};
				// origin-relative dev path → absolute URL the worker can fetch; HF/absolute URLs pass through. (Resolved
				// HERE because location.origin is reliable on the main thread; the worker receives the absolute URL.)
				const tdef = TUNED[sel];
				const tuned = tdef
					? { id: tdef.id, stockId: tdef.stockId, url: tdef.url.startsWith('http') ? tdef.url : new URL(tdef.url, location.origin).href }
					: undefined;
				await this.#call({ type: 'reload', modelId, tuned });
				this.phase = 'ready';
				this.text = 'AI ready';
				dlog('llm', 'engine ready', { model: modelId });
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
			// headroom (max_tokens 768) for multi-op compound replies — the model emits whole chains in one shot
			const reply = await this.#call({ type: 'generate', system, instruction, temperature, maxTokens: 768, schema: SCHEMA_STR });
			const raw = reply.content ?? '{}';
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
