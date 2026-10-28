// The extension's tests: the real-browser files apart, so the browser is
// warmed up (`test/warm-up.js`) only when one of them runs.
import { defineConfig } from "vitest/config";

/** The test files that start a real browser (`test/browser.js`). */
const BROWSER = ["test/click-keys.test.js", "test/video-button.test.js"];

export default defineConfig({
  test: {
    projects: [
      { test: { name: "unit", include: ["test/**/*.test.js"], exclude: BROWSER } },
      { test: { name: "browser", include: BROWSER, globalSetup: ["test/warm-up.js"] } },
    ],
  },
});
