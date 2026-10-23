import { isIP } from 'node:net';

// Do not let a connector resolve URL syntax to local services. The outer
// network namespace/firewall additionally blocks private DNS answers.
export function websiteURL(value) {
    if (typeof value !== 'string' || value.length > 4096) throw new Error('invalid website URL');
    const uri = new URL(value);
    const host = uri.hostname.replace(/^\[|\]$/g, '').replace(/\.$/, '').toLowerCase();
    if (!['http:', 'https:'].includes(uri.protocol) || uri.username || uri.password ||
        !host || isIP(host) || host === 'localhost' || host.endsWith('.localhost') ||
        host.endsWith('.local')) throw new Error('unsupported website URL');
    return uri;
}

export function sameOriginWebsite(value, allowedOrigin) {
    try {
        return Boolean(allowedOrigin && websiteURL(value).origin === allowedOrigin);
    } catch {
        return false;
    }
}

export function guardRemoteNavigation(contents, allowedOrigin) {
    const check = (event, url) => {
        if (!sameOriginWebsite(url, allowedOrigin())) event.preventDefault();
    };
    contents.on('will-navigate', check);
    contents.on('will-redirect', check);
}

const forbiddenHeaders = new Set([
    'cookie', 'authorization', 'proxy-authorization', 'host', 'origin',
    'sec-fetch-site', 'sec-fetch-mode', 'sec-fetch-dest',
]);

export function safeHeaderName(name) {
    const key = name.replace(/^x-fetchapi-/i, '').toLowerCase();
    return !forbiddenHeaders.has(key) && key !== 'referer' && !key.startsWith('proxy-');
}

export function remoteHeaders(raw) {
    if (raw === undefined) return undefined;
    if (typeof raw !== 'string' || raw.length > 8192) throw new Error('invalid headers');
    const lines = [];
    for (const line of raw.split('\n')) {
        if (!line) continue;
        const match = /^([!#$%&'*+.^_`|~0-9A-Za-z-]+):[ \t]*([^\r\n\x00-\x1f\x7f]*)\r?$/.exec(line);
        if (!match) throw new Error('invalid header');
        if (safeHeaderName(match[1])) lines.push(`${match[1]}: ${match[2]}`);
    }
    return lines.join('\n');
}
