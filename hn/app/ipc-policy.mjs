// A webContents can contain untrusted subframes. IPC privileges belong only
// to Panel's own top-level, locally bundled renderer, never an iframe.
export function isTrustedSender(event, browser) {
    const frame = browser?.mainFrame;
    return Boolean(event?.sender === browser && frame && event.senderFrame === frame &&
        frame.url?.startsWith('panel-hn://app/'));
}
