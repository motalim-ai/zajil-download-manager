<div dir="rtl">

# موقع زاجِل

الموقع العام لزاجِل، على `https://zajil.by-wow.com`. صفحات ثابتة فقط: HTML وCSS، وسكربت صغير واحد لأزرار التحميل. بلا مكتبات من npm، وبلا خطوط من الخارج، وبلا تتبع ولا إحصاءات.

- العربية هي الصفحة الأساسية (`/`)، والإنجليزية تحت `/en/`. ولكل صفحة عربية صفحة إنجليزية بنفس المسار.
- الصفحات: الرئيسية (ومعها الصور)، والتحميل (ومعه خطوات التثبيت والتأكد من الملفات)، والأسئلة الشائعة، وسياسة الخصوصية.

## الملفات

| الملف | ما هو |
|---|---|
| `src/pages/` | محتوى كل صفحة (`<main>` فقط)، وفوقه العنوان والوصف في تعليق |
| `src/assets/site.css` | التنسيق كله، للاتجاهين وللوضع الفاتح والداكن |
| `src/assets/download.js` | يضع رابط كل ملف في زره (انظر أدناه) |
| `src/_headers` | ترويسات الأمان في Cloudflare Pages، ومنها Content-Security-Policy |
| `build.mjs` | يلف كل صفحة بالقالب (الرأس والقائمة والتذييل) ويكتب الموقع في `dist/` |
| `check.mjs` | يبني الموقع ثم يفحصه، ويعمل في الـ CI |

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

أسماء ملفات الإصدار فيها رقمه (`Zajil_1.0.0_universal.dmg`)، فلا يمكن كتابة رابط `releases/latest/download/<الاسم>` مقدمًا. لذلك كل زر يبدأ برابط صفحة آخر إصدار على GitHub (`releases/latest`)، وهو يعمل دائمًا. ثم يسأل `download.js` واجهة GitHub مرة واحدة عن آخر إصدار، فإن ردّت وضع في كل زر رابط ملفه مباشرة، واسمه وحجمه. ولا يقبل إلا رابطًا تحت `github.com/motalim-ai/zajil-download-manager/releases/download/`. وإن لم ترد الواجهة (بلا شبكة، أو تجاوز حد الطلبات، أو لا إصدار منشور بعد) يبقى رابط صفحة الإصدار.

## النشر على Cloudflare Pages

المالك وحده يُنشئ المشروع ويربط النطاق. الإعدادات بالضبط:

1. **Workers & Pages** ← **Create** ← **Pages** ← **Connect to Git**، واختر المستودع `motalim-ai/zajil-download-manager`.
2. إعدادات البناء:

   | الإعداد | القيمة |
   |---|---|
   | Project name | `zajil` |
   | Production branch | `main` |
   | Framework preset | `None` |
   | Build command | `node site/build.mjs` |
   | Build output directory | `site/dist` |
   | Root directory | اتركه فارغًا (جذر المستودع، لأن البناء يقرأ `docs/screenshots` و`assets/icon`) |
   | Environment variable | `NODE_VERSION` = `24` |

3. **اختياري:** في **Settings** ← **Builds** ← **Build watch paths**، اجعل Include: `site/*` و`docs/screenshots/*` و`assets/icon/*`، فلا يُعاد بناء الموقع مع كل تغيير في البرنامج.
4. **النطاق:** في المشروع ← **Custom domains** ← **Set up a custom domain** ← `zajil.by-wow.com`. ولأن `by-wow.com` على Cloudflare، يضيف Cloudflare سجل CNAME من `zajil` إلى عنوان `pages.dev` الخاص بالمشروع بنفسه بعد موافقتك.
5. **لا تفعّل Web Analytics** ولا أي إحصاءات في المشروع: سياسة الخصوصية تقول إن الموقع بلا إحصاءات.

ملف `_headers` يُنشر مع الموقع، ومعه صفحتا `404.html` (عربية للجذر، وإنجليزية تحت `/en/`).

</div>

---

## English summary

Zajil's public site, for `https://zajil.by-wow.com`: static HTML and CSS, one small script for the download buttons, no npm packages, no outside fonts, no tracking. Arabic is at `/`, English under `/en/`.

- `node site/check.mjs` builds into `site/dist` and checks every internal link and anchor, twin pages, and that nothing loads from outside or inline.
- The download buttons link to `releases/latest` and, if GitHub's API answers, straight to each file (the asset names carry the version, so `releases/latest/download/<name>` cannot be written ahead).
- **Cloudflare Pages:** connect the repository; production branch `main`; framework preset None; build command `node site/build.mjs`; output directory `site/dist`; root directory empty (the repository root); `NODE_VERSION=24`; optional build watch paths `site/*`, `docs/screenshots/*`, `assets/icon/*`; custom domain `zajil.by-wow.com`. Do not turn on Web Analytics.
