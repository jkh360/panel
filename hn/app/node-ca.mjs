import tls from 'node:tls';
import { X509Certificate } from 'node:crypto';

// Node's CA bundle is independent of Chromium's. This only hardens anonymous
// Node fetches; the browser remains gated until its certificate path is tested.
export function removeCompromisedNodeRoot() {
    if (typeof tls.getCACertificates !== 'function' ||
        typeof tls.setDefaultCACertificates !== 'function') {
        throw new Error('Node runtime cannot constrain default CA trust');
    }
    const roots = tls.getCACertificates('default');
    const trusted = roots.filter(pem =>
        !/CN=emSign ECC Root CA - G3\b/.test(new X509Certificate(pem).subject));
    if (!trusted.length) throw new Error('no CA roots remain');
    tls.setDefaultCACertificates(trusted);
    if (tls.getCACertificates('default').some(pem =>
        /CN=emSign ECC Root CA - G3\b/.test(new X509Certificate(pem).subject))) {
        throw new Error('affected CA remains trusted');
    }
    return roots.length - trusted.length;
}
