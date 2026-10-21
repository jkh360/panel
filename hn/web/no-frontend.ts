// Upstream SettingsGlobal imports the Classic UI only to read its ID.
// Keep that import from pulling the entire GUI into the backend bundle.
export const Info = {
    ID: 'panel',
    Label: 'Panel backend',
    Description: 'Headless backend without an upstream frontend',
    Screenshots: [],
    LoadModule: async () => { throw new Error('No GUI in Panel backend'); },
};
