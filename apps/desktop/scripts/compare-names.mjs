// The image test of mixed names (M4 criterion, owner's decision): in the
// screenshot of the real window (scenario `names`), each name drawn by the
// real component must look the same as its reference (pieces of one
// direction each, in an explicit visual order), and different from the
// wrong renderings marked as such.
//
//   node scripts/compare-names.mjs <out-folder>   (reads names.png, names.json)
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { PNG } from "pngjs";

const out = process.argv[2];
const report = JSON.parse(readFileSync(join(out, "names.json"), "utf8"));
const png = PNG.sync.read(readFileSync(join(out, "names.png")));

const lum = (x, y) => {
  const i = (png.width * y + x) << 2;
  return 0.299 * png.data[i] + 0.587 * png.data[i + 1] + 0.114 * png.data[i + 2];
};

// Where the page is in the screenshot: found from the two magenta markers
// the page draws (the captured rectangle may include a title bar, as on
// macOS), never assumed.
function findMarkers() {
  const hits = [];
  for (let y = 0; y < png.height; y++)
    for (let x = 0; x < png.width; x++) {
      const i = (png.width * y + x) << 2;
      if (png.data[i] > 200 && png.data[i + 1] < 80 && png.data[i + 2] > 200) hits.push([x, y]);
    }
  if (hits.length === 0) return null;
  const midX = (Math.min(...hits.map((h) => h[0])) + Math.max(...hits.map((h) => h[0]))) / 2;
  const corner = (side) => {
    const pts = hits.filter((h) => (side < 0 ? h[0] < midX : h[0] >= midX));
    return [Math.min(...pts.map((p) => p[0])), Math.min(...pts.map((p) => p[1]))];
  };
  return { a: corner(-1), b: corner(1) };
}
const found = findMarkers();
if (!found) {
  console.error("no magenta marker in the screenshot: it does not show the test page");
  process.exit(1);
}
const scale = (found.b[0] - found.a[0]) / (report.markers.b[0] - report.markers.a[0]);
const offset = [found.a[0] - report.markers.a[0] * scale, found.a[1] - report.markers.a[1] * scale];
if (Math.abs(scale - report.window.scale) > 0.05) {
  console.error(`the markers give a scale of ${scale}, the window says ${report.window.scale}`);
  process.exit(1);
}

/// The dark pixels ("ink") inside a page rectangle, cut to their bounds.
/// A rectangle not wholly in the screenshot is a failure of its own, so a
/// window too small for the page is never read as a wrong rendering.
function ink(rect, what) {
  const [rx, ry, rw, rh] = [rect[0] * scale + offset[0], rect[1] * scale + offset[1], rect[2] * scale, rect[3] * scale].map(Math.round);
  if (rx < 0 || ry < 0 || rx + rw > png.width || ry + rh > png.height) {
    console.error(`${what} is outside the captured window (${rx},${ry} ${rw}x${rh} in ${png.width}x${png.height}): the window is smaller than the test page`);
    process.exit(1);
  }
  let x0 = Infinity, y0 = Infinity, x1 = -1, y1 = -1;
  for (let y = ry; y < ry + rh; y++)
    for (let x = rx; x < rx + rw; x++)
      if (lum(x, y) < 160) {
        x0 = Math.min(x0, x); y0 = Math.min(y0, y); x1 = Math.max(x1, x); y1 = Math.max(y1, y);
      }
  if (x1 < 0) return null;
  const w = x1 - x0 + 1, h = y1 - y0 + 1;
  const px = new Float32Array(w * h);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) px[y * w + x] = lum(x0 + x, y0 + y);
  return { w, h, px };
}

/// `img` averaged over 3x3 pixels: the edges of a glyph placed half a
/// pixel apart become alike, while a glyph in another place stays apart.
function smooth(img) {
  const px = new Float32Array(img.w * img.h);
  for (let y = 0; y < img.h; y++)
    for (let x = 0; x < img.w; x++) {
      let sum = 0, n = 0;
      for (let dy = -1; dy <= 1; dy++)
        for (let dx = -1; dx <= 1; dx++) {
          const xx = x + dx, yy = y + dy;
          if (xx >= 0 && yy >= 0 && xx < img.w && yy < img.h) {
            sum += img.px[yy * img.w + xx];
            n++;
          }
        }
      px[y * img.w + x] = sum / n;
    }
  return { w: img.w, h: img.h, px };
}

/// The share of pixels that differ clearly, once both are aligned on
/// their ink and smoothed; 1 when their sizes differ by more than a pixel.
/// Smoothing first: a reference made of separate pieces may place them
/// half a pixel off on Windows, which changed the edges' brightness enough
/// to count 7% of the pixels of an identical-looking name as different.
function difference(a, b) {
  if (!a || !b) return 1;
  if (Math.abs(a.w - b.w) > 1 || Math.abs(a.h - b.h) > 1) return 1;
  const [sa, sb] = [smooth(a), smooth(b)];
  const w = Math.min(a.w, b.w), h = Math.min(a.h, b.h);
  let bad = 0;
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) if (Math.abs(sa.px[y * sa.w + x] - sb.px[y * sb.w + x]) > 64) bad++;
  return bad / (w * h);
}

// The real component and its reference are drawn by the same engine with
// the same font, so they match (0.000 on the three systems, smoothed). A
// wrong rendering differs at least by its dot and extension: 0.032 for the
// rejected `.pdf` rendering on Linux, the smallest seen.
const SAME = 0.01;
const DIFFERENT = 0.02;
let failed = false;
const results = [];
for (const row of report.rows) {
  const real = ink(row.real, `${row.name}: the real rendering`);
  const vsReference = difference(real, ink(row.reference, `${row.name}: the reference`));
  const vsNaive = difference(real, ink(row.naive, `${row.name}: the plain rendering`));
  const vsExtensionOnly = difference(real, ink(row.extensionOnly, `${row.name}: the rejected rendering`));
  const problems = [];
  if (vsReference > SAME) problems.push(`differs from its reference (${vsReference.toFixed(3)})`);
  if (row.naiveDiffers && vsNaive < DIFFERENT) problems.push(`looks like the plain rendering (${vsNaive.toFixed(3)})`);
  if (row.extensionOnlyDiffers && vsExtensionOnly < DIFFERENT)
    problems.push(`looks like the rejected rendering (${vsExtensionOnly.toFixed(3)})`);
  results.push({ name: row.name, vsReference, vsNaive, vsExtensionOnly, problems });
  console.log(`${problems.length ? "FAIL" : "ok  "} ${row.name}: reference ${vsReference.toFixed(3)}, plain ${vsNaive.toFixed(3)}, extension only ${vsExtensionOnly.toFixed(3)}${problems.length ? " — " + problems.join("; ") : ""}`);
  if (problems.length) failed = true;
}
writeFileSync(join(out, "names-result.json"), JSON.stringify(results, null, 2));
process.exit(failed ? 1 : 0);
