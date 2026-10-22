import { app, BrowserWindow, ipcMain, net, protocol, session } from 'electron';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import readline from 'node:readline';
import { IPC } from '../../vendor/haruneko/app/electron/src/ipc/InterProcessCommunication';
import { FetchProvider } from '../../vendor/haruneko/app/electron/src/ipc/FetchProvider';
import { registerRemoteWindows } from './remote-windows';
import { certificateDecision } from './ca-policy.mjs';
import { removeCompromisedNodeRoot } from './node-ca.mjs';

if (process.env.PANEL_HN_NET_ISOLATED !== '1') {
    console.error('panel-hn: refusing to run outside hn/net-sandbox.sh');
    process.exit(1);
}
// 2026-10-20: eight compromised eMudhra intermediates chain to the G3 root.
// Drop it from Node's trust before any networking, but do NOT enable websites:
// Electron/Chromium's independent verifier still needs real-chain testing.
try {
    removeCompromisedNodeRoot();
} catch {
    console.error('panel-hn: online backend paused: could not constrain CA trust');
    process.exit(1);
}
console.error('panel-hn: online backend paused pending Chromium CA review');
process.exit(1);

// No external asset server, remote app URL, RPC listener, or debugging port.
protocol.registerSchemesAsPrivileged([{ scheme: 'panel-hn', privileges: {
    standard: true, secure: true, supportFetchAPI: true,
}}]);
app.disableHardwareAcceleration();
fs.mkdirSync('/tmp/panel-hn-profile', { recursive: true, mode: 0o700 });
app.setPath('userData', '/tmp/panel-hn-profile');

