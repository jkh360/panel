import test from 'node:test';
import assert from 'node:assert/strict';
import { decodeField, encodeItems } from './metadata-wire.mjs';

test('unicode and separators round-trip as opaque metadata', () => {
    const id = 'a\t\n';
    const title = '登山\t\n\u001b[31m';
    const line = encodeItems(1, [{ id, title }]);
    assert.equal(line.split('\n').length, 3);
    assert.equal(line.split('\t').length, 5);
    const [, , encodedId, encodedTitle] = line.split('\n')[0].split('\t');
    assert.equal(decodeField(encodedId, 1024), id);
    assert.equal(decodeField(encodedTitle, 2048), title);
});

test('rejects malformed encodings, oversized replies and unsafe fields', () => {
    assert.throws(() => decodeField('%%%=', 100));
    assert.throws(() => decodeField('a', 100));
    assert.throws(() => decodeField(Buffer.from('too long').toString('base64'), 3));
    assert.throws(() => encodeItems(1, Array(2001).fill({ id: 'x', title: 'y' })));
    assert.equal(encodeItems(1, [{ id: '', title: 'missing' }]), '1\tend\n');
    assert.equal(encodeItems(1, [{ id: 'x', title: 'y'.repeat(513) }]), '1\tend\n');
});
