// Screenshots of the real window (owner's decision on the M4 plan): runs
// the built app with each scenario, waits until the page says it is on
// screen, and captures exactly the page's rectangle with the system's own
// tool. Nothing else on the screen is captured.
//
//   node scripts/ui-shots.mjs <app> <out-folder> <scenario>...
//   scenario: <view>-<lang>-<theme>, e.g. list-ar-light, or names
//
// macOS: screencapture (needs the Screen Recording permission); Linux:
// ImageMagick `import` on the X display (Xvfb in CI); Windows: .NET
// CopyFromScreen through PowerShell.
import { spawn, execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [app, out, ...scenarios] = process.argv.slice(2);
if (!app || !out || scenarios.length === 0) {
  console.error("usage: node scripts/ui-shots.mjs <app> <out-folder> <scenario>...");
  process.exit(2);
}
mkdirSync(out, { recursive: true });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function capture({ x, y, width, height, scale }, file) {
  if (process.platform === "darwin") {
    // screencapture takes points, not pixels.
    const p = (n) => Math.round(n / scale);
    execFileSync("screencapture", ["-x", "-R", `${p(x)},${p(y)},${p(width)},${p(height)}`, file]);
  } else if (process.platform === "linux") {
    execFileSync("import", ["-window", "root", "-crop", `${width}x${height}+${x}+${y}`, "+repage", file]);
  } else {
    const ps = [
      "Add-Type -AssemblyName System.Drawing",
      `$b = New-Object System.Drawing.Bitmap ${width}, ${height}`,
      "$g = [System.Drawing.Graphics]::FromImage($b)",
      `$g.CopyFromScreen(${x}, ${y}, 0, 0, $b.Size)`,
      `$b.Save('${file.replaceAll("'", "''")}', [System.Drawing.Imaging.ImageFormat]::Png)`,
    ].join("; ");
    execFileSync("powershell", ["-NoProfile", "-Command", ps]);
  }
}

let failed = false;
for (const scenario of scenarios) {
  const report = join(out, `${scenario}.json`);
  const data = mkdtempSync(join(tmpdir(), "zajil-ui-"));
  const child = spawn(app, ["--data-dir", data, "--ui-test", scenario, "--ui-test-out", out], {
    stdio: ["ignore", "inherit", "inherit"],
  });
  let exited = false;
  child.on("exit", () => (exited = true));
  const deadline = Date.now() + 90_000;
  while (!existsSync(report) && Date.now() < deadline && !exited) await sleep(200);
  if (!existsSync(report)) {
    console.error(`${scenario}: the page never reported (exited: ${exited})`);
    failed = true;
    child.kill();
    continue;
  }
  // Let the window manager finish drawing the frame.
  await sleep(800);
  const { window } = JSON.parse(readFileSync(report, "utf8"));
  const file = join(out, `${scenario}.png`);
  try {
    capture(window, file);
    console.log(`${scenario}: ${file} (${window.width}x${window.height} at ${window.x},${window.y}, scale ${window.scale})`);
  } catch (e) {
    console.error(`${scenario}: capture failed: ${e.message}`);
    failed = true;
  }
  child.kill();
  while (!exited) await sleep(100);
}
process.exit(failed ? 1 : 0);
