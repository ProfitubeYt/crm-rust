import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import path from 'path'
import pkg from './package.json' with { type: 'json' }

// The Telegram section as a standalone page, embedded into the native client.
export default defineConfig({
    plugins: [react()],
    base: './',
    publicDir: false,
    define: {
        __APP_VERSION__: JSON.stringify(pkg.version),
    },
    resolve: {
        alias: {
            '@': path.resolve(__dirname, './src'),
        },
    },
    build: {
        outDir: 'dist-telegram',
        emptyOutDir: true,
        sourcemap: false,
        assetsDir: 'assets',
        rollupOptions: {
            input: path.resolve(__dirname, 'telegram.html'),
            output: {
                entryFileNames: 'assets/[name]-[hash].js',
                chunkFileNames: 'assets/[name]-[hash].js',
                assetFileNames: 'assets/[name]-[hash].[ext]',
            },
        },
    },
})
