// Builds Zajil's site into site/dist: plain static files, no packages.
//
// Each page in site/src/pages is its <main> only, after a short head of
// `key: value` lines in an HTML comment (title, description, nav, and an
// optional script). This file wraps it in the one layout (header, menu,
// icons, footer) for its language: pages under `en/` are English, the rest
// Arabic. The screenshots and the icon are copied from where the
// repository keeps them (docs/screenshots, assets/icon), never kept twice.
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

// GitHub Pages sets no headers of ours, so the policy goes in each page's
// head. Scripts, styles, images and fonts come from the site only; the one
// address beyond it is GitHub's API, for the download buttons.
export const CSP = [
  "default-src 'none'",
  "script-src 'self'",
  "style-src 'self'",
  "img-src 'self'",
  "font-src 'self'",
  "connect-src https://api.github.com",
  "base-uri 'none'",
  "form-action 'none'",
].join("; ");
export const REPO = "https://github.com/motalim-ai/zajil-download-manager";

const TEXT = {
  ar: {
    dir: "rtl",
    locale: "ar_AR",
    name: "زاجِل",
    fullName: "زاجِل لإدارة التحميل",
    home: "زاجِل، الصفحة الرئيسية",
    skip: "انتقل إلى المحتوى",
    menu: "أقسام الموقع",
    themeLabel: "الوضع الداكن",
    menuLabel: "فتح القائمة",
    get: "احصل على زاجِل",
    getFree: "احصل على زاجِل مجاناً",
    group: "زاجِل",
    sections: [
      ["/#journey", "كيف يعمل"],
      ["/#features", "المزايا"],
      ["/#screens", "صور البرنامج"],
      ["/#extension", "إضافة المتصفح"],
    ],
    nav: [
      ["download", "/download/", "التحميل"],
      ["faq", "/faq/", "أسئلة شائعة"],
      ["privacy", "/privacy/", "الخصوصية"],
    ],
    support: ["support", "/support/", "ادعم زاجِل"],
    otherLabel: "الموقع بالإنجليزية",
    footerNav: "روابط أسفل الصفحة",
    footerAbout: "برنامج مجاني ومفتوح المصدر بترخيص GPL-3.0، يوصل ملفك كاملاً مهما بعدت المسافة.",
    footerSite: [
      ["/download/", "التحميل"],
      ["/download/#install", "خطوات التثبيت"],
      ["/faq/", "أسئلة شائعة"],
      ["/privacy/", "سياسة الخصوصية"],
      ["/support/", "ادعم زاجِل"],
    ],
    footerRepo: [
      [REPO, "GitHub"],
      [`${REPO}/releases`, "الإصدارات"],
      [`${REPO}/blob/main/CONTRIBUTING.md`, "المساهمة"],
      [`${REPO}/issues`, "أبلغ عن مشكلة"],
      [`${REPO}/security/advisories/new`, "أبلغ عن ثغرة أمنية"],
    ],
    base: "زاجِل مشروع مستقل ومفتوح المصدر. مصادره الرسمية الوحيدة هذا الموقع ومستودعه على GitHub.",
    toTop: "إلى أعلى الصفحة",
  },
  en: {
    dir: "ltr",
    locale: "en_US",
    name: "Zajil",
    fullName: "Zajil Download Manager",
    home: "Zajil, home page",
    skip: "Skip to content",
    menu: "Site sections",
    themeLabel: "Dark mode",
    menuLabel: "Open the menu",
    get: "Get Zajil",
    getFree: "Get Zajil for free",
    group: "Zajil",
    sections: [
      ["/en/#journey", "How it works"],
      ["/en/#features", "Features"],
      ["/en/#screens", "Screenshots"],
      ["/en/#extension", "Extension"],
    ],
    nav: [
      ["download", "/en/download/", "Download"],
      ["faq", "/en/faq/", "FAQ"],
      ["privacy", "/en/privacy/", "Privacy"],
    ],
    support: ["support", "/en/support/", "Support Zajil"],
    otherLabel: "الموقع بالعربية",
    footerNav: "Footer links",
    footerAbout: "Free and open-source software under GPL-3.0, that gets your file to you whole, however far it comes from.",
    footerSite: [
      ["/en/download/", "Download"],
      ["/en/download/#install", "Install steps"],
      ["/en/faq/", "FAQ"],
      ["/en/privacy/", "Privacy policy"],
      ["/en/support/", "Support Zajil"],
    ],
    footerRepo: [
      [REPO, "GitHub"],
      [`${REPO}/releases`, "Releases"],
      [`${REPO}/blob/main/CONTRIBUTING.md`, "Contributing"],
      [`${REPO}/issues`, "Report a problem"],
      [`${REPO}/security/advisories/new`, "Report a security issue"],
    ],
    base: "Zajil is an independent, open-source project. Its only official sources are this site and its GitHub repository.",
    toTop: "Back to top",
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

const links = (list) => list.map(([href, label]) => `<li><a href="${href}">${label}</a></li>`).join("\n            ");

function layout({ lang, url, meta, body, icons }) {
  const t = TEXT[lang];
  const twin = twinOf(url);
  const arUrl = lang === "ar" ? url : twin;
  const enUrl = lang === "en" ? url : twin;
  const homeUrl = lang === "ar" ? "/" : "/en/";
  const downloadUrl = t.nav.find(([key]) => key === "download")[1];
  const current = (key) => (key === meta.nav ? ' aria-current="page"' : "");
  const sections = t.sections.map(([href, label]) => `<li><a href="${href}">${label}</a></li>`).join("\n                ");
  const nav = [
    `<li class="nav__group">
              <button class="nav__group-btn" type="button" aria-expanded="false" aria-controls="nav-sections" data-subnav>${t.group}<svg class="icon" aria-hidden="true"><use href="#i-chev"/></svg></button>
              <ul class="nav__sub" id="nav-sections">
                ${sections}
              </ul>
            </li>`,
    ...t.nav.map(([key, href, label]) => `<li><a href="${href}"${current(key)}>${label}</a></li>`),
    `<li class="nav__support"><a href="${t.support[1]}"${current(t.support[0])}><svg class="icon" aria-hidden="true"><use href="#i-heart"/></svg>${t.support[2]}</a></li>`,
  ].join("\n            ");
  const otherLang = lang === "ar" ? "en" : "ar";
  const scripts = (meta.script ? meta.script.split(/\s+/) : [])
    .map((s) => `\n  <script src="/assets/${s}.js" defer></script>`)
    .join("");
  const notFound = url.endsWith("/404.html");
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
  <meta http-equiv="Content-Security-Policy" content="${CSP}">
  <meta name="referrer" content="strict-origin-when-cross-origin">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>${escape(meta.title)}</title>
  <meta name="description" content="${escape(meta.description)}">${robots}
  <meta name="color-scheme" content="light dark">
  <meta name="theme-color" content="#F4F8FF" media="(prefers-color-scheme: light)">
  <meta name="theme-color" content="#060E2B" media="(prefers-color-scheme: dark)">${alternates}
  <meta property="og:type" content="website">
  <meta property="og:site_name" content="${t.fullName}">
  <meta property="og:title" content="${escape(meta.title)}">
  <meta property="og:description" content="${escape(meta.description)}">
  <meta property="og:url" content="${ORIGIN}${url}">
  <meta property="og:image" content="${ORIGIN}/screenshots/main-ar-light.png">
  <meta property="og:locale" content="${t.locale}">
  <link rel="icon" href="/assets/zajil-icon.svg" type="image/svg+xml">
  <link rel="icon" href="/assets/icon-32.png" sizes="32x32" type="image/png">
  <link rel="apple-touch-icon" href="/assets/icon-256.png">
  <link rel="preload" href="/assets/fonts/alexandria-500-900-arabic.woff2" as="font" type="font/woff2" crossorigin>
  <link rel="preload" href="/assets/fonts/ibm-plex-sans-arabic-400-arabic.woff2" as="font" type="font/woff2" crossorigin>
  <link rel="stylesheet" href="/assets/fonts.css">
  <link rel="stylesheet" href="/assets/site.css">
  <script src="/assets/theme.js"></script>
  <script src="/assets/main.js" defer></script>${scripts}
</head>
<body>
  <svg class="sprite" aria-hidden="true" focusable="false">
    <defs>
      ${icons.trim().split("\n").join("\n      ")}
    </defs>
  </svg>

  <a class="skip" href="#main">${t.skip}</a>

  <!-- The pigeon dives down this rail as the page is read: download, top to bottom -->
  <div class="flight-rail" aria-hidden="true">
    <span class="flight-rail__track"></span>
    <span class="flight-rail__trail"></span>
    <span class="flight-rail__bird"><span class="mark"></span></span>
  </div>

  <header class="site-header" id="top">
    <div class="wrap site-header__inner">
      <a class="brand" href="${homeUrl}" aria-label="${t.home}">
        <img class="brand__icon" src="/assets/zajil-icon.svg" alt="" width="40" height="40">
        <span class="brand__name">${t.name}</span>
      </a>

      <nav class="nav" id="site-nav" aria-label="${t.menu}">
        <ul class="nav__list">
            ${nav}
          <li class="nav__cta"><a class="btn btn--primary" href="${downloadUrl}">${t.getFree}</a></li>
        </ul>
      </nav>

      <div class="site-header__actions">
        <a class="switch switch--lang" href="${twin}" hreflang="${otherLang}" aria-label="${t.otherLabel}" data-lang-switch>
          <span class="switch__track" aria-hidden="true">
            <span class="switch__knob"></span>
            <span class="switch__opt switch__opt--en">EN</span>
            <span class="switch__opt switch__opt--ar">ع</span>
          </span>
        </a>
        <button class="switch switch--theme" type="button" role="switch" aria-checked="false" aria-label="${t.themeLabel}" data-theme-toggle>
          <span class="switch__track" aria-hidden="true">
            <span class="switch__knob">
              <svg class="icon icon--sun"><use href="#i-sun"/></svg>
              <svg class="icon icon--moon"><use href="#i-moon"/></svg>
            </span>
            <svg class="icon switch__opt switch__opt--sun"><use href="#i-sun"/></svg>
            <svg class="icon switch__opt switch__opt--moon"><use href="#i-moon"/></svg>
          </span>
        </button>
        <a class="btn btn--primary btn--sm header-cta" href="${downloadUrl}">${t.get}</a>
        <button class="icon-btn nav-toggle" type="button" aria-expanded="false" aria-controls="site-nav" aria-label="${t.menuLabel}" data-nav-toggle>
          <svg class="icon icon--open" aria-hidden="true"><use href="#i-menu"/></svg>
          <svg class="icon icon--close" aria-hidden="true"><use href="#i-close"/></svg>
        </button>
      </div>
    </div>
  </header>

${body.trimEnd()}

  <footer class="site-footer">
    <div class="wrap site-footer__inner">
      <div class="site-footer__brand">
        <a class="brand" href="${homeUrl}" aria-label="${t.home}">
          <img class="brand__icon" src="/assets/zajil-icon.svg" alt="" width="40" height="40">
          <span class="brand__name">${t.fullName}</span>
        </a>
        <p>${t.footerAbout}</p>
      </div>
      <nav class="site-footer__nav" aria-label="${t.footerNav}">
        <ul>
            ${links(t.footerSite)}
        </ul>
        <ul>
            ${links(t.footerRepo)}
        </ul>
      </nav>
    </div>
    <div class="wrap site-footer__base">
      <p>${t.base}</p>
      <a class="to-top" href="#top">${t.toTop}</a>
    </div>
  </footer>
</body>
</html>
`;
}

export function build() {
  rmSync(OUT, { recursive: true, force: true });
  mkdirSync(OUT, { recursive: true });
  const icons = readFileSync(join(SRC, "icons.svg"), "utf8");

  for (const file of walk(PAGES)) {
    const source = readFileSync(join(PAGES, file), "utf8");
    const { meta, body } = parse(file, source);
    const url = urlOf(file);
    const lang = url.startsWith("/en/") ? "en" : "ar";
    const out = join(OUT, file);
    mkdirSync(dirname(out), { recursive: true });
    writeFileSync(out, layout({ lang, url, meta, body, icons }));
  }

  cpSync(join(SRC, "assets"), join(OUT, "assets"), { recursive: true });
  cpSync(join(ROOT, "docs", "screenshots"), join(OUT, "screenshots"), { recursive: true });
  for (const name of ["zajil-icon.svg", "zajil-mark.svg"]) {
    cpSync(join(ROOT, "assets", "icon", name), join(OUT, "assets", name));
  }
  const appIcons = join(ROOT, "apps", "desktop", "src-tauri", "icons");
  cpSync(join(appIcons, "32x32.png"), join(OUT, "assets", "icon-32.png"));
  cpSync(join(appIcons, "128x128@2x.png"), join(OUT, "assets", "icon-256.png"));

  return walk(OUT).map((p) => relative(OUT, join(OUT, p)));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const files = build();
  console.log(`site/dist: ${files.length} files`);
}
