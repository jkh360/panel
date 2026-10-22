import assert from 'node:assert/strict';
import test from 'node:test';
import { certificateDecision, hasBlockedIssuer } from './ca-policy.mjs';

const safe = { subject: { commonName: 'example.org' }, issuer: { commonName: 'Known Safe Root' } };
const compromisedIntermediate = {
    subject: { commonName: 'Compromised Intermediate' },
    issuer: { commonName: 'emSign ECC Root CA - G3' },
};

test('does not elevate an invalid certificate to trusted', () => {
    assert.equal(certificateDecision({ certificate: safe, errorCode: 0 }), 0);
    assert.equal(certificateDecision({ certificate: safe, errorCode: -202 }), -202);
    assert.equal(certificateDecision({ certificate: safe, errorCode: undefined }), -2);
    assert.equal(certificateDecision({ errorCode: 0 }), -2);
});

test('rejects a compromised issuer with or without a supplied root cert', () => {
    const leaf = { subject: { commonName: 'example.org' }, issuerCert: compromisedIntermediate };
    assert.equal(hasBlockedIssuer(leaf), true);
    assert.equal(certificateDecision({ certificate: leaf, errorCode: 0 }), -2);
    assert.equal(certificateDecision({ certificate: safe, validatedCertificate: leaf, errorCode: 0 }), -2);
});

test('rejects malformed and cyclic chains', () => {
    const cycle = { subject: { commonName: 'example.org' } };
    cycle.issuerCert = cycle;
    assert.equal(certificateDecision({ certificate: cycle, errorCode: 0 }), -2);
    let long = safe;
    for (let i = 0; i < 18; i++) long = { issuerCert: long };
    assert.equal(certificateDecision({ certificate: long, errorCode: 0 }), -2);
});
