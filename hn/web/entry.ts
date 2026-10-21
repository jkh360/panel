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

installFetchBridge();
const storage = CreateStorageController();
const settings = new SettingsManager(storage);
const plugins = new PluginController(storage, settings);
const flags = new FeatureFlags(settings);
SetupFetchProvider(flags);
const ready = flags.Initialize();

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
