import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  plugins: [svelte()],
  server: {
    port: 5173,
    strictPort: true
  },
  build: {
    outDir: "../static",
    emptyOutDir: false,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (!id.includes("node_modules")) {
            return;
          }

          if (id.includes("@tauri-apps/api")) {
            return "tauri";
          }

          if (
            id.includes("marked") ||
            id.includes("highlight.js") ||
            id.includes("dompurify")
          ) {
            return "markdown";
          }

          if (id.includes("lucide-svelte")) {
            return "icons";
          }

          if (id.includes("/svelte/")) {
            return "svelte";
          }

          return "vendor";
        }
      }
    }
  }
});
