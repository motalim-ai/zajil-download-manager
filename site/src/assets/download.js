// The download page's buttons: each one starts as a link to the latest
// release's page on GitHub, which always works. This asks GitHub's API once
// for the latest release and, if it answers, points each button straight at
// its file there. The file names carry the version (Zajil_1.0.0_…), so
// `releases/latest/download/<name>` cannot be written in advance.
//
// Only a link under Zajil's own release downloads is ever used: never one
// taken as is from the answer. Nothing is stored, and no cookie or referrer
// is sent.

const REPO = "motalim-ai/zajil-download-manager";
const API = `https://api.github.com/repos/${REPO}/releases/latest`;
const DOWNLOADS = `https://github.com/${REPO}/releases/download/`;

/** Each button's file, by the names the release workflow gives them. */
const FILES = {
  windows: /^Zajil_\d+\.\d+\.\d+_x64-setup\.exe$/,
  mac: /^Zajil_\d+\.\d+\.\d+_universal\.dmg$/,
  deb: /^Zajil_\d+\.\d+\.\d+_amd64\.deb$/,
  appimage: /^Zajil_\d+\.\d+\.\d+_amd64\.AppImage$/,
  chrome: /^zajil-extension-chrome-edge-\d+\.\d+\.\d+\.zip$/,
  firefox: /^zajil-extension-firefox-\d+\.\d+\.\d+\.zip$/,
};

const TAG = /^v\d+\.\d+\.\d+$/;

const lang = document.documentElement.lang === "en" ? "en" : "ar";

const TEXT = {
  ar: {
    release: (v) => `آخر إصدار: زاجِل ${v}.`,
    notes: "ما الجديد فيه",
    mb: (n) => `${n.toLocaleString("ar")} ميجابايت`,
  },
  en: {
    release: (v) => `Latest release: Zajil ${v}.`,
    notes: "What's new",
    mb: (n) => `${n.toLocaleString("en")} MB`,
  },
}[lang];

async function latest() {
  const stop = new AbortController();
  const timer = setTimeout(() => stop.abort(), 8000);
  try {
    const answer = await fetch(API, {
      headers: { Accept: "application/vnd.github+json" },
      credentials: "omit",
      referrerPolicy: "no-referrer",
      cache: "no-cache",
      signal: stop.signal,
    });
    if (!answer.ok) return null;
    return await answer.json();
  } catch {
    return null;
  } finally {
    clearTimeout(timer);
  }
}

async function main() {
  const release = await latest();
  if (!release || typeof release.tag_name !== "string" || !TAG.test(release.tag_name)) return;
  if (release.draft || !Array.isArray(release.assets)) return;
  const tag = release.tag_name;
  const page = `https://github.com/${REPO}/releases/tag/${tag}`;

  for (const [key, pattern] of Object.entries(FILES)) {
    const asset = release.assets.find((a) => typeof a?.name === "string" && pattern.test(a.name));
    if (!asset) continue;
    const href = DOWNLOADS + tag + "/" + asset.name;
    if (asset.browser_download_url !== href) continue;
    for (const link of document.querySelectorAll(`[data-asset="${key}"]`)) {
      link.href = href;
      const name = link.querySelector("[data-name]");
      if (name) {
        const mb = Number.isSafeInteger(asset.size) && asset.size > 0 ? Math.max(1, Math.round(asset.size / 1e6)) : 0;
        name.textContent = mb ? `${asset.name} · ${TEXT.mb(mb)}` : asset.name;
      }
    }
  }
  for (const link of document.querySelectorAll('[data-asset="page"]')) link.href = page;

  const line = document.querySelector("[data-release]");
  if (line) {
    const notes = document.createElement("a");
    notes.href = page;
    notes.textContent = TEXT.notes;
    line.replaceChildren(TEXT.release(tag.slice(1)) + " ", notes);
  }
}

void main();
