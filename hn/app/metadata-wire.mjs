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

export function encodeDownloadReply(id, kind, result) {
    if (!Number.isSafeInteger(id)) throw new Error('invalid request');
    if (kind === 'prepared' && Number.isInteger(result?.count)
        && result.count >= 1 && result.count <= 300
        && typeof result.token === 'string' && /^[a-f0-9-]{36}$/.test(result.token)) {
        return `${id}\tprepared\t${result.token}\t${result.count}\n`;
    }
    if (kind === 'page' && typeof result === 'string'
        && result.length >= 24 && result.length <= 16_000_000
        && /^[A-Za-z0-9+/]+={0,2}$/.test(result)) {
        return `${id}\tpage\t${result}\n`;
    }
    if (kind === 'done' && result === true) return `${id}\tdone\n`;
    throw new Error('invalid download response');
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
