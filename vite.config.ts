import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { spzCspPlugin } from "./scripts/spz-csp-plugin.mjs";

export default defineConfig({
  plugins: [react(), spzCspPlugin()],
  worker: { plugins: () => [spzCspPlugin()] },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    // Phase one is Windows-only; WebView2 tracks Chromium.
    target: "chrome105",
    minify: process.env.TAURI_ENV_DEBUG ? false : "esbuild",
    sourcemap: Boolean(process.env.TAURI_ENV_DEBUG),
  },
});
