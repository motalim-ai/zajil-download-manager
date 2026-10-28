// Builds Zajil's site into site/dist: plain static files, no packages.
//
// Each page in site/src/pages is its <main> only, after a short head of
// `key: value` lines in an HTML comment (title, description, and an
// optional script). This file wraps it in the one layout (header, menu,
// footer) for its language: pages under `en/` are English, the rest Arabic.
// The screenshots and the icon are copied from where the repository keeps
// them (docs/screenshots, assets/icon), never kept twice.
//
// Run from anywhere: node site/build.mjs

import { cpSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

const SITE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(SITE, "..");
const SRC = join(SITE, "src");
const PAGES = join(SRC, "pages");
const OUT = join(SITE, "dist");

export const ORIGIN = "https://zajil.by-wow.com";
export const REPO = "https://github.com/motalim-ai/zajil-download-manager";

const TEXT = {
  ar: {
    dir: "rtl",
    locale: "ar_AR",
    name: "زاجِل",
    skip: "تخطَّ إلى المحتوى",
    menu: "القائمة الرئيسية",
    nav: [
      ["home", "/", "الرئيسية"],
      ["shots", "/#screenshots", "الصور"],
      ["download", "/download/", "التحميل"],
      ["install", "/download/#install", "التثبيت"],
      ["faq", "/faq/", "الأسئلة الشائعة"],
      ["privacy", "/privacy/", "الخصوصية"],
    ],
    other: "English",
    footerLinks: [
      [REPO, "الكود على GitHub"],
      [`${REPO}/releases`, "كل الإصدارات"],
      [`${REPO}/issues`, "أبلغ عن مشكلة"],
      [`${REPO}/security/advisories/new`, "أبلغ عن ثغرة أمنية"],
      ["/privacy/", "سياسة الخصوصية"],
    ],
    footer:
      "زاجِل برنامج مجاني ومفتوح المصدر بترخيص GPL-3.0-or-later. مصادره الرسمية الوحيدة هذا الموقع ومستودعه على GitHub. وهذا الموقع بلا كوكيز ولا تتبع ولا إحصاءات.",
  },
  en: {
    dir: "ltr",
    locale: "en_US",
    name: "Zajil",
    skip: "Skip to content",
    menu: "Main menu",
    nav: [
      ["home", "/en/", "Home"],
      ["shots", "/en/#screenshots", "Screenshots"],
      ["download", "/en/download/", "Download"],
      ["install", "/en/download/#install", "Install"],
      ["faq", "/en/faq/", "FAQ"],
      ["privacy", "/en/privacy/", "Privacy"],
    ],
    other: "العربية",
    footerLinks: [
      [REPO, "Source code on GitHub"],
      [`${REPO}/releases`, "All releases"],
      [`${REPO}/issues`, "Report a problem"],
      [`${REPO}/security/advisories/new`, "Report a security issue"],
      ["/en/privacy/", "Privacy policy"],
    ],
    footer:
      "Zajil is free and open-source software under GPL-3.0-or-later. Its only official sources are this site and its GitHub repository. This site has no cookies, no tracking and no analytics.",
  },
};

/** Every file under `dir`, as paths relative to it. */
function walk(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? walk(path).map((p) => join(name, p)) : [name];
  });
}

const escape = (s) => s.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");

/** The page's head lines and its body. */
function parse(file, source) {
  const m = /^<!--\n([\s\S]*?)\n-->\n/.exec(source);
  if (!m) throw new Error(`${file}: no head comment`);
  const meta = {};
  for (const line of m[1].split("\n")) {
    const at = line.indexOf(":");
    if (at < 1) throw new Error(`${file}: bad head line: ${line}`);
    meta[line.slice(0, at).trim()] = line.slice(at + 1).trim();
  }
  for (const key of ["title", "description"]) if (!meta[key]) throw new Error(`${file}: no ${key}`);
  return { meta, body: source.slice(m[0].length) };
}

/** The address of a page file: `en/faq/index.html` is `/en/faq/`. */
const urlOf = (file) => "/" + file.split(sep).join("/").replace(/index\.html$/, "");

/** The same page in the other language. */
const twinOf = (url) => (url.startsWith("/en/") ? url.slice(3) : "/en" + url);

