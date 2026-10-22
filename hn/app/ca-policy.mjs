// Defense in depth for Chromium's separate certificate verifier. Never turn
// an invalid chain into a trusted one: preserve Chromium's original errorCode.
// The entire online backend remains paused until this is tested against real
// Electron certificate chains and Node's compromised root has been removed.
const blockedRootName = 'emSign ECC Root CA - G3';

export function hasBlockedIssuer(certificate) {
    const visited = new Set();
    let current = certificate;
    for (let depth = 0; current && depth < 16; depth++) {
        if (visited.has(current)) return true; // malformed chain: fail closed
        visited.add(current);
        if (current.subject?.commonName === blockedRootName ||
            current.issuer?.commonName === blockedRootName) return true;
        current = current.issuerCert;
    }
    return Boolean(current); // unreasonably deep chain: fail closed
}

export function certificateDecision(request) {
    if (!request?.certificate || hasBlockedIssuer(request.certificate) ||
        hasBlockedIssuer(request.validatedCertificate)) return -2;
    // 0 means Chromium already verified this chain. Return existing failures
    // unchanged; never grant trust to a self-signed/expired/revoked certificate.
    if (request.errorCode === 0) return 0;
    return Number.isInteger(request.errorCode) && request.errorCode < 0
        ? request.errorCode : -2;
}
