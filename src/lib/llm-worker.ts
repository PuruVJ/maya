// WebLLM inference Web Worker — keeps token generation OFF the main thread so the 3D scene never
// freezes during a build (the 3B model takes several seconds; on the main thread that's a long jank).
// The app drives it via CreateWebWorkerMLCEngine in llm.svelte.ts.
import { WebWorkerMLCEngineHandler } from '@mlc-ai/web-llm';

const handler = new WebWorkerMLCEngineHandler();
self.onmessage = (msg: MessageEvent) => {
	handler.onmessage(msg);
};
