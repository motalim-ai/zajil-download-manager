// Checks the built site (site/dist) before it is deployed: every link and
// image inside the site leads to a file there (and to an id on that page,
// for a link with #), every Arabic page has its English twin, every page
// says its language and direction, and nothing loads from outside the site
// or inline (the Content-Security-Policy in each page's head would block it).
//
// Run after the build: node site/build.mjs && node site/check.mjs

import { existsSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { CSP, build } from "./build.mjs";

const OUT = join(import.meta.dirname, "dist");
const files = build();
const pages = files.filter((f) => f.endsWith(".html")).map((f) => f.split("\\").join("/"));
const problems = [];
const fail = (page, what) => problems.push(`${page}: ${what}`);

/** The file a site address is served from, or null. */
function fileOf(path) {
  const plain = decodeURI(path);
  const candidates = plain.endsWith("/") ? [plain + "index.html"] : [plain, plain + "/index.html"];
  for (const c of candidates) {
    const full = join(OUT, c);
    if (existsSync(full) && statSync(full).isFile()) return c.replace(/^\//, "");
  }
  return null;
}

const ids = new Map();
const idsOf = (page) => {
  if (!ids.has(page)) {
    const html = readFileSync(join(OUT, page), "utf8");
    ids.set(page, new Set([...html.matchAll(/\sid="([^"]+)"/g)].map((m) => m[1])));
  }
  return ids.get(page);
};

for (const page of pages) {
  const html = readFileSync(join(OUT, page), "utf8");
  const english = page.startsWith("en/");

  const root = /<html lang="(ar|en)" dir="(rtl|ltr)">/.exec(html);
  if (!root) fail(page, "no <html lang dir>");
  else if (root[1] !== (english ? "en" : "ar") || root[2] !== (english ? "ltr" : "rtl"))
    fail(page, `wrong language or direction: ${root[0]}`);

  const twin = english ? page.slice(3) : "en/" + page;
  if (!pages.includes(twin)) fail(page, `no twin page ${twin}`);

  if (!html.includes('<meta http-equiv="Content-Security-Policy" content="' + CSP + '">')) fail(page, "no Content-Security-Policy meta");
  if (!/<meta name="referrer" content="[^"]+">/.test(html)) fail(page, "no referrer meta");
  if (/<script(?![^>]*\ssrc=)[^>]*>/.test(html)) fail(page, "an inline <script>");
  if (/\sstyle="/.test(html) || /<style[\s>]/.test(html)) fail(page, "an inline style");
  if (/\son[a-z]+="/.test(html)) fail(page, "an inline event handler");
  if (/http:\/\//.test(html)) fail(page, "an http:// address");

  for (const m of html.matchAll(/<(script|link|img|source)\b[^>]*>/g)) {
    const tag = m[0];
    if (/^<link/.test(tag) && !/rel="(stylesheet|icon|apple-touch-icon)"/.test(tag)) continue;
    const at = /\s(?:src|href|srcset)="([^"]+)"/.exec(tag);
    if (!at) continue;
    if (!at[1].startsWith("/")) fail(page, `loads from outside the site: ${at[1]}`);
  }

  for (const m of html.matchAll(/\s(href|src|srcset)="(\/[^"]*)"/g)) {
    const [path, hash] = m[2].split("#");
    const target = path === "" ? page : fileOf(path);
    if (!target) {
      fail(page, `broken link: ${m[2]}`);
      continue;
    }
    if (hash && target.endsWith(".html") && !idsOf(target).has(hash)) fail(page, `no #${hash} in ${target}`);
  }

  for (const m of html.matchAll(/\shref="#([^"]+)"/g)) {
    if (!idsOf(page).has(m[1])) fail(page, `no #${m[1]} on the page`);
  }

  for (const m of html.matchAll(/\shref="(https:\/\/[^"]+)"/g)) {
    const url = new URL(m[1]);
    if (url.hostname === "github.com" && !url.pathname.startsWith("/motalim-ai/zajil-download-manager"))
      fail(page, `a GitHub link outside Zajil's repository: ${m[1]}`);
  }
}

if (problems.length) {
  console.error(problems.join("\n"));
  console.error(`\n${problems.length} problem(s)`);
  process.exit(1);
}
console.log(`site/dist: ${pages.length} pages checked, ${files.length} files`);
