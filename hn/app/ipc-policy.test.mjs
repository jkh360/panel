import test from 'node:test';
import assert from 'node:assert/strict';
import { isTrustedSender } from './ipc-policy.mjs';

test('only the local top-level renderer can invoke privileged IPC', () => {
    const mainFrame = { url: 'panel-hn://app/index.html' };
    const browser = { mainFrame };
    assert.equal(isTrustedSender({ sender: browser, senderFrame: mainFrame }, browser), true);
    assert.equal(isTrustedSender({ sender: browser, senderFrame: { url: 'https://site.invalid/' } }, browser), false);
    assert.equal(isTrustedSender({ sender: browser, senderFrame: { url: mainFrame.url } }, browser), false);
    assert.equal(isTrustedSender({ sender: {}, senderFrame: mainFrame }, browser), false);
    assert.equal(isTrustedSender({ sender: browser, senderFrame: null }, browser), false);
    mainFrame.url = 'https://site.invalid/';
    assert.equal(isTrustedSender({ sender: browser, senderFrame: mainFrame }, browser), false);
});
