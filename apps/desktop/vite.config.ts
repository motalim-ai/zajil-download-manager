import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// The translations live at the root of the repository (`locales/`), shared
// with the Rust side, so Vite may read files from there.
export default defineConfig(({ mode }) => ({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 5173, strictPort: true, fs: { allow: ["..", "../.."] } },
  define: {
    // Test pages exist only in `vite build --mode ui-test` (CI checks that
    // a release build has none).
    __UI_TEST__: JSON.stringify(mode === "ui-test"),
  },
  build: { target: "es2022", sourcemap: false },
  test: { environment: "node", include: ["tests/**/*.test.ts"] },
}));
