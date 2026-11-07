import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The app must never load anything from another origin (see AGENTS.md,
// privacy rules). Keep every asset bundled; no CDN URLs.
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      "/api": "http://localhost:8080",
    },
  },
  build: {
    outDir: "dist",
    sourcemap: false,
  },
});
