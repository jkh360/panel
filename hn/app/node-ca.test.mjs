import assert from 'node:assert/strict';
import test from 'node:test';
import tls from 'node:tls';
import { X509Certificate } from 'node:crypto';
import { removeCompromisedNodeRoot } from './node-ca.mjs';

test('drops affected G3 root without adding trust or changing other roots', () => {
    const before = tls.getCACertificates('default');
    const removed = removeCompromisedNodeRoot();
    const after = tls.getCACertificates('default');
    assert.equal(before.length - removed, after.length);
    assert.ok(after.length > 50);
    assert.equal(after.some(pem =>
        /CN=emSign ECC Root CA - G3\b/.test(new X509Certificate(pem).subject)), false);
    // Node may normalize PEM formatting when replacing the default store.
    const beforeFingerprints = new Set(before.map(pem => new X509Certificate(pem).fingerprint256));
    assert.ok(after.every(pem => beforeFingerprints.has(new X509Certificate(pem).fingerprint256)));
    assert.equal(removeCompromisedNodeRoot(), 0);
});
