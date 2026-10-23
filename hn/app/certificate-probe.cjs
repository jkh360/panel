// Offline-only Electron probe. Launched by hn/test-certificates.sh inside a
// network namespace with no external route and the production filesystem
// sandbox; NEVER use this as the application entry point.
const { app, BrowserWindow, session } = require('electron');
const https = require('node:https');
const fs = require('node:fs');
const tls = require('node:tls');
const { X509Certificate } = require('node:crypto');
const { pathToFileURL } = require('node:url');

const fixture = '/app/backend/app/certificate-fixture';
const mismatch = process.argv.includes('--mismatch');
app.disableHardwareAcceleration();
app.setPath('userData', '/tmp/panel-ca-profile');

async function main() {
    const { certificateDecision } = await import(pathToFileURL('/app/backend/app/ca-policy.mjs').href);
    const { removeCompromisedNodeRoot } = await import(pathToFileURL('/app/backend/app/node-ca.mjs').href);
    const root = fs.readFileSync(`${fixture}/root.crt`, 'utf8');
    const leaf = fs.readFileSync(`${fixture}/leaf.crt`, 'utf8');
    const key = fs.readFileSync(`${fixture}/leaf.key`, 'utf8');
    // Real servers commonly omit the root; require Electron to recover the
    // issuer path from the trusted local store, not just the wire chain.
    const tlsOptions = { cert: leaf, key };
    const server = https.createServer(tlsOptions, (_, response) => {
        response.writeHead(200, { 'Content-Type': 'text/html', Connection: 'close' });
        response.end('<!doctype html><title>Local certificate test</title>');
    });
    // Only loopback has an interface/route in the enclosing network namespace.
    await new Promise(resolve => server.listen(0, '0.0.0.0', resolve));
    const url = `https://localhost:${server.address().port}/`;
    const before = tls.getCACertificates('default');
    const fingerprint = new X509Certificate(root).fingerprint256;
    const isTestRootPresent = roots => roots.some(pem => new X509Certificate(pem).fingerprint256 === fingerprint);
    const hadAffectedRoot = before.some(pem => /CN=emSign ECC Root CA - G3\b/.test(new X509Certificate(pem).subject));
    tls.setDefaultCACertificates([...before, root]);
    const testRootBefore = isTestRootPresent(tls.getCACertificates('default'));
    const fetchBefore = (await fetch(url)).status;
    removeCompromisedNodeRoot();
    const testRootAfter = isTestRootPresent(tls.getCACertificates('default'));
    const freshServer = https.createServer(tlsOptions, (_, response) => response.end('offline'));
    await new Promise(resolve => freshServer.listen(0, '127.0.0.1', resolve));
    let fetchAfter;
    try {
        fetchAfter = (await fetch(`https://localhost:${freshServer.address().port}/`)).status;
    } catch (error) {
        fetchAfter = error.cause?.code || 'TLS_REJECTED';
    }
    freshServer.close();
    const browsers = [];
    const useBackendSession = process.argv.includes('--backend-session');
    const name = useBackendSession ? 'backend' : 'default';
    const partition = useBackendSession ? 'panel-ca-probe' : undefined;
    const browserSession = useBackendSession ? session.fromPartition(partition) : session.defaultSession;
    const host = mismatch ? '127.0.0.1' : 'localhost';
    {
        let verification;
        browserSession.setCertificateVerifyProc((request, callback) => {
            const decision = certificateDecision(request);
            verification = {
                errorCode: request.errorCode,
                issuer: request.certificate?.issuer?.commonName,
                chainSubject: request.certificate?.issuerCert?.subject?.commonName,
                chainIssuer: request.certificate?.issuerCert?.issuer?.commonName,
                decision,
            };
            callback(decision);
        });
        const window = new BrowserWindow({ show: false, webPreferences: {
            sandbox: true, webSecurity: true, nodeIntegration: false, contextIsolation: true,
            ...(partition ? { partition } : {}),
        } });
        let loaded = false;
        try { await window.loadURL(`https://${host}:${server.address().port}/`); loaded = true; }
        catch { /* expected for blocked chains */ }
        browsers.push({ name, verification, loaded });
        window.destroy();
    }
    console.log('CERT_PROBE=' + JSON.stringify({
        hadAffectedRoot, testRootBefore, fetchBefore, testRootAfter, fetchAfter, browsers,
    }));
    server.close();
    app.quit();
}
app.whenReady().then(main).catch(error => {
    console.error('certificate probe failed:', error instanceof Error ? error.message : 'unknown');
    app.exit(2);
});
