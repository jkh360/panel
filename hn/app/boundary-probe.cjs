// Offline-only Electron test: real senderFrame identities and redirect events.
// Launched only inside the no-route namespace and production filesystem sandbox.
const { app, BrowserWindow, ipcMain, protocol, session } = require('electron');
const http = require('node:http');
const fs = require('node:fs');
const { pathToFileURL } = require('node:url');

protocol.registerSchemesAsPrivileged([{ scheme: 'panel-hn', privileges: {
    standard: true, secure: true, supportFetchAPI: true,
}}]);
app.disableHardwareAcceleration();
app.on('window-all-closed', () => {});
fs.mkdirSync('/tmp/panel-boundary-profile', { recursive: true, mode: 0o700 });
app.setPath('userData', '/tmp/panel-boundary-profile');
app.commandLine.appendSwitch('host-resolver-rules',
    'MAP panel-probe.test 127.0.0.1, MAP other-probe.test 127.0.0.1');

async function main() {
    const { isTrustedSender } = await import(pathToFileURL('/app/backend/app/ipc-policy.mjs').href);
    const { guardRemoteNavigation, websiteURL } = await import(pathToFileURL('/app/backend/app/net-policy.mjs').href);
    const backendSession = session.fromPartition('panel-hn-ephemeral');
    backendSession.protocol.handle('panel-hn', request => new Response(request.url.endsWith('/iframe.html')
        ? '<!doctype html><title>iframe</title>'
        : '<!doctype html><iframe src="panel-hn://app/iframe.html"></iframe>', {
        headers: { 'Content-Type': 'text/html' },
    }));
    const mainWindow = new BrowserWindow({ show: false, webPreferences: {
        sandbox: true, contextIsolation: true, nodeIntegration: false,
        nodeIntegrationInSubFrames: true, webSecurity: true,
        partition: 'panel-hn-ephemeral', preload: '/app/backend/app/boundary-preload.cjs',
    } });
    ipcMain.handle('panel-probe:privileged', event => {
        if (!isTrustedSender(event, mainWindow.webContents)) throw new Error('untrusted IPC sender');
        return 'allowed';
    });
    await mainWindow.loadURL('panel-hn://app/index.html');
    const main = await mainWindow.webContents.executeJavaScript('panelProbe.invoke()');
    const iframe = mainWindow.webContents.mainFrame.frames[0];
    if (!iframe) throw new Error('fixture iframe not loaded');
    const subframe = await iframe.executeJavaScript('panelProbe.invoke().catch(error => error.message)');
    mainWindow.destroy();

    const hits = { safe: 0, cross: 0 };
    const server = http.createServer((request, response) => {
        if (request.url === '/safe') { hits.safe++; response.end('safe'); }
        else if (request.url === '/cross') { hits.cross++; response.end('CROSS-ORIGIN LEAK'); }
        else if (request.url === '/redirect-safe') {
            response.writeHead(302, { Location: '/safe' }); response.end();
        } else if (request.url === '/redirect-cross') {
            response.writeHead(302, { Location: `http://other-probe.test:${server.address().port}/cross` }); response.end();
        } else response.end('initial');
    });
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const origin = `http://panel-probe.test:${server.address().port}`;
    const remoteWindow = new BrowserWindow({ show: false, webPreferences: {
        sandbox: true, contextIsolation: true, nodeIntegration: false, webSecurity: true,
    } });
    guardRemoteNavigation(remoteWindow.webContents, () => websiteURL(origin).origin);
    const events = [];
    for (const name of ['will-navigate', 'will-redirect', 'did-fail-load', 'destroyed']) {
        remoteWindow.webContents.on(name, (event, url) => events.push({ name, url: String(url).slice(0, 200), prevented: event.defaultPrevented }));
    }
    let safeLoaded = false;
    let safeError;
    try { await remoteWindow.loadURL(`${origin}/redirect-safe`); safeLoaded = true; }
    catch (error) { safeError = error.code || error.message; }
    let crossNavigatePrevented = false;
    if (safeLoaded) {
        const navigation = new Promise((resolve, reject) => {
            const timer = setTimeout(() => reject(new Error('navigation event missing')), 5000);
            remoteWindow.webContents.once('will-navigate', event => {
                clearTimeout(timer);
                resolve(event.defaultPrevented);
            });
        });
        await remoteWindow.webContents.executeJavaScript(
            `window.location.href = ${JSON.stringify(`http://other-probe.test:${server.address().port}/cross`)}`);
        crossNavigatePrevented = await navigation;
    }
    let crossLoaded = false;
    let crossError;
    try { await remoteWindow.loadURL(`${origin}/redirect-cross`); crossLoaded = true; }
    catch (error) { crossError = error.code || error.message; }
    const result = { main, subframe, safeLoaded, crossLoaded, crossNavigatePrevented, safeError, crossError, hits, events };
    remoteWindow.destroy();
    server.close();
    console.log('BOUNDARY_PROBE=' + JSON.stringify(result));
    app.quit();
}
app.whenReady().then(main).catch(error => {
    console.error('boundary probe failed:', error instanceof Error ? error.message : 'unknown');
    app.exit(2);
});
