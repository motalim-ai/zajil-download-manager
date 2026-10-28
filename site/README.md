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

أسماء ملفات الإصدار فيها رقمه (`Zajil_1.0.0_universal.dmg`)، فلا يمكن كتابة رابط `releases/latest/download/<الاسم>` مقدمًا. لذلك كل زر يبدأ برابط صفحة آخر إصدار على GitHub (`releases/latest`)، وهو يعمل دائمًا. ثم يسأل `download.js` واجهة GitHub مرة واحدة عن آخر إصدار، فإن ردّت وضع في كل زر رابط ملفه مباشرة، واسمه وحجمه. ولا يقبل إلا رابطًا تحت `github.com/motalim-ai/zajil-download-manager/releases/download/`. وإن لم ترد الواجهة (بلا شبكة، أو تجاوز حد الطلبات) يبقى رابط صفحة الإصدار.

## النشر على Cloudflare Pages

المالك وحده يُنشئ المشروع ويربط النطاق، من لوحة Cloudflare للحساب الذي فيه `by-wow.com`. الخطوات بالترتيب:

1. **ربط المستودع:** **Workers & Pages** ← **Create application** ← تبويب **Pages** ← **Connect to Git**. سجّل الدخول إلى GitHub بحساب `motalim-ai`، وحين يسأل GitHub عن صلاحيات تطبيق Cloudflare اختر **Only select repositories** ← `zajil-download-manager` وحده. ثم اختر المستودع واضغط **Begin setup**.
2. **إعدادات البناء** (في نفس الصفحة):

   | الإعداد | القيمة |
   |---|---|
   | Project name | `zajil` (ويصير عنوانه `zajil.pages.dev`، أو اسمًا قريبًا منه إن كان مأخوذًا) |
   | Production branch | `main` |
   | Framework preset | `None` |
   | Build command | `node site/build.mjs` |
   | Build output directory | `site/dist` |
   | Root directory (advanced) | اتركه فارغًا: البناء يقرأ `docs/screenshots` و`assets/icon` من جذر المستودع |
   | Environment variables (advanced) | `NODE_VERSION` = `24` |

3. اضغط **Save and Deploy**، وانتظر أول بناء (أقل من دقيقة: لا تنزيلات). ثم افتح عنوان `pages.dev` الذي يظهر وتأكد أن الموقع يعمل، و`/en/` أيضًا.
4. **النطاق:** في المشروع ← **Custom domains** ← **Set up a custom domain** ← اكتب `zajil.by-wow.com` ← **Continue** ← **Activate domain**.
   - **سجل DNS الذي يضيفه Cloudflare بنفسه** في منطقة `by-wow.com`: نوعه **CNAME**، واسمه `zajil`، وقيمته عنوان المشروع (`zajil.pages.dev`)، **Proxied** (السحابة البرتقالية). لا تضفه يدويًا قبل هذه الخطوة.
   - تصدر شهادة HTTPS وحدها، وقد يأخذ تفعيل النطاق بضع دقائق، والحالة تظهر **Active** حين ينتهي.
5. **اختياري:** **Settings** ← **Build** ← **Build watch paths** ← Include: `site/*` و`docs/screenshots/*` و`assets/icon/*`، فلا يُعاد بناء الموقع مع كل تغيير في البرنامج.
6. **لا تفعّل Web Analytics** ولا أي إحصاءات في المشروع: سياسة الخصوصية تقول إن الموقع بلا إحصاءات.

بعد ذلك، كل دمج في `main` ينشر الموقع وحده، وكل Pull Request يأخذ معاينة على عنوان `pages.dev` خاص به.

ملف `_headers` يُنشر مع الموقع (ومنه Content-Security-Policy)، ومعه صفحتا `404.html` (عربية للجذر، وإنجليزية تحت `/en/`). وللتأكد من الترويسات بعد النشر:

```bash
curl -sI https://zajil.by-wow.com/ | grep -i -E 'content-security-policy|referrer-policy|x-content-type-options'
```

</div>

---

## English summary

Zajil's public site, for `https://zajil.by-wow.com`: static HTML and CSS, a few small scripts, no npm packages, no tracking. Arabic is at `/`, English under `/en/`. Fonts (Alexandria, IBM Plex Sans Arabic, OFL-1.1) are kept in the site; Thmanyah Sans is used only where the visitor has it installed (its licence forbids hosting it).

- `node site/check.mjs` builds into `site/dist` and checks every internal link and anchor, twin pages, and that nothing loads from outside or inline.
- The download buttons link to `releases/latest` and, if GitHub's API answers, straight to each file (the asset names carry the version, so `releases/latest/download/<name>` cannot be written ahead).
- **Cloudflare Pages:** Workers & Pages → Create application → Pages → Connect to Git, with the GitHub app limited to this repository; production branch `main`; framework preset None; build command `node site/build.mjs`; output directory `site/dist`; root directory empty (the repository root); `NODE_VERSION=24`; Save and Deploy. Then Custom domains → `zajil.by-wow.com`: Cloudflare adds a proxied CNAME `zajil` → the project's `pages.dev` address itself, and issues the certificate. Optional build watch paths `site/*`, `docs/screenshots/*`, `assets/icon/*`. Do not turn on Web Analytics.
