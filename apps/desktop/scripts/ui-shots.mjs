// Screenshots of the real window (owner's decision on the M4 plan): runs
// the built app with each scenario, waits until the page says it is on
// screen, and captures exactly the page's rectangle with the system's own
// tool. Nothing else on the screen is captured: a rectangle not wholly
// inside the screen's work area (under the taskbar, or off the screen) is
// refused, not captured (review of #47).
//
//   node scripts/ui-shots.mjs <app> <out-folder> <scenario>...
//   scenario: <view>-<lang>-<theme>, e.g. list-ar-light, or names; view is
//   list, add, link, progress, complete, schedule, batch, grab, tools,
//   quality, details, refresh, settings, categories, or switch (the
//   language changes in the same session, checked here)
//
// Each scenario also carries an axe-core check of the page: a critical or
// serious problem fails the run. And every file name a scene shows must
// be drawn as the FileName component draws it (M5.5: mixed names right
// in every window); the views below must show at least one.
//
// macOS: screencapture (needs the Screen Recording permission); Linux:
// ImageMagick `import` on the X display (Xvfb in CI); Windows: .NET
// CopyFromScreen through PowerShell.
import { spawn, execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [app, out, ...scenarios] = process.argv.slice(2);
if (!app || !out || scenarios.length === 0) {
  console.error("usage: node scripts/ui-shots.mjs <app> <out-folder> <scenario>...");
  process.exit(2);
}
mkdirSync(out, { recursive: true });

// The dark «Download file info» shows a Google Drive link named «download»;
// the progress window shows its name only in its title bar, which the
// system draws.
const SHOW_NAMES = ["list", "add-light", "complete", "schedule", "quality", "details", "refresh"];

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
// No system call may hang the job.
const CALL = { timeout: 30_000 };

/// Why the rectangle cannot be captured whole, or null.
function outside(rect) {
  const s = rect.screen;
  if (!s) return "the page did not say where the screen is";
  if (rect.x < s.x || rect.y < s.y || rect.x + rect.width > s.x + s.width || rect.y + rect.height > s.y + s.height)
    return `the window (${rect.width}x${rect.height} at ${rect.x},${rect.y}) is not inside the screen's work area (${s.width}x${s.height} at ${s.x},${s.y})`;
  return null;
}

function capture({ x, y, width, height, scale }, file) {
  if (process.platform === "darwin") {
    // screencapture takes points, not pixels.
    const p = (n) => Math.round(n / scale);
    execFileSync("screencapture", ["-x", "-R", `${p(x)},${p(y)},${p(width)},${p(height)}`, file], CALL);
  } else if (process.platform === "linux") {
    execFileSync("import", ["-window", "root", "-crop", `${width}x${height}+${x}+${y}`, "+repage", file], CALL);
  } else {
    const ps = [
      "Add-Type -AssemblyName System.Drawing",
      `$b = New-Object System.Drawing.Bitmap ${width}, ${height}`,
      "$g = [System.Drawing.Graphics]::FromImage($b)",
      `$g.CopyFromScreen(${x}, ${y}, 0, 0, $b.Size)`,
      `$b.Save('${file.replaceAll("'", "''")}', [System.Drawing.Imaging.ImageFormat]::Png)`,
    ].join("; ");
    execFileSync("powershell", ["-NoProfile", "-Command", ps], CALL);
  }
}

/// Stops the app: asks first, then kills it after 10 s, and waits for it.
/// Returns why it would not stop, or null.
async function stop(child, ended) {
  if (ended()) return null;
  child.kill();
  for (let i = 0; i < 100 && !ended(); i++) await sleep(100);
  if (ended()) return null;
  child.kill("SIGKILL");
  for (let i = 0; i < 100 && !ended(); i++) await sleep(100);
  return ended() ? "it ignored the request to close and was killed" : "it would not stop, even killed";
}

let failed = false;
for (const scenario of scenarios) {
  const report = join(out, `${scenario}.json`);
  const file = join(out, `${scenario}.png`);
  // A report left from an earlier run would be trusted (review of #47).
  rmSync(report, { force: true });
  rmSync(file, { force: true });
  const data = mkdtempSync(join(tmpdir(), "zajil-ui-"));
  const child = spawn(app, ["--data-dir", data, "--ui-test", scenario, "--ui-test-out", out], {
    stdio: ["ignore", "inherit", "inherit"],
  });
  let exited = false;
  let startError = null;
  child.on("exit", () => (exited = true));
  child.on("error", (e) => {
    startError = e.message;
    exited = true;
  });
  const fail = (why) => {
    console.error(`${scenario}: ${why}`);
    failed = true;
  };
  const deadline = Date.now() + 90_000;
  while (!existsSync(report) && Date.now() < deadline && !exited) await sleep(200);
  if (startError) fail(`the app could not start: ${startError}`);
  else if (!existsSync(report)) fail(`the page never reported (exited: ${exited})`);
  else {
    // Let the window manager finish drawing the frame.
    await sleep(800);
    const { window, accessibility = [], languageSwitch, names } = JSON.parse(readFileSync(report, "utf8"));
    // Any critical or serious accessibility problem fails (M4 criterion);
    // the others are listed.
    for (const v of accessibility) {
      const blocking = v.impact === "critical" || v.impact === "serious";
      (blocking ? console.error : console.log)(`${scenario}: accessibility ${v.impact}: ${v.id} (${v.help}) at ${v.nodes.join(", ")}`);
      if (blocking) failed = true;
    }
    if (scenario !== "names") {
      const [view, , theme] = scenario.split("-");
      for (const why of names?.raw ?? []) fail(`a file name not drawn as FileName draws it: ${why}`);
      if (!names) fail("the page did not say which file names it drew");
      else if ((SHOW_NAMES.includes(view) || SHOW_NAMES.includes(`${view}-${theme}`)) && names.drawn === 0) fail("no mixed file name shown, so none was checked");
      else console.log(`${scenario}: ${names.drawn} mixed file names drawn as FileName draws them`);
    }
    if (scenario.startsWith("switch-")) {
      const { before, after } = languageSwitch ?? {};
      const ok = before && after && before.dir !== after.dir && before.lang !== after.lang && before.text !== after.text;
      console.log(`${scenario}: language switch ${ok ? "ok" : "FAILED"}: ${JSON.stringify(languageSwitch)}`);
      if (!ok) failed = true;
    }
    const why = outside(window);
    if (why) fail(`not captured: ${why}`);
    else {
      try {
        capture(window, file);
        console.log(`${scenario}: ${file} (${window.width}x${window.height} at ${window.x},${window.y}, scale ${window.scale})`);
      } catch (e) {
        fail(`capture failed: ${e.message}`);
      }
    }
  }
  const stopped = await stop(child, () => exited);
  if (stopped) fail(stopped);
  rmSync(data, { recursive: true, force: true });
}
process.exit(failed ? 1 : 0);
