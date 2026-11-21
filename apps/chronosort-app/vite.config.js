import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Offline app: everything is bundled locally, nothing is loaded from the network.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { target: "es2022", outDir: "dist", emptyOutDir: true },
});
