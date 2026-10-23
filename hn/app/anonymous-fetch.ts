import type { IPC } from '../../vendor/haruneko/app/electron/src/ipc/InterProcessCommunication';
import { Channels } from '../../vendor/haruneko/app/electron/src/ipc/InterProcessCommunicationChannels';

// The pinned engine expects these channels. Unlike upstream's FetchProvider,
// do not read session cookies or rewrite concealed request headers globally.
// The separate PanelHN::Fetch bridge handles anonymous requests with explicit
// limits and strips Cookie/Authorization. Login-only connectors fail visibly.
export function registerAnonymousFetch(ipc: IPC): void {
    ipc.Handle(Channels.FetchProvider.Initialize, (prefix: string) => {
        if (typeof prefix !== 'string' || prefix.length > 80) throw new Error('invalid prefix');
    });
    ipc.Handle(Channels.FetchProvider.GetSessionCookies, async () => []);
}
