// The built app opens its window and closes cleanly, on each system
// (the first part of the desktop app must stand on its own: owner's
// decision on #45). Nothing is captured from the screen.
//
//   node scripts/open-close.mjs <app> [--times <n>]
//
// Opened: the system lists a window of the process (Linux: xdotool on the
// X display, Xvfb in CI; macOS: CGWindowListCopyWindowInfo through JXA,
// which needs no permission for the owner of a window; Windows: the
// process's main window handle), and the page has loaded: the app says
// "the window is ready" when it first asks for the downloads.
// Closed: SIGTERM on Linux and macOS, and on macOS also the system's Quit
// event (what Cmd+Q, the Dock's Quit and a logout send); a window close
// (taskkill without /F) on Windows. The app must say that the downloads
// stopped, then exit by itself with code 0, within 4.5 s: sooner than its
// own 5 s limit, so a close that only waited it out fails (review of #45).
//
// A failure says at which stage it happened, how the app ended, and
// everything the app wrote to stderr (owner's request on #45); if the app
// still runs, also its state, its child processes (WebView2's) and every
// window it has, shown or not, before it is killed (#220: once on Windows
// no window came in 90 s and the app had said nothing).
import { spawn, execFileSync } from "node:child_process";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const args = process.argv.slice(2);
const app = args[0];
const timesAt = args.indexOf("--times");
const times = timesAt > 0 ? Number(args[timesAt + 1]) : 1;
if (!app || !Number.isInteger(times) || times < 1) {
  console.error("usage: node scripts/open-close.mjs <app> [--times <n>]");
  process.exit(2);
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// No system call may hang the job (review of #45).
const CALL = { encoding: "utf8", timeout: 10_000 };

function hasWindow(pid) {
  try {
    if (process.platform === "linux") {
      return execFileSync("xdotool", ["search", "--pid", String(pid)], CALL).trim() !== "";
    }
    if (process.platform === "darwin") {
      const script = `ObjC.import('CoreGraphics'); ObjC.deepUnwrap(ObjC.castRefToObject($.CGWindowListCopyWindowInfo($.kCGWindowListOptionAll, 0))).filter(w => w.kCGWindowOwnerPID === ${pid} && w.kCGWindowLayer === 0).length`;
      return Number(execFileSync("osascript", ["-l", "JavaScript", "-e", script], CALL).trim()) > 0;
    }
    const handle = execFileSync("powershell", ["-NoProfile", "-Command", `(Get-Process -Id ${pid}).MainWindowHandle`], CALL).trim();
    return handle !== "" && handle !== "0";
  } catch {
    return false;
  }
}

/// What the system says of a process still running: its state, its child
/// processes, and its windows. Lines to print; never throws.
function diagnose(pid) {
  const run = (file, argv) => {
    try {
      return execFileSync(file, argv, { ...CALL, timeout: 20_000 }).trimEnd().split("\n");
    } catch (e) {
      return [`(${file} failed: ${String(e.message ?? e).split("\n")[0]})`];
    }
  };
  if (process.platform === "win32") {
    const ps = [
      `$p = Get-Process -Id ${pid}`,
      `"process: responding=$($p.Responding) cpu=$($p.CPU) s threads=$($p.Threads.Count) main window=$($p.MainWindowHandle) '$($p.MainWindowTitle)'"`,
      `Get-CimInstance Win32_Process | Where-Object { $_.ParentProcessId -eq ${pid} } | ForEach-Object { "child: $($_.ProcessId) $($_.Name)" }`,
      "Add-Type -TypeDefinition 'using System; using System.Text; using System.Runtime.InteropServices; public static class ZajilWindows { public delegate bool EnumProc(IntPtr h, IntPtr l); [DllImport(\"user32.dll\")] public static extern bool EnumWindows(EnumProc f, IntPtr l); [DllImport(\"user32.dll\")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid); [DllImport(\"user32.dll\")] public static extern bool IsWindowVisible(IntPtr h); [DllImport(\"user32.dll\", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n); [DllImport(\"user32.dll\", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n); public static void List(uint pid) { EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p); if (p == pid) { var t = new StringBuilder(256); var c = new StringBuilder(256); GetWindowText(h, t, 256); GetClassName(h, c, 256); Console.WriteLine(\"window: \" + h + \" class=\" + c + \" visible=\" + IsWindowVisible(h) + \" title=\" + t); } return true; }, IntPtr.Zero); } }'",
      `[ZajilWindows]::List(${pid})`,
    ].join("; ");
    return run("powershell", ["-NoProfile", "-Command", ps]);
  }
  const lines = run("ps", ["-A", "-o", "pid=,ppid=,stat=,etime=,time=,comm="]).filter((l) => {
    const [p, pp] = l.trim().split(/\s+/);
    return p === String(pid) || pp === String(pid);
  });
  if (process.platform === "linux") lines.push(...run("xdotool", ["search", "--all", "--pid", String(pid)]).map((w) => `window: ${w}`));
  else {
    const script = `ObjC.import('CoreGraphics'); ObjC.deepUnwrap(ObjC.castRefToObject($.CGWindowListCopyWindowInfo($.kCGWindowListOptionAll, 0))).filter(w => w.kCGWindowOwnerPID === ${pid}).map(w => 'window: layer=' + w.kCGWindowLayer + ' onscreen=' + (w.kCGWindowIsOnscreen ? 1 : 0) + ' ' + JSON.stringify(w.kCGWindowBounds)).join('\\n')`;
    lines.push(...run("osascript", ["-l", "JavaScript", "-e", script]));
  }
  return lines;
}

/// The ways of closing the app on this system.
const WAYS = { linux: ["SIGTERM"], darwin: ["SIGTERM", "quit"], win32: ["window close"] }[process.platform] ?? ["SIGTERM"];

function close(pid, way) {
  if (way === "window close") execFileSync("taskkill", ["/PID", String(pid)], CALL);
  else if (way === "quit") {
    const script = `ObjC.import('AppKit'); $.NSRunningApplication.runningApplicationWithProcessIdentifier(${pid}).terminate`;
    execFileSync("osascript", ["-l", "JavaScript", "-e", script], CALL);
  } else process.kill(pid, "SIGTERM");
}

/// One run: opens the app, waits for its window and its page, closes it
/// (`way`), and checks that it stops its downloads and exits by itself with
/// code 0. Returns null, or why it failed.
async function once(run, way) {
  const started = Date.now();
  const at = () => `${((Date.now() - started) / 1000).toFixed(1)} s`;
  let stage = "starting the app";
  const log = (text) => console.log(`[run ${run} (${way}), ${at()}] ${text}`);
  const data = mkdtempSync(join(tmpdir(), "zajil-open-close-"));
  // `--close-quits`: on Windows the window's close is how it is asked to
  // quit (otherwise it hides into the tray, #109).
  const child = spawn(app, ["--data-dir", data, "--close-quits"], { stdio: ["ignore", "inherit", "pipe"] });
  let stderr = "";
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
    process.stderr.write(chunk);
  });
  let ended = null;
  child.on("exit", (code, signal) => (ended = { code, signal, at: at() }));
  child.on("error", (err) => (ended = { error: String(err), at: at() }));
  const fail = (why) => {
    const seen = ended ? [] : diagnose(child.pid);
    if (!ended) child.kill("SIGKILL");
    return [
      `run ${run} FAILED at the stage "${stage}": ${why}`,
      `  how the app ended: ${ended ? JSON.stringify(ended) : "still running (killed now)"}`,
      `  the app's stderr (${stderr.length} bytes; it writes no log file):`,
      ...(stderr.trim() ? stderr.trimEnd().split("\n").map((l) => `    | ${l}`) : ["    (nothing)"]),
      ...(seen.length ? ["  what the system said of it before it was killed:", ...seen.map((l) => `    | ${l}`)] : []),
    ].join("\n");
  };

  stage = "waiting for the window";
  while (!ended && !hasWindow(child.pid) && Date.now() - started < 90_000) await sleep(500);
  if (ended) return fail("the app exited before its window opened");
  if (!hasWindow(child.pid)) return fail("no window after 90 s");
  log("the window opened");

  stage = "waiting for the page to load";
  const loading = Date.now();
  while (!ended && !stderr.includes("zajil: the window is ready") && Date.now() - loading < 60_000) await sleep(200);
  if (ended) return fail("the app exited before its page loaded");
  if (!stderr.includes("zajil: the window is ready")) return fail("the page never asked for the downloads within 60 s");
  log("the page loaded");
  // Let the queues start before closing.
  await sleep(1000);
  if (ended) return fail("the app exited by itself while open");

  stage = `closing it (${way})`;
  close(child.pid, way);
  log(stage);
  stage = "waiting for the app to exit";
  const closing = Date.now();
  while (!ended && Date.now() - closing < 30_000) await sleep(100);
  const took = Date.now() - closing;
  if (!ended) return fail("no exit within 30 s of being asked to close");
  if (ended.code !== 0) return fail("it did not exit by itself with code 0");
  if (!stderr.includes("zajil: the downloads stopped; exiting")) return fail("it exited without saying that the downloads stopped");
  if (took > 4500) return fail(`it took ${took} ms: as long as its 5 s limit, so the downloads did not stop`);
  log(`the app stopped its downloads and closed cleanly in ${took} ms`);
  return null;
}

let failures = 0;
for (let run = 1; run <= times; run++) {
  for (const way of WAYS) {
    const why = await once(run, way);
    if (why) {
      failures += 1;
      console.error(why);
    }
  }
}
const total = times * WAYS.length;
console.log(`${total - failures} of ${total} runs opened and closed cleanly (${WAYS.join(", ")})`);
process.exit(failures === 0 ? 0 : 1);
