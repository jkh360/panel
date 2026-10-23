import test from 'node:test';
import assert from 'node:assert/strict';
import { remoteHeaders, safeHeaderName, sameOriginWebsite, websiteURL } from './net-policy.mjs';

test('website URLs reject local targets, credentials, and non-web schemes', () => {
    for (const url of [
        'http://localhost:1234/', 'http://foo.local/', 'http://localhost./',
        'http://127.0.0.1/', 'http://2130706433/', 'http://[::ffff:7f00:1]/',
        'http://192.168.1.1/', 'file:///etc/passwd', 'https://name:secret@example.org/',
        'https://example.org@localhost/', 'https://example.org/'.padEnd(4100, 'x'),
    ]) assert.throws(() => websiteURL(url), url);
    assert.equal(websiteURL('https://example.org/chapter?a=1').hostname, 'example.org');
    assert.equal(websiteURL('http://sub.example.org/').protocol, 'http:');
});

test('remote navigations and redirects stay at their requested origin', () => {
    const origin = 'https://example.org';
    assert.equal(sameOriginWebsite('https://example.org/chapter', origin), true);
    for (const url of ['https://example.org.evil.test/', 'https://localhost/',
        'https://127.0.0.1/', 'https://example.org:444/', 'http://example.org/',
        'file:///etc/passwd', 'https://other.test/']) {
        assert.equal(sameOriginWebsite(url, origin), false, url);
    }
    assert.equal(sameOriginWebsite('https://example.org/', undefined), false);
});

test('remote headers cannot carry authentication or smuggle new headers', () => {
    assert.equal(remoteHeaders('Accept: text/html\nCookie: private\nX-FetchAPI-Authorization: secret\nHost: internal\nX-Test: hi'),
        'Accept: text/html\nX-Test: hi');
    assert.equal(safeHeaderName('x-fetchapi-Proxy-Authorization'), false);
    assert.equal(remoteHeaders(undefined), undefined);
    for (const raw of ['X-Test: ok\r\nCookie: sneaky\rX: bad', 'X-Test: x\n Evil: injected',
        'X-Test: \u0000', ': bad', 'X-Test: value'.repeat(1000)]) {
        assert.throws(() => remoteHeaders(raw));
    }
});
