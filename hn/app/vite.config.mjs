import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
import { defineConfig } from '../../vendor/haruneko/node_modules/vite/dist/node/index.js';
const root = dirname(fileURLToPath(import.meta.url));
export default defineConfig({
    root,
    build: {
        ssr: true,
        emptyOutDir: false,
        outDir: resolve(root, '../generated/app'),
        lib: {
            entry: [resolve(root, 'main.ts'), resolve(root, 'preload.ts')],
            formats: ['cjs'],
        },
        rolldownOptions: {
            external: ['electron'],
            output: { entryFileNames: ({ name }) => `${name}.js` },
        },
    },
});