async function run(): Promise<void> {
    await app.whenReady();
    const assets = path.resolve(app.getAppPath(), '../web');
    const root = fs.realpathSync(assets);
    const backendSession = session.fromPartition('panel-hn-ephemeral');
    for (const browserSession of [session.defaultSession, backendSession]) {
        browserSession.setCertificateVerifyProc((request, callback) =>
            callback(certificateDecision(request)));
    }
    backendSession.protocol.handle('panel-hn', async request => {
        try {
            const uri = new URL(request.url);
            if (uri.hostname !== 'app') return new Response('Not found', { status: 404 });
            const file = fs.realpathSync(path.resolve(root, '.' + decodeURIComponent(uri.pathname)));
            if (!file.startsWith(root + path.sep)) return new Response('Not found', { status: 404 });
            return await net.fetch(pathToFileURL(file).href, { bypassCustomProtocolHandlers: true });
        } catch (error) {
            console.error('panel-hn: local asset error:', error);
            return new Response('Not found', { status: 404 });
        }
    });
    const win = new BrowserWindow({
        show: false,
        webPreferences: {
            preload: path.resolve(app.getAppPath(), 'preload.js'),
            sandbox: true,
            contextIsolation: true,
            nodeIntegration: false,
            webSecurity: true,
            partition: 'panel-hn-ephemeral',
        },
    });
    const browser = win.webContents;
    browser.setWindowOpenHandler(() => ({ action: 'deny' }));
    browser.on('will-navigate', (event, url) => {
        if (!url.startsWith('panel-hn://app/')) event.preventDefault();
    });
    backendSession.setPermissionRequestHandler((_contents, _permission, callback) => callback(false));
    backendSession.setPermissionCheckHandler(() => false);
    class SafeIPC extends IPC {
        public override Handle<TParameters extends JSONArray, TReturn extends JSONElement>(
            channel: string, callback: (...parameters: TParameters) => TReturn | PromiseLike<TReturn>
        ): void {
            ipcMain.handle(channel, (event, ...args: TParameters) => {
                if (event.sender !== browser) throw new Error('untrusted IPC sender');
                return callback(...args);
            });
        }
    }
    const ipc = new SafeIPC(browser);
    new FetchProvider(ipc, browser);
    registerRemoteWindows(ipc);
    ipcMain.handle('PanelHN::Fetch', async (event, payload) => {
        if (event.sender !== browser) throw new Error('untrusted IPC sender');
        if (typeof payload?.url !== 'string' || payload.url.length > 4096) throw new Error('invalid URL');
        const uri = new URL(payload.url);
        if (!['http:', 'https:'].includes(uri.protocol) || uri.username || uri.password) {
            throw new Error('unsupported URL');
        }
        if (!['GET', 'POST', 'HEAD'].includes(payload.method)
            || !Array.isArray(payload.headers) || payload.headers.length > 80
            || typeof payload.body !== 'string' || payload.body.length > 2_800_000) {
            throw new Error('invalid request');
        }
        const headers = new Headers();
        for (const entry of payload.headers) {
            if (!Array.isArray(entry) || entry.length !== 2
                || typeof entry[0] !== 'string' || typeof entry[1] !== 'string'
                || entry[0].length > 100 || entry[1].length > 4096) throw new Error('invalid header');
            const key = entry[0].replace(/^x-fetchapi-/i, '').toLowerCase();
            // Neither browser profile cookies nor upstream connector credentials
            // are sent by the anonymous bridge.
            if (['cookie', 'authorization', 'proxy-authorization', 'host', 'origin',
                'referer', 'sec-fetch-site', 'sec-fetch-mode', 'sec-fetch-dest'].includes(key)) {
                if (key === 'referer' && /^https?:\/\//i.test(entry[1])) headers.set('referer', entry[1]);
                continue;
            }
            headers.set(key, entry[1]);
        }
        const abort = new AbortController();
        const timer = setTimeout(() => abort.abort(), 30_000);
        try {
            const response = await fetch(uri.href, {
                method: payload.method,
                headers,
                body: payload.body ? Buffer.from(payload.body, 'base64') : undefined,
                signal: abort.signal,
                // Never forward connector headers across an unreviewed redirect.
                // Sources requiring redirects fail until a safe policy is added.
                redirect: 'manual',
                credentials: 'omit',
            });
            const chunks: Buffer[] = [];
            let length = 0;
            for await (const chunk of response.body ?? []) {
                length += chunk.byteLength;
                if (length > 24_000_000) throw new Error('response size limit');
                chunks.push(Buffer.from(chunk));
            }
            const outputHeaders = Object.fromEntries([...response.headers].filter(([name]) =>
                !['set-cookie', 'content-encoding', 'content-length', 'transfer-encoding'].includes(name.toLowerCase())));
            return { status: response.status, headers: outputHeaders,
                body: Buffer.concat(chunks).toString('base64') };
        } finally {
            clearTimeout(timer);
        }
    });
    await win.loadURL('panel-hn://app/index.html');

    // One JSON request per stdin line, one JSON reply per stdout line. No network
    // listener or browser remote-debugging API. Only whitelisted operations.
    const input = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
    let queue = Promise.resolve();
    input.on('line', line => {
        queue = queue.then(async () => {
            let id: unknown = null;
            try {
                if (line.length > 16_384) throw new Error('oversized request');
                const request = JSON.parse(line);
                id = typeof request.id === 'number' && Number.isSafeInteger(request.id) ? request.id : null;
                let operation: string;
                if (request.op === 'sources') {
                    operation = 'globalThis.panelHN.sources()';
                } else if (request.op === 'search'
                    && typeof request.source === 'string' && /^[a-z0-9_-]{1,80}$/.test(request.source)
                    && typeof request.query === 'string' && request.query.length >= 2 && request.query.length <= 120) {
                    operation = `globalThis.panelHN.search(${JSON.stringify(request.source)},${JSON.stringify(request.query)})`;
                } else if (request.op === 'chapters'
                    && typeof request.source === 'string' && /^[a-z0-9_-]{1,80}$/.test(request.source)
                    && typeof request.mangaId === 'string' && request.mangaId.length > 0 && request.mangaId.length <= 1024
                    && typeof request.mangaTitle === 'string' && request.mangaTitle.length > 0 && request.mangaTitle.length <= 1024) {
                    operation = `globalThis.panelHN.chapters(${JSON.stringify(request.source)},${JSON.stringify(request.mangaId)},${JSON.stringify(request.mangaTitle)})`;
                } else throw new Error('unknown or invalid operation');
                const results = await browser.executeJavaScript(operation);
                process.stdout.write(JSON.stringify({ id, results }) + '\n');
            } catch {
                // Connector exceptions may contain URLs, credentials or page data;
                // never print them into Panel's log or forward them to the TUI.
                process.stdout.write(JSON.stringify({ id, error: 'request failed' }) + '\n');
            }
        });
    });
    input.on('close', () => void queue.finally(() => app.quit()));
}
run().catch(error => {
    console.error('panel-hn: startup failed:', error instanceof Error ? error.message : 'unknown error');
    app.exit(1);
});
