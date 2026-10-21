// Only upstream engine components: no GUI, service worker or remote RPC.
import { PluginController } from '../../vendor/haruneko/web/src/engine/PluginController';
import { CreateStorageController } from '../../vendor/haruneko/web/src/engine/StorageController';
import { SettingsManager } from '../../vendor/haruneko/web/src/engine/SettingsManager';
import { FeatureFlags } from '../../vendor/haruneko/web/src/engine/FeatureFlags';
import { SetupFetchProvider } from '../../vendor/haruneko/web/src/engine/platform/FetchProvider';

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
    }),
    configurable: false,
    writable: false,
});
