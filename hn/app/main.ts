import { app, BrowserWindow, net, protocol, session } from 'electron';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import readline from 'node:readline';
import { IPC } from '../../vendor/haruneko/app/electron/src/ipc/InterProcessCommunication';
import { FetchProvider } from '../../vendor/haruneko/app/electron/src/ipc/FetchProvider';

if (process.env.PANEL_HN_NET_ISOLATED !== '1') {
    console.error('panel-hn: refusing to run outside hn/net-sandbox.sh');
    process.exit(1);
}

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
    new FetchProvider(new IPC(browser), browser);
    await win.loadURL('panel-hn://app/index.html');

    // One JSON request per stdin line, one JSON reply per stdout line. No network
    // listener or browser remote-debugging API. Only whitelisted operations.
    const input = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
    let queue = Promise.resolve();
    input.on('line', line => {
        queue = queue.then(async () => {
            let id: unknown = null;
            try {
                const request = JSON.parse(line);
                id = request.id ?? null;
                if (request.op !== 'sources') throw new Error('unknown operation');
                const sources = await browser.executeJavaScript('globalThis.panelHN.sources()');
                process.stdout.write(JSON.stringify({ id, sources }) + '\n');
            } catch {
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