function layout({ lang, url, meta, body }) {
  const t = TEXT[lang];
  const twin = twinOf(url);
  const arUrl = lang === "ar" ? url : twin;
  const enUrl = lang === "en" ? url : twin;
  const nav = t.nav
    .map(([key, href, label]) => {
      const current = key === meta.nav ? ' aria-current="page"' : "";
      return `<li><a href="${href}"${current}>${label}</a></li>`;
    })
    .join("\n          ");
  const otherLang = lang === "ar" ? "en" : "ar";
  const footerLinks = t.footerLinks.map(([href, label]) => `<li><a href="${href}">${label}</a></li>`).join("\n        ");
  const script = meta.script ? `\n  <script src="/assets/${meta.script}.js" defer></script>` : "";
  const notFound = url === "/404.html" || url === "/en/404.html";
  const robots = notFound ? '\n  <meta name="robots" content="noindex">' : "";
  const alternates = notFound
    ? ""
    : `
  <link rel="canonical" href="${ORIGIN}${url}">
  <link rel="alternate" hreflang="ar" href="${ORIGIN}${arUrl}">
  <link rel="alternate" hreflang="en" href="${ORIGIN}${enUrl}">
  <link rel="alternate" hreflang="x-default" href="${ORIGIN}${arUrl}">`;
  return `<!doctype html>
<html lang="${lang}" dir="${t.dir}">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>${escape(meta.title)}</title>
  <meta name="description" content="${escape(meta.description)}">${robots}
  <meta name="color-scheme" content="light dark">
  <meta name="theme-color" content="#ffffff" media="(prefers-color-scheme: light)">
  <meta name="theme-color" content="#141d31" media="(prefers-color-scheme: dark)">${alternates}
  <meta property="og:type" content="website">
  <meta property="og:site_name" content="${t.name}">
  <meta property="og:title" content="${escape(meta.title)}">
  <meta property="og:description" content="${escape(meta.description)}">
  <meta property="og:url" content="${ORIGIN}${url}">
  <meta property="og:image" content="${ORIGIN}/screenshots/main-ar-light.png">
  <meta property="og:locale" content="${t.locale}">
  <link rel="icon" href="/assets/zajil-icon.svg" type="image/svg+xml">
  <link rel="icon" href="/assets/icon-32.png" sizes="32x32" type="image/png">
  <link rel="apple-touch-icon" href="/assets/icon-256.png">
  <link rel="stylesheet" href="/assets/site.css">${script}
</head>
<body>
  <a class="skip" href="#main">${t.skip}</a>
  <header class="site-header">
    <div class="wrap">
      <a class="brand" href="${t.nav[0][1]}"><img src="/assets/zajil-icon.svg" alt="" width="40" height="40"><span>${t.name}</span></a>
      <nav class="site-nav" aria-label="${t.menu}">
        <ul>
          ${nav}
          <li class="lang"><a href="${twin}" hreflang="${otherLang}" lang="${otherLang}">${t.other}</a></li>
        </ul>
      </nav>
    </div>
  </header>
${body.trimEnd()}
  <footer class="site-footer">
    <div class="wrap">
      <ul>
        ${footerLinks}
      </ul>
      <p>${t.footer}</p>
    </div>
  </footer>
</body>
</html>
`;
}

export function build() {
  rmSync(OUT, { recursive: true, force: true });
  mkdirSync(OUT, { recursive: true });

  for (const file of walk(PAGES)) {
    const source = readFileSync(join(PAGES, file), "utf8");
    const { meta, body } = parse(file, source);
    const url = urlOf(file);
    const lang = url.startsWith("/en/") ? "en" : "ar";
    const out = join(OUT, file);
    mkdirSync(dirname(out), { recursive: true });
    writeFileSync(out, layout({ lang, url, meta, body }));
  }

  cpSync(join(SRC, "assets"), join(OUT, "assets"), { recursive: true });
  cpSync(join(SRC, "_headers"), join(OUT, "_headers"));
  cpSync(join(ROOT, "docs", "screenshots"), join(OUT, "screenshots"), { recursive: true });
  cpSync(join(ROOT, "assets", "icon", "zajil-icon.svg"), join(OUT, "assets", "zajil-icon.svg"));
  const icons = join(ROOT, "apps", "desktop", "src-tauri", "icons");
  cpSync(join(icons, "32x32.png"), join(OUT, "assets", "icon-32.png"));
  cpSync(join(icons, "128x128@2x.png"), join(OUT, "assets", "icon-256.png"));

  return walk(OUT).map((p) => relative(OUT, join(OUT, p)));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const files = build();
  console.log(`site/dist: ${files.length} files`);
}
