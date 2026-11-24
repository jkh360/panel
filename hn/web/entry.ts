// Only upstream engine components: no GUI, service worker or remote RPC.
// Upstream's web entry installs these required engine-level helpers.
import '../../vendor/haruneko/web/src/engine/ArrayExtensions';
import '../../vendor/haruneko/web/src/engine/RegExpSafe';
import { PluginController } from '../../vendor/haruneko/web/src/engine/PluginController';
import { CreateStorageController } from '../../vendor/haruneko/web/src/engine/StorageController';
import { SettingsManager } from '../../vendor/haruneko/web/src/engine/SettingsManager';
import { FeatureFlags } from '../../vendor/haruneko/web/src/engine/FeatureFlags';
import { SetupFetchProvider } from '../../vendor/haruneko/web/src/engine/platform/FetchProvider';
import { installFetchBridge } from './fetch';
import type { Page } from '../../vendor/haruneko/web/src/engine/providers/MangaPlugin';

installFetchBridge();
const storage = CreateStorageController();
const settings = new SettingsManager(storage);
const plugins = new PluginController(storage, settings);
// The official MANGA Plus API accepts anonymous requests. Upstream's scraper
// opens a remote window solely to read a login token from localStorage; the
// disposable Panel profile has none. Keep the token empty and avoid that
// brittle, unnecessary window while retaining the scraper's normal API calls.
const official = plugins.WebsitePlugins.find(site => site.Identifier === 'shueishamangaplus');
if (official) {
    const scraper = (official as unknown as { scraper: { Initialize(): Promise<void> } }).scraper;
    scraper.Initialize = async () => {};
}
const flags = new FeatureFlags(settings);
SetupFetchProvider(flags);
const ready = flags.Initialize();
let activeDownload: { token: string; pages: Page[] } | undefined;

// Listing a connector does not imply that it is live or tested.
Object.defineProperty(globalThis, 'panelHN', {
    value: Object.freeze({
        sources: async () => {
            await ready;
            return plugins.WebsitePlugins.map(source => ({
                id: source.Identifier,
                title: source.Title,
            }));
        },
        search: async (id: string, query: string) => {
            await ready;
            const source = plugins.WebsitePlugins.find(site => site.Identifier === id);
            if (!source || query.length < 2 || query.length > 120) throw new Error('invalid search');
            await source.Update();
            const needle = query.toLocaleLowerCase();
            return source.Entries.Value.filter(entry => entry.Title.toLocaleLowerCase().includes(needle))
                .slice(0, 50).map(entry => ({ id: entry.Identifier, title: entry.Title }));
        },
        prepareDownload: async (id: string, mangaId: string, mangaTitle: string, chapterId: string) => {
            await ready;
            activeDownload = undefined;
            const source = plugins.WebsitePlugins.find(site => site.Identifier === id);
            if (!source || !mangaId || !mangaTitle || !chapterId) throw new Error('invalid chapter');
            const manga = source.CreateEntry(mangaId, mangaTitle);
            await manga.Update();
            const chapter = manga.Entries.Value.find(item => item.Identifier === chapterId);
            if (!chapter) throw new Error('chapter unavailable');
            await chapter.Update();
            const pages = chapter.Entries.Value;
            if (pages.length < 1 || pages.length > 300) throw new Error('page count limit');
            const token = crypto.randomUUID();
            activeDownload = { token, pages };
            return { token, count: pages.length };
        },
        fetchPage: async (token: string, index: number) => {
            const active = activeDownload;
            if (!active || active.token !== token || !Number.isSafeInteger(index)
                || index < 0 || index >= active.pages.length) throw new Error('invalid page');
            const abort = new AbortController();
            const timer = setTimeout(() => abort.abort(), 90_000);
            try {
                const blob = await active.pages[index].Fetch(4, abort.signal); // Priority.Normal
                if (!(blob instanceof Blob) || blob.size < 16 || blob.size > 12_000_000) {
                    throw new Error('image size limit');
                }
                const bytes = new Uint8Array(await blob.arrayBuffer());
                const parts: string[] = [];
                for (let offset = 0; offset < bytes.length; offset += 8192) {
                    parts.push(String.fromCharCode(...bytes.subarray(offset, offset + 8192)));
                }
                return btoa(parts.join(''));
            } finally {
                clearTimeout(timer);
            }
        },
        finishDownload: (token: string) => {
            if (activeDownload?.token !== token) throw new Error('unknown download');
            activeDownload = undefined;
            return true;
        },
        chapters: async (id: string, mangaId: string, mangaTitle: string) => {
            await ready;
            const source = plugins.WebsitePlugins.find(site => site.Identifier === id);
            if (!source || !mangaId || mangaId.length > 1024 || mangaTitle.length > 1024) {
                throw new Error('invalid chapter request');
            }
            const manga = source.CreateEntry(mangaId, mangaTitle);
            await manga.Update();
            return manga.Entries.Value.slice(0, 2_000)
                .map(chapter => ({ id: chapter.Identifier, title: chapter.Title }));
        },
    }),
    configurable: false,
    writable: false,
});
