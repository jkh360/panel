// Metadata-only pipe for Panel's TUI. Website strings are encoded as base64
// fields so they cannot inject separators or terminal control sequences into
// the line protocol. Rust independently bounds and sanitizes every field.
export function decodeField(value, limit) {
    if (typeof value !== 'string' || value.length > limit * 2
        || !/^[A-Za-z0-9+/]*={0,2}$/.test(value)) throw new Error('invalid field');
    const bytes = Buffer.from(value, 'base64');
    if (bytes.length > limit || bytes.toString('base64') !== value) throw new Error('invalid field');
    return new TextDecoder('utf-8', { fatal: true }).decode(bytes);
}

export function encodeItems(id, results) {
    if (!Number.isSafeInteger(id) || !Array.isArray(results) || results.length > 2_000) {
        throw new Error('invalid metadata');
    }
    const encode = text => Buffer.from(text, 'utf8').toString('base64');
    const lines = [];
    for (const item of results) {
        if (typeof item?.id !== 'string' || typeof item?.title !== 'string'
            || !item.id || item.id.length > 1024 || item.title.length > 512) continue;
        lines.push(`${id}\titem\t${encode(item.id)}\t${encode(item.title)}\n`);
    }
    lines.push(`${id}\tend\n`);
    return lines.join('');
}
