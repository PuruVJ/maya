// Browser → file debug pipe. Explicit logs, uncaught errors, AND library console.warn/
// console.error are batched to the dev-only /api/debug endpoint, which appends them to
// ./debug.log so they can be read directly without the user re-describing anything.
// No-ops gracefully in production.

interface Entry {
	t: string;
	scope: string;
	level: 'log' | 'warn' | 'error';
	msg: string;
	data?: unknown;
}

// Capture the real console methods at module load, before any patching, so our own
// mirroring never recurses through the patched versions.
const raw =
	typeof console !== 'undefined'
		? {
				log: console.log.bind(console),
				warn: console.warn.bind(console),
				error: console.error.bind(console)
			}
		: { log: () => {}, warn: () => {}, error: () => {} };

let queue: Entry[] = [];
let timer: ReturnType<typeof setTimeout> | null = null;
let started = false;

function send(immediate = false) {
	if (timer) {
		clearTimeout(timer);
		timer = null;
	}
	if (queue.length === 0) return;
	if (!immediate) {
		timer = setTimeout(() => {
			timer = null;
			send(true);
		}, 400);
		return;
	}
	const batch = queue;
	queue = [];
	fetch('/api/debug', {
		method: 'POST',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify(batch),
		keepalive: true
	}).catch(() => {
		/* dev pipe only; ignore failures */
	});
}

function record(scope: string, level: Entry['level'], msg: string, data: unknown, print: boolean) {
	if (print) (raw[level] ?? raw.log)(`[${scope}] ${msg}`, data ?? '');
	queue.push({ t: new Date().toISOString(), scope, level, msg, data });
	send(level === 'error'); // errors flush immediately
}

export const dlog = (scope: string, msg: string, data?: unknown) => record(scope, 'log', msg, data, true);
export const dwarn = (scope: string, msg: string, data?: unknown) => record(scope, 'warn', msg, data, true);
export const derror = (scope: string, msg: string, data?: unknown) => record(scope, 'error', msg, data, true);

function fmtArg(a: unknown): string {
	if (typeof a === 'string') return a;
	if (a instanceof Error) return a.message;
	try {
		return JSON.stringify(a);
	} catch {
		return String(a);
	}
}

export function initDebug() {
	if (started || typeof window === 'undefined') return;
	started = true;

	// Forward library console.warn/error (e.g. three.js deprecations) to the pipe.
	console.warn = (...args: unknown[]) => {
		raw.warn(...args);
		record('console', 'warn', args.map(fmtArg).join(' '), undefined, false);
	};
	console.error = (...args: unknown[]) => {
		raw.error(...args);
		record('console', 'error', args.map(fmtArg).join(' '), undefined, false);
	};

	window.addEventListener('error', (e) => {
		record('window', 'error', e.message, { src: e.filename, line: e.lineno, col: e.colno, stack: e.error?.stack }, false);
	});
	window.addEventListener('unhandledrejection', (e) => {
		record('window', 'error', 'unhandledrejection: ' + String(e.reason), { stack: (e.reason as Error)?.stack }, false);
	});
	window.addEventListener('beforeunload', () => {
		if (queue.length && navigator.sendBeacon) {
			navigator.sendBeacon('/api/debug', new Blob([JSON.stringify(queue)], { type: 'application/json' }));
			queue = [];
		}
	});

	dlog('app', 'debug pipe initialized', { ua: navigator.userAgent, url: location.href });
}
