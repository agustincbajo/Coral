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
        // Fixed filenames (no content hash) so the bundle is byte-stable
        // across OS/Node versions. The Rust binary embeds these via
        // `include_dir!`, and the CI drift check compares the committed
        // dist/ against a fresh CI build — content-hashed names made
        // that diff thrash. Cache-busting still works because we set
        // `Cache-Control: no-cache` on index.html and let the assets/
        // subtree be `max-age=31536000, immutable` per filename.
        entryFileNames: "assets/[name].js",
        chunkFileNames: "assets/[name].js",
        assetFileNames: "assets/[name].[ext]",
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
