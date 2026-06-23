// WebLLM inference Web Worker — hosts the FULL MLCEngine in-worker so token generation stays OFF the main thread
// (the model takes several seconds; on the main thread that's a long render-loop jank). The main thread drives it
// through a tiny custom postMessage RPC (see llm.svelte.ts) — which means `@mlc-ai/web-llm` (~5.8 MB, monolithic,
// no subpath exports) is bundled HERE ONLY. The old setup also imported it on the main thread (just to call
// CreateWebWorkerMLCEngine), shipping the whole library a SECOND time as a page chunk; this kills that duplicate.
import { CreateMLCEngine, prebuiltAppConfig } from '@mlc-ai/web-llm';

type ModelRecord = (typeof prebuiltAppConfig.model_list)[number];
type Tuned = { id: string; url: string; stockId: string };
type Req =
	| { id: number; type: 'reload'; modelId: string; tuned?: Tuned }
	| { id: number; type: 'generate'; system: string; instruction: string; temperature: number; maxTokens: number; schema: string };

let engine: Awaited<ReturnType<typeof CreateMLCEngine>> | null = null;

self.onmessage = async (e: MessageEvent<Req>) => {
	const msg = e.data;
	try {
		if (msg.type === 'reload') {
			// register our hosted (fine-tuned) weights against the matching STOCK model lib — same architecture + quant,
			// so there's no custom WASM to compile; only the weights are ours. (modelUrl is already absolute, made so on
			// the main thread where location.origin is reliable.)
			let appConfig = prebuiltAppConfig;
			if (msg.tuned) {
				const t = msg.tuned;
				const stock = prebuiltAppConfig.model_list.find((m) => m.model_id === t.stockId);
				appConfig = {
					...prebuiltAppConfig,
					model_list: [...prebuiltAppConfig.model_list, { model: t.url, model_id: t.id, model_lib: stock?.model_lib } as ModelRecord]
				};
			}
			engine = await CreateMLCEngine(msg.modelId, {
				appConfig,
				initProgressCallback: (r) => self.postMessage({ type: 'progress', progress: r.progress ?? 0, text: r.text ?? '' })
			});
			self.postMessage({ id: msg.id, ok: true });
		} else if (msg.type === 'generate') {
			if (!engine) throw new Error('engine not loaded');
			const reply = await engine.chat.completions.create({
				messages: [
					{ role: 'system', content: msg.system },
					{ role: 'user', content: msg.instruction }
				],
				response_format: { type: 'json_object', schema: msg.schema },
				temperature: msg.temperature,
				max_tokens: msg.maxTokens
			});
			self.postMessage({ id: msg.id, ok: true, content: reply.choices?.[0]?.message?.content ?? '{}' });
		}
	} catch (err) {
		self.postMessage({ id: msg.id, ok: false, error: String(err) });
	}
};
