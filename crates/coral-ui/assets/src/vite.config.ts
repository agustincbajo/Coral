import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";

// NOTE(coral-ui frontend): output goes to ../dist so the Rust crate's
// include_dir!("$CARGO_MANIFEST_DIR/assets/dist") picks it up at compile time.
export default defineConfig({
  plugins: [react()],
  base: "/",
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    sourcemap: false,
    chunkSizeWarningLimit: 600,
    rollupOptions: {
      output: {
        manualChunks: {
          sigma: [
            "sigma",
            "@react-sigma/core",
            "graphology",
            "graphology-layout-forceatlas2",
            "graphology-layout-noverlap",
          ],
          markdown: ["react-markdown", "rehype-sanitize", "remark-gfm"],
          // mermaid is dynamically imported in MarkdownRenderer.
        },
      },
    },
  },
  server: {
    port: 5173,
    proxy: {
      "/api": "http://localhost:3838",
      "/health": "http://localhost:3838",
    },
  },
});
