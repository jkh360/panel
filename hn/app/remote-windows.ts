import { BrowserWindow } from 'electron';
import fs from 'node:fs';
import { randomUUID } from 'node:crypto';
import path from 'node:path';
import type { IPC } from '../../vendor/haruneko/app/electron/src/ipc/InterProcessCommunication';
import { Channels } from '../../vendor/haruneko/app/electron/src/ipc/InterProcessCommunicationChannels';
import { guardRemoteNavigation, remoteHeaders, websiteURL } from './net-policy.mjs';

// Unlike upstream's controller, never accept renderer-supplied webPreferences
// or expose arbitrary existing BrowserWindows by ID. The caller is restricted
// to Panel's trusted engine renderer by SafeIPC in main.ts.
async function bounded<T>(win: BrowserWindow, operation: Promise<T>, ms: number): Promise<T> {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const deadline = new Promise<never>((_resolve, reject) => {
        timer = setTimeout(() => {
            if (!win.isDestroyed()) win.destroy();
            reject(new Error('remote browser timeout'));
        }, ms);
    });
    try {
        return await Promise.race([operation, deadline]);
    } finally {
        clearTimeout(timer);
    }
}

export function registerRemoteWindows(ipc: IPC): void {
    const windows = new Map<number, { window: BrowserWindow; preload?: string; allowedOrigin?: string; loading?: boolean }>();
    const get = (id: number): BrowserWindow => {
        const win = windows.get(id)?.window;
        if (!win || win.isDestroyed()) throw new Error('unknown browser window');
        return win;
    };
    ipc.Handle(Channels.RemoteBrowserWindowController.OpenWindow, async (options: string) => {
        if (options.length > 16_384 || windows.size >= 3) throw new Error('browser window limit');
        const requested = JSON.parse(options);
        let preload: string | undefined;
        const script = requested.webPreferences?.preload;
        if (script) {
            if (typeof script !== 'string' || script.length > 262_144) throw new Error('invalid preload');
            preload = path.join('/tmp', `panel-hn-preload-${randomUUID()}.js`);
            fs.writeFileSync(preload, script, { mode: 0o600, flag: 'wx' });
        }
        const win = new BrowserWindow({
            show: false,
            width: 1000,
            height: 800,
            webPreferences: {
                sandbox: true,
                webSecurity: true,
                nodeIntegration: false,
                contextIsolation: !preload,
                backgroundThrottling: false,
                preload,
            },
        });
        win.setMenuBarVisibility(false);
        win.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
        win.webContents.session.setPermissionRequestHandler((_webContents, _permission, callback) => callback(false));
        win.webContents.session.setPermissionCheckHandler(() => false);
        // A redirect must not carry connector headers to another origin.
        guardRemoteNavigation(win.webContents, () => windows.get(win.id)?.allowedOrigin);
        win.webContents.on('did-start-navigation', (_event, url, isMainFrame, isSameDocument) => {
            ipc.Send(Channels.RemoteBrowserWindowController.OnBeforeNavigate, win.id, url, isMainFrame, isSameDocument);
        });
        win.webContents.on('dom-ready', () => ipc.Send(Channels.RemoteBrowserWindowController.OnDomReady, win.id));
        windows.set(win.id, { window: win, preload });
        win.once('closed', () => {
            windows.delete(win.id);
            if (preload) fs.rmSync(preload, { force: true });
        });
        return win.id;
    });
    ipc.Handle(Channels.RemoteBrowserWindowController.LoadURL, async (id: number, url: string, options: string) => {
        const uri = websiteURL(url);
        if (typeof options !== 'string' || options.length > 16_384) throw new Error('invalid load options');
        const requested = JSON.parse(options);
        if (!requested || typeof requested !== 'object' || Array.isArray(requested)) {
            throw new Error('invalid load options');
        }
        const entry = windows.get(id);
        if (!entry || entry.window.isDestroyed()) throw new Error('unknown browser window');
        if (entry.loading) throw new Error('browser window busy');
        const loadOptions = {
            userAgent: typeof requested.userAgent === 'string' ? requested.userAgent.slice(0, 512) : undefined,
            httpReferrer: typeof requested.httpReferrer === 'string' && /^https?:\/\//i.test(requested.httpReferrer)
                ? websiteURL(requested.httpReferrer).href.slice(0, 2048) : undefined,
            extraHeaders: remoteHeaders(requested.extraHeaders),
        };
        // Do not change the redirect origin beneath an in-flight load.
        entry.loading = true;
        entry.allowedOrigin = uri.origin;
        try {
            await bounded(entry.window, entry.window.loadURL(uri.href, loadOptions), 30_000);
        } finally {
            entry.loading = false;
        }
    });
    ipc.Handle(Channels.RemoteBrowserWindowController.CloseWindow, async (id: number) => get(id).destroy());
    ipc.Handle(Channels.RemoteBrowserWindowController.SetVisibility, async (_id: number, _show: boolean) => {
        // No interactive captcha or desktop GUI. Unsupported connectors time out.
    });
    ipc.Handle(Channels.RemoteBrowserWindowController.ExecuteScript, async (id: number, script: string) => {
        if (typeof script !== 'string' || script.length > 262_144) throw new Error('script limit');
        const win = get(id);
        const result = await bounded(win, win.webContents.executeJavaScript(script, true), 15_000);
        // Bound what the pinned connector can relay into Panel's renderer.
        // Electron must deserialize the result first; this is not a RAM cap.
        const encoded = JSON.stringify(result);
        if (encoded && Buffer.byteLength(encoded) > 2_000_000) throw new Error('script result limit');
        return result;
    });
    ipc.Handle(Channels.RemoteBrowserWindowController.SendDebugCommand, async () => {
        throw new Error('remote debugging disabled');
    });
}
