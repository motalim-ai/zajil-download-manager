// How many times Zajil was downloaded (owner's request, 2026-10-30): the
// installers of every release (`Zajil_<version>_…`), on every system, as
// GitHub counts them. Not the browser extensions, the checksums, nor the
// video tools' release (CI downloads those). Written as
// `<dist>/downloads.json`, read by the site's pages and, as a shields.io
// endpoint badge, by the README. Run by site.yml once a day and on each
// deploy; nothing is sent from Zajil itself (no tracking).
//
//   node site/downloads.mjs site/dist
import { writeFile } from "node:fs/promises";
import { join } from "node:path";

const REPO = "motalim-ai/zajil-download-manager";
const INSTALLER = /^Zajil_\d+\.\d+\.\d+_/;
const TAG = /^v\d+\.\d+\.\d+$/;

const dist = process.argv[2];
if (!dist) throw new Error("usage: node site/downloads.mjs <dist folder>");

const headers = { Accept: "application/vnd.github+json", "User-Agent": "zajil-site" };
if (process.env.GH_TOKEN) headers.Authorization = `Bearer ${process.env.GH_TOKEN}`;

let total = 0;
for (let page = 1; ; page++) {
  const answer = await fetch(`https://api.github.com/repos/${REPO}/releases?per_page=100&page=${page}`, { headers });
  if (!answer.ok) throw new Error(`GitHub answered ${answer.status}`);
  const releases = await answer.json();
  for (const release of releases) {
    if (release.draft || !TAG.test(release.tag_name)) continue;
    for (const asset of release.assets) {
      if (INSTALLER.test(asset.name)) total += asset.download_count;
    }
  }
  if (releases.length < 100) break;
}

const json = {
  // shields.io's endpoint badge.
  schemaVersion: 1,
  label: "downloads",
  message: total.toLocaleString("en"),
  color: "1747C9",
  // The site's own.
  total,
  updated: new Date().toISOString(),
};
await writeFile(join(dist, "downloads.json"), JSON.stringify(json) + "\n");
console.log(`downloads: ${total}`);
