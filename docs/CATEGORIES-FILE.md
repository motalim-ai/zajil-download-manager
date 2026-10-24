<div dir="rtl">

# مواصفات ملف الفئات `categories.json` (المخطط v1)

| البند | القيمة |
|---|---|
| الحالة | مُطبَّق في M4 |
| القرار | قرار المالك على خطة M4 ([#6](https://github.com/motalim-ai/zajil-download-manager/issues/6)): ملف منفصل عن `queues.json`، لأن الفئات إعدادات يعدّلها المستخدم، والقوائم حالة عمل. وبنفس قواعده ([ADR-0007](adr/0007-queues-and-settings-storage.md)) |
| الكود | `crates/zajil-core/src/category.rs` |

## المكان

في مجلد البيانات بجانب `queues.json`: `AppPaths::data_dir()`، أو `zajil --data-dir`. ومعه `categories.lock`: قفل تأخذه كل عملية تعدّل الملف.

**الملف غير الموجود** يعني الفئات الخمس المدمجة، كلها دون مجلد خاص.

## الحقول

| الحقل | النوع | المعنى |
|---|---|---|
| `format` | نص | دائمًا `"zajil-categories"` |
| `schema_version` | عدد | دائمًا `1` في هذا المخطط |
| `categories` | قائمة | الفئات بالترتيب. الترتيب مهم: أول فئة تطابق تفوز |
| `categories[].id` | نص | معرّف ثابت، من 1 إلى 32: حروف إنجليزية صغيرة وأرقام و`-` و`_`. المدمجة: `programs` و`video` و`music` و`documents` و`archives` |
| `categories[].name` | نص أو `null` | الاسم الذي أعطاه المستخدم، من 1 إلى 64 حرفًا مرئيًا: لا رموز تحكم، ولا رموز تنسيق خفية مثل U+202E. `null` لفئة مدمجة لم يُغيَّر اسمها، **فتعرض الواجهة اسمها المترجَم** (المفتاح `category-<id>`)، فيتبع لغة الواجهة |
| `categories[].extensions` | قائمة | الامتدادات دون النقطة، بحروف صغيرة: من 1 إلى 16 حرفًا إنجليزيًا أو رقمًا. وحتى 200 امتداد |
| `categories[].sites` | قائمة | أسماء المواقع بحروف صغيرة، وبـ punycode لغير اللاتينية. الموقع يطابق نفسه ومواقعه الفرعية: `example.com` يطابق `files.example.com`، ولا يطابق `badexample.com`. وحتى 200 موقع |
| `categories[].folder` | نص أو `null` | المجلد الذي تذهب إليه تحميلات الفئة، مسار مطلق دون `..`. `null`: تبقى في المجلد الذي أُعطي للتحميل |
| `updated_at` | نص | وقت آخر تعديل، بصيغة RFC 3339 بتوقيت UTC |
| `checksum` | نص | `sha256:` ثم البصمة، بنفس الصيغة القانونية لملف الحالة ([STATE-FILE](STATE-FILE.md#البصمة-والصيغة-القانونية)) |

## كيف يجد التحميل فئته

1. **أول فئة بالترتيب فيها موقع** يطابق اسم المضيف في الرابط النهائي، بعد التحويلات.
2. **وإلا أول فئة بالترتيب فيها امتداد الملف**: الجزء بعد آخر نقطة، دون اعتبار لحالة الحروف. فـ `x.tar.gz` يُعرف بـ `gz`.
3. **وإلا لا فئة**: يبقى التحميل في المجلد الذي أُعطي له.

**الاسم** هو اسم الملف الذي يقرره المحرّك: من `Content-Disposition` إن وُجد، وإلا من الرابط. لذلك يُختار المجلد **بعد أن يرد السيرفر وقبل إنشاء أي ملف** (`Engine::download_choosing_until`).

**متى تُستعمل الفئات**:
- `zajil get` و`zajil queue add` **دون** `--dir`.
- أو معه ومع `--by-category`: عندها `--dir` هو المجلد الاحتياطي، للملف الذي ليس لفئته مجلد.
- و`--dir` وحده يفوز دائمًا.

**الفئات المدمجة دون مجلدات افتراضيًا**، حتى لا يُنشئ البرنامج مجلدات لم يطلبها المستخدم. ويحدد المستخدم المجلد بـ `zajil category set <id> --folder` (والواجهة في M4).

## قواعد القراءة

يُرفض الملف، **ولا يُكتب فوقه**، إذا:
- كان رابطًا رمزيًا أو رابطًا صلبًا أو غير ملف عادي.
- كان أكبر من 1 ميجابايت.
- لم يكن JSON صالحًا، أو كان `format` غيره، أو `schema_version` غير مدعوم.
- احتوى حقلًا غير معروف، أو لم تطابق البصمة.
- كان فيه معرّف مكرر أو غير صالح، أو اسم أو امتداد أو موقع غير صالح، أو مجلد غير مطلق أو فيه `..`، أو أكثر من 64 فئة.

## قواعد الكتابة

- **كل تعديل يمر عبر `CategoryStore::update`**: يأخذ `categories.lock` (الذي لا يُفتح عبر رابط أبدًا)، ثم يقرأ ويتحقق، ثم يعدّل، ثم يتحقق من النتيجة، ثم يكتب.
- **الكتابة الآمنة** بنفس دالة ملف الحالة: ملف مؤقت لا يقرؤه إلا صاحبه (صلاحيات `0600` في ماك ولينكس، وقائمة محمية في ويندوز)، ثم `fsync`، ثم إعادة تسمية، ثم `fsync` للمجلد. ونفس نافذة ماك الصغيرة المعروفة ([STATE-FILE](STATE-FILE.md#قواعد-الكتابة)، القاعدة 0).

</div>

---

## English summary

`categories.json` in the data folder (schema v1), apart from `queues.json` (owner's decision on the M4 plan): `format` (`zajil-categories`), `schema_version`, `categories [{id, name, extensions, sites, folder}]`, `updated_at`, `checksum` (same canonical SHA-256). A missing file means the five built-in categories (programs, video, music, documents, archives) with no folders; a built-in category with `name: null` is shown by its translated name. A download's category: the first category, in order, with a site matching the final URL's host (or a parent domain), else the first with the file's extension, else none. The folder is chosen after the server answers (the name may come from `Content-Disposition`) and before any file is created. Same rules as the queues file: strict reading (no links, size limit, unknown fields refused), never overwritten when damaged, owner-only safe writes, `categories.lock` from read to write.
