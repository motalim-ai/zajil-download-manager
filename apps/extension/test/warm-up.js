// Vitest's global setup of the real-browser test files (`vitest.config.js`):
// the browser started once, with a page, before they start theirs (#230,
// #296; `warmUp`). In a Node of its own, as the test files' browsers are:
// in Vitest's own process, Playwright's first call throws (Vite's mapping
// of stack traces fails on Playwright's).
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

export default async function setup() {
  const script = `import { warmUp } from ${JSON.stringify(new URL("./browser.js", import.meta.url).href)}; await warmUp();`;
  const node = spawn(process.execPath, ["--input-type=module", "--eval", script], {
    cwd: fileURLToPath(new URL("..", import.meta.url)),
    stdio: ["ignore", "inherit", "inherit"],
  });
  await new Promise((resolve) => node.on("exit", resolve).on("error", resolve));
}
