// Preserve Chromium's webSecurity. Cross-origin connector requests instead use
// the private Electron IPC pipe; the main process makes an anonymous request.
// Authentication/cookie-dependent sources must fail until explicitly supported.
export function installFetchBridge(): void {
    const nativeFetch = globalThis.fetch.bind(globalThis);
    globalThis.fetch = async (resource: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
        const request = new Request(resource, init);
        if (!/^https?:\/\//i.test(request.url)) return nativeFetch(resource, init);
        const bytes = request.method === 'GET' || request.method === 'HEAD'
            ? new Uint8Array() : new Uint8Array(await request.clone().arrayBuffer());
        if (bytes.byteLength > 2_000_000) throw new Error('request body limit');
        let binary = '';
        for (let offset = 0; offset < bytes.length; offset += 8192) {
            binary += String.fromCharCode(...bytes.subarray(offset, offset + 8192));
        }
        const body = btoa(binary);
        const result = await globalThis.ipcRenderer.invoke('PanelHN::Fetch', {
            url: request.url,
            method: request.method,
            headers: Array.from(request.headers),
            body,
        });
        const decoded = atob(result.body);
        const output = new Uint8Array(decoded.length);
        for (let index = 0; index < decoded.length; index++) output[index] = decoded.charCodeAt(index);
        return new Response(output, { status: result.status, headers: result.headers });
    };
}
