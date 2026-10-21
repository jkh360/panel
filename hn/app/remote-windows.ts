import { BrowserWindow } from 'electron';
import fs from 'node:fs';
import { randomUUID } from 'node:crypto';
import path from 'node:path';
import type { IPC } from '../../vendor/haruneko/app/electron/src/ipc/InterProcessCommunication';
import { Channels } from '../../vendor/haruneko/app/electron/src/ipc/InterProcessCommunicationChannels';

// Unlike upstream's controller, never accept renderer-supplied webPreferences
// or expose arbitrary existing BrowserWindows by ID. The caller is restricted
// to Panel's trusted engine renderer by SafeIPC in main.ts.
export function registerRemoteWindows(ipc: IPC): void {
    const windows = new Map<number, { window: BrowserWindow; preload?: string }>();
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
        win.webContents.on('will-navigate', (event, url) => {
            if (!/^https?:\/\//i.test(url)) event.preventDefault();
        });
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
        if (!/^https?:\/\//i.test(url)) throw new Error('unsupported website URL');
        const requested = JSON.parse(options);
        await get(id).loadURL(url, {
            userAgent: typeof requested.userAgent === 'string' ? requested.userAgent.slice(0, 512) : undefined,
            httpReferrer: typeof requested.httpReferrer === 'string' && /^https?:\/\//i.test(requested.httpReferrer)
                ? requested.httpReferrer.slice(0, 2048) : undefined,
            extraHeaders: typeof requested.extraHeaders === 'string' ? requested.extraHeaders.slice(0, 8192) : undefined,
        });
    });
    ipc.Handle(Channels.RemoteBrowserWindowController.CloseWindow, async (id: number) => get(id).destroy());
    ipc.Handle(Channels.RemoteBrowserWindowController.SetVisibility, async (_id: number, _show: boolean) => {
        // No interactive captcha or desktop GUI. Unsupported connectors time out.
    });
    ipc.Handle(Channels.RemoteBrowserWindowController.ExecuteScript, async (id: number, script: string) => {
        if (typeof script !== 'string' || script.length > 262_144) throw new Error('script limit');
        return get(id).webContents.executeJavaScript(script, true);
    });
    ipc.Handle(Channels.RemoteBrowserWindowController.SendDebugCommand, async () => {
        throw new Error('remote debugging disabled');
    });
}
