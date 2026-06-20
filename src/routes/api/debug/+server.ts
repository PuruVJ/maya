// Dev-only sink for the browser debug pipe. Appends batched log entries to ./debug.log.
// In production this is a 204 no-op (and node:fs is only imported under the dev guard,
// so it never gets pulled into an edge/static bundle).
import { dev } from '$app/environment';
import type { RequestHandler } from './$types';

interface Entry {
	t?: string;
	scope?: string;
	level?: string;
	msg?: string;
	data?: unknown;
}

const safe = (d: unknown) => {
	if (d === undefined) return '';
	try {
		return ' ' + (typeof d === 'string' ? d : JSON.stringify(d));
	} catch {
		return ' [unserializable]';
	}
};

export const POST: RequestHandler = async ({ request }) => {
	if (!dev) return new Response(null, { status: 204 });
	let entries: Entry | Entry[];
	try {
		entries = await request.json();
	} catch {
		return new Response('bad json', { status: 400 });
	}
	const list = Array.isArray(entries) ? entries : [entries];
	// `prompt`-scoped entries get their own clean, readable log (prompts.log) so the full
	// type-a-prompt → ops history is easy to scan for failures + decide what to train next.
	const prompts = list.filter((e) => e.scope === 'prompt');
	const rest = list.filter((e) => e.scope !== 'prompt');
	try {
		const fs = await import('node:fs');
		const path = await import('node:path');
		if (rest.length) {
			const lines =
				rest
					.map((e) => `${e.t ?? ''} [${e.scope ?? '?'}] ${(e.level ?? 'log').toUpperCase()}: ${e.msg ?? ''}${safe(e.data)}`)
					.join('\n') + '\n';
			fs.appendFileSync(path.resolve('debug.log'), lines);
		}
		if (prompts.length) {
			const lines =
				prompts
					.map((e) => {
						// eslint-disable-next-line @typescript-eslint/no-explicit-any
						const d = (e.data ?? {}) as any;
						const hasOps = Array.isArray(d.ops) && d.ops.length > 0;
						// show raw model output when nothing valid came back — that's the failure to learn from
						const out = hasOps ? JSON.stringify(d.ops) : d.error ? `ERROR ${d.error}` : `(no valid ops) raw=${d.raw ?? '?'}`;
						return `${e.t ?? ''} [${d.model ?? '?'}]${d.ms != null ? ` (${d.ms}ms)` : ''} "${e.msg ?? ''}"\n   → ${out}`;
					})
					.join('\n') + '\n';
			fs.appendFileSync(path.resolve('prompts.log'), lines);
		}
	} catch {
		/* ignore */
	}
	return new Response(null, { status: 204 });
};

// Clear the log (handy between debugging sessions).
export const DELETE: RequestHandler = async () => {
	if (dev) {
		try {
			const fs = await import('node:fs');
			const path = await import('node:path');
			fs.writeFileSync(path.resolve('debug.log'), '');
		} catch {
			/* ignore */
		}
	}
	return new Response(null, { status: 204 });
};
