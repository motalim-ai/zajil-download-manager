<div dir="rtl">

# موقع زاجِل

الموقع العام لزاجِل، على `https://zajil.by-wow.com`. صفحات ثابتة فقط: HTML وCSS، وسكربتات صغيرة بلا مكتبات. بلا مكتبات من npm، وخطوطه محفوظة فيه، وبلا تتبع ولا إحصاءات.

- العربية هي الصفحة الأساسية (`/`)، والإنجليزية تحت `/en/`. ولكل صفحة عربية صفحة إنجليزية بنفس المسار.
- الصفحات: الرئيسية (ومعها الصور)، والتحميل (ومعه خطوات التثبيت والتأكد من الملفات)، والأسئلة الشائعة، وسياسة الخصوصية.

## الملفات

| الملف | ما هو |
|---|---|
| `src/pages/` | محتوى كل صفحة (`<main>` فقط)، وفوقه العنوان والوصف في تعليق |
| `src/icons.svg` | الأيقونات، ويضعها البناء في كل صفحة |
| `src/assets/site.css` | التنسيق كله، للاتجاهين وللوضع الفاتح والداكن |
| `src/assets/fonts.css` و`src/assets/fonts/` | خطا Alexandria وIBM Plex Sans Arabic (OFL-1.1)، ونص ترخيصهما `OFL.txt` |
| خط ثمانية | يُستعمل من جهاز الزائر فقط إن كان مثبّتًا عنده (`local()` في `fonts.css`). ترخيصه يمنع رفع ملفاته أو استضافتها أو تحويلها، فلا ملف منه في المستودع ولا في الموقع |
| `src/assets/theme.js` | يضع الوضع الفاتح أو الداكن المحفوظ قبل أن تظهر الصفحة |
| `src/assets/main.js` | زر الوضع، وقائمة الهاتف، وتبويبات الصور، ومحاكاة التحميل بالأجزاء |
| `src/assets/download.js` | يضع رابط كل ملف في زره (انظر أدناه) |
| `build.mjs` | يلف كل صفحة بالقالب (الرأس والقائمة والتذييل)، ويضع في رأس كل صفحة Content-Security-Policy وسياسة الـ referrer، ويكتب الموقع في `dist/` |
| `check.mjs` | يبني الموقع ثم يفحصه، ويعمل في الـ CI وقبل كل نشر |
| `../.github/workflows/site.yml` | ينشر الموقع على GitHub Pages |

الصور تُنسخ وقت البناء من `docs/screenshots/`، والأيقونة من `assets/icon/` و`apps/desktop/src-tauri/icons/`. لا نسخة ثانية منها هنا.

## التجربة محليًا

```bash
node site/check.mjs
```

```bash
python3 -m http.server 8000 --bind 127.0.0.1 --directory site/dist
```

ثم افتح `http://127.0.0.1:8000/`. والأمر الأول يبني ويفحص: كل رابط داخلي يصل إلى ملف (وإلى `id` في الصفحة إن كان فيه `#`)، ولكل صفحة توأمها باللغة الأخرى، ولا شيء يُحمَّل من خارج الموقع ولا داخل الصفحة نفسها.

## أزرار التحميل

أسماء ملفات الإصدار فيها رقمه (`Zajil_1.0.0_universal.dmg`)، فلا يمكن كتابة رابط `releases/latest/download/<الاسم>` مقدمًا. لذلك كل زر يبدأ برابط صفحة آخر إصدار على GitHub (`releases/latest`)، وهو يعمل دائمًا. ثم يسأل `download.js` واجهة GitHub مرة واحدة عن آخر إصدار، فإن ردّت وضع في كل زر رابط ملفه مباشرة، واسمه وحجمه. ولا يقبل إلا رابطًا تحت `github.com/motalim-ai/zajil-download-manager/releases/download/`. وإن لم ترد الواجهة (بلا شبكة، أو تجاوز حد الطلبات) يبقى رابط صفحة الإصدار.

## النشر على GitHub Pages

الموقع على **GitHub Pages**، بالنطاق `zajil.by-wow.com`. والإعداد في المستودع جاهز: Pages مفعّل بمصدر **GitHub Actions**، والنطاق مضبوط في **Settings** ← **Pages** ← **Custom domain**. فلا ملف `CNAME` في الموقع.

**كيف ينشر:** `.github/workflows/site.yml` يعمل عند كل دمج في `main` يغيّر `site/` أو `docs/screenshots/` أو `assets/icon/` (أو أيقونتي التطبيق اللتين ينسخهما البناء)، ويمكن تشغيله باليد من تبويب **Actions** ← **Website deploy** ← **Run workflow**. وهو:

1. يشغّل `node site/check.mjs`: يبني الموقع في `site/dist` ويفحصه، ويتوقف إن وجد مشكلة، فلا يُنشر شيء مكسور.
2. يرفع `site/dist` بـ `actions/upload-pages-artifact`، وينشره بـ `actions/deploy-pages` في البيئة `github-pages`.

ونشر واحد في الوقت نفسه (`concurrency: pages`). وكل Pull Request يُفحص في الـ CI (وظيفة «Website») ولا يُنشر.

**سجل DNS** في منطقة `by-wow.com` على Cloudflare (يضيفه المالك):

| النوع | الاسم | القيمة | Proxy |
|---|---|---|---|
| CNAME | `zajil` | `motalim-ai.github.io` | **DNS only** (السحابة الرمادية) |

السحابة الرمادية ضرورية: بها يصل الزائر إلى GitHub مباشرة، ويصدر GitHub شهادة HTTPS للنطاق. وبعد أن تظهر الشهادة، فعّل **Enforce HTTPS** في إعدادات Pages.

**الترويسات:** GitHub Pages لا يسمح بترويسات خاصة. لذلك Content-Security-Policy وسياسة الـ referrer في وسوم `<meta>` داخل كل صفحة، و`check.mjs` يتأكد من وجودهما. أما ما لا يُكتب في `<meta>` (`frame-ancestors` و`X-Frame-Options` و`X-Content-Type-Options` و`Permissions-Policy`) فلا يمكن ضبطه على GitHub Pages.

وصفحة `404.html` في الجذر يعرضها GitHub Pages لأي عنوان غير موجود، والإنجليزية `/en/404.html` تبقى للروابط المباشرة.

</div>

---

## English summary

Zajil's public site, for `https://zajil.by-wow.com`: static HTML and CSS, a few small scripts, no npm packages, no tracking. Arabic is at `/`, English under `/en/`. Fonts (Alexandria, IBM Plex Sans Arabic, OFL-1.1) are kept in the site; Thmanyah Sans is used only where the visitor has it installed (its licence forbids hosting it).

- `node site/check.mjs` builds into `site/dist` and checks every internal link and anchor, twin pages, and that nothing loads from outside or inline.
- The download buttons link to `releases/latest` and, if GitHub's API answers, straight to each file (the asset names carry the version, so `releases/latest/download/<name>` cannot be written ahead).
- **GitHub Pages:** `.github/workflows/site.yml` builds, checks and publishes `site/dist` on every merge to `main` that touches the site, the screenshots or the icon (or by hand). DNS: CNAME `zajil` → `motalim-ai.github.io`, DNS only (grey cloud), then Enforce HTTPS. GitHub Pages sets no custom headers, so the CSP and referrer policy are `<meta>` tags in each page.
