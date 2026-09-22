import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [vue()],

  // Vite options tailored for Tauri development
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  // Persistent build cache: avoids re-transforming unchanged source files.
  // Vite stores per-file transform caches keyed by content hash.
  cacheDir: ".build/.vite",

  build: {
    target: "chrome120",
    cssCodeSplit: true,
    minify: "esbuild",
    cssMinify: "lightningcss",
    reportCompressedSize: false,
    modulePreload: {
      polyfill: false,
    },
    rollupOptions: {
      output: {
        experimentalMinChunkSize: 4096,
        manualChunks(id: string) {
          if (id.includes('node_modules')) {
            // Separate Vue framework from other deps for stable caching
            if (id.includes('vue') || id.includes('vue-router') || id.includes('pinia')) {
              return 'framework'
            }
            return 'vendor'
          }
        },
      },
    },
  },
});
