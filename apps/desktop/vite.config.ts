import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// The translations live at the root of the repository (`locales/`), shared
// with the Rust side, so Vite may read files from there.
export default defineConfig(() => ({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 5173, strictPort: true, fs: { allow: ["..", "../.."] } },
  build: { target: "es2022", sourcemap: false },
  test: { environment: "node", include: ["tests/**/*.test.ts"] },
}));
