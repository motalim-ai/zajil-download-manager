// The Android page's buttons: each one starts as a link to the Android
// releases on GitHub, which always works. This asks GitHub's API once for
// the newest releases and, if it answers, points the button straight at the
// newest Android APK (tag android-vX.Y.Z, published, not a draft). Android
// releases are pre-releases, so `releases/latest` never names one.
//
// Only a link under Zajil's own release downloads is ever used: never one
// taken as is from the answer. Nothing is stored, and no cookie or referrer
// is sent.

const REPO = "motalim-ai/zajil-download-manager";
const API = `https://api.github.com/repos/${REPO}/releases?per_page=30`;
const DOWNLOADS = `https://github.com/${REPO}/releases/download/`;

const TAG = /^android-v(\d+\.\d+\.\d+)$/;

const lang = document.documentElement.lang === "en" ? "en" : "ar";

const TEXT = {
  ar: {
    release: (v) => `آخر إصدار: زاجِل لأندرويد ${v}.`,
    notes: "ما الجديد فيه",
    mb: (n) => `${n.toLocaleString("ar")} ميجابايت`,
  },
  en: {
    release: (v) => `Latest release: Zajil for Android ${v}.`,
    notes: "What's new",
    mb: (n) => `${n.toLocaleString("en")} MB`,
  },
}[lang];

async function releases() {
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
    const list = await answer.json();
    return Array.isArray(list) ? list : null;
  } catch {
    return null;
  } finally {
    clearTimeout(timer);
  }
}

async function main() {
  const list = await releases();
  if (!list) return;
  // The newest published Android release that has its APK.
  for (const release of list) {
    if (!release || release.draft || typeof release.tag_name !== "string") continue;
    const m = TAG.exec(release.tag_name);
    if (!m || !Array.isArray(release.assets)) continue;
    const version = m[1];
    const tag = release.tag_name;
    const name = `zajil-android-${version}-arm64-v8a.apk`;
    const asset = release.assets.find((a) => a?.name === name);
    const href = DOWNLOADS + tag + "/" + name;
    if (!asset || asset.browser_download_url !== href) continue;

    const page = `https://github.com/${REPO}/releases/tag/${tag}`;
    for (const link of document.querySelectorAll('[data-android="apk"]')) {
      link.href = href;
      const label = link.querySelector("[data-name]");
      if (label) {
        const mb = Number.isSafeInteger(asset.size) && asset.size > 0 ? Math.max(1, Math.round(asset.size / 1e6)) : 0;
        label.textContent = mb ? `${name} · ${TEXT.mb(mb)}` : name;
      }
    }
    for (const link of document.querySelectorAll('[data-android="page"]')) link.href = page;
    const line = document.querySelector("[data-release]");
    if (line) {
      const notes = document.createElement("a");
      notes.href = page;
      notes.textContent = TEXT.notes;
      line.replaceChildren(TEXT.release(version) + " ", notes);
    }
    return;
  }
}

void main();
