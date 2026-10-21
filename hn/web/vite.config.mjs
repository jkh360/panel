import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
import { defineConfig } from '../../vendor/haruneko/node_modules/vite/dist/node/index.js';

const root = dirname(fileURLToPath(import.meta.url));
export default defineConfig({
    root,
    resolve: {
        alias: [{
            find: '../frontend/classic/FrontendInfo',
            replacement: resolve(root, 'no-frontend.ts'),
        }],
    },
    base: './',
    build: {
        outDir: resolve(root, '../generated/web'),
        emptyOutDir: true,
        sourcemap: false,
        rolldownOptions: {
            input: resolve(root, 'index.html'),
            output: { minify: { mangle: { keepNames: true } } },
        },
    },
});
