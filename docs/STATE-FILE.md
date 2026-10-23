<div dir="rtl">

# مواصفات ملف الحالة `.zajil-state` (المخطط v1)

| البند | القيمة |
|---|---|
| الحالة | مُطبَّق في M1 |
| القرار | [ADR-0004](adr/0004-state-file-format.md). المثال في الـ ADR كان للتوضيح، وهذا هو المخطط النهائي |
| الكود | `crates/zajil-core/src/state.rs` |

## الملفات

لكل تحميل، في مجلد التحميل:
- `<الاسم>.zajil-part`: البيانات.
- `<الاسم>.zajil-state`: الحالة، بصيغة JSON وترميز UTF-8.

عند الانتهاء يُعاد تسمية `.zajil-part` إلى `<الاسم>`، ويُحذف ملف الحالة.

## الحقول

| الحقل | النوع | المعنى |
|---|---|---|
| `format` | نص | دائمًا `"zajil-state"` |
| `schema_version` | عدد | دائمًا `1` في هذا المخطط. أي رقم آخر يُرفض برسالة واضحة |
| `url` | نص | الرابط الذي طلبه المستخدم |
| `final_url` | نص | الرابط بعد التحويلات (redirects) |
| `validator.etag` | نص أو `null` | قيمة `ETag` كما أرسلها السيرفر |
| `validator.last_modified` | نص أو `null` | قيمة `Last-Modified` كما أرسلها السيرفر |
| `validator.date` | نص أو `null` | قيمة `Date` من نفس الرد. نحتاجها لنعرف هل `last_modified` مُحقِّق «قوي» (RFC 9110 §8.8.2.2) |
| `content_length` | عدد أو `null` | حجم الملف كاملًا إن كان معروفًا |
| `accept_ranges` | منطقي | هل أعلن السيرفر `Accept-Ranges: bytes` |
| `had_credentials` | منطقي، اختياري | هل كان في الرابط الذي أعطاه المستخدم اسم مستخدم أو كلمة مرور. **القيمتان نفسهما لا تُحفظان أبدًا** (القاعدة 7 في ADR-0004). فائدته: إذا رد السيرفر بـ 401 عند الاستكمال، يقول زاجِل إن التحميل يحتاج بيانات الدخول من جديد **فقط** عندما يكون `true`. وإلا فهو 401 عادية (رابط موقّع انتهت صلاحيته مثلًا). أُضيف إلى المخطط v1 قبل نشره، بقرار المالك على #28. وإذا غاب عن الملف (مثل الملف الثابت للاختبار) فقيمته `false` |
| `segments` | قائمة | الأجزاء، مرتبة ومتصلة: تبدأ من البايت 0 وتصل إلى آخر بايت دون فجوات أو تداخل. جزء واحد للاتصال الواحد، وجزء لكل اتصال في التحميل المتوازي (M2) |
| `segments[].start` | عدد | أول بايت في الجزء |
| `segments[].end` | عدد أو `null` | آخر بايت (شامل). `null` إذا كان الحجم غير معروف |
| `segments[].done` | عدد | عدد البايتات من `start` المكتوبة **والمثبَّتة على القرص** |
| `created_at` و`updated_at` | نص | وقت بصيغة RFC 3339 بتوقيت UTC |
| `checksum` | نص | `sha256:` ثم بصمة SHA-256 بالـ hex للصيغة القانونية (القسم التالي) |

## البصمة والصيغة القانونية

البصمة **لا تعتمد على ترتيب الحقول** في الملف ولا في الكود:
1. يُقرأ الملف كما هو، وتُوضع قيمة `checksum` نصًا فارغًا `""` (الحقل يبقى موجودًا).
2. يُكتب JSON مضغوطًا، دون أي مسافات، و**أعضاء كل كائن مرتبة حسب المفتاح** بترتيب نقاط الترميز (code points)، في كل المستويات. والنصوص والأرقام كما يكتبها JSON المعتاد (الأرقام الصحيحة دون كسور).
3. البصمة = `sha256:` + SHA-256 بالـ hex لنص UTF-8 الناتج.

هذا يساوي في Python: `json.dumps(state, sort_keys=True, separators=(",", ":"), ensure_ascii=False)`.

**ملف ثابت للاختبار**: `crates/zajil-core/tests/fixtures/state-v1.golden.json` كُتب من هذه المواصفة بسكربت مستقل، وأعضاؤه بترتيب مختلف عن الكود. يجب أن تقبله كل نسخة من زاجِل تقرأ المخطط v1 (`the_golden_v1_file_is_always_accepted`).

## قواعد القراءة

**ملف الحالة مُدخل غير موثوق.** البصمة ليست توقيعًا، والملف قد يأتي من جهاز آخر (ADR-0004 يدعم نقل مجلد تحميل غير مكتمل). فكل ما فيه، ومنه `url` و`final_url` والأرقام، يُعامل كبيانات من الإنترنت.

يُرفض الملف، **ولا يُخمَّن أبدًا**، إذا:
- كان رابطًا رمزيًا (symlink) أو رابطًا صلبًا (hard link) أو أي شيء غير ملف عادي. يُفتح عبر `zajil_platform::fs::open_regular_nofollow`، والفحص على الملف المفتوح نفسه.
- كان أكبر من 1 ميجابايت.
- لم يكن JSON صالحًا، أو كان ناقصًا.
- كان `format` غير `zajil-state`.
- كان `schema_version` غير مدعوم.
- احتوى حقلًا غير معروف (هذا يمنع أيضًا حفظ الكوكيز أو الأسرار فيه بالخطأ).
- لم تطابق البصمة.
- كانت الأرقام مستحيلة: جزء ينتهي قبل أن يبدأ، أو `done` أكبر من حجم الجزء، أو جزء يتجاوز نهاية الملف، أو أجزاء لا تبدأ من 0، أو بينها فجوة أو تداخل، أو لا تصل إلى نهاية الملف. والملف مجهول الحجم له جزء واحد مفتوح النهاية بالضبط. ولا يمكن أن يتجاوز أي حساب هنا حدود الأرقام (M2).

## قواعد الكتابة

0. **لا يقرأ الملفَ إلا صاحبه** (قرار المالك على #28): `url` و`final_url` قد يحملان توكنًا يحتاجه الاستكمال (`?sig=…`)، فيُنشأ الملف المؤقت بـ `zajil_platform::fs::create_private`: صلاحيات `0600` في ماك ولينكس، وفي ويندوز قائمة صلاحيات محمية (لا ترث شيئًا من المجلد) فيها إذن واحد للمستخدم الحالي. وإعادة التسمية فوق الملف القديم تحفظ ذلك. والتوكن لا يُطبع أبدًا.
1. البيانات تُثبَّت في `.zajil-part` (`fsync`) **قبل** حفظ حالة تعدّها مكتملة.
2. الحفظ في خطوة واحدة:
   - يُحذف أي `<الاسم>.zajil-state.tmp` قديم أولًا. وإذا كان رابطًا رمزيًا، يُحذف الرابط نفسه ولا يُلمس ما يشير إليه.
   - يُنشأ الملف المؤقت بـ `create_private` (مثل `create_new`)، فيُرفض أي شيء يظهر مكانه في اللحظة نفسها.
   - `fsync`، ثم استبدال الملف القديم (`zajil_platform::fs::replace_file`).
   - في ماك ولينكس: `rename` ثم `fsync` للمجلد (`sync_dir`).
   - في ويندوز: `MoveFileExW` مع `MOVEFILE_WRITE_THROUGH`، وإعادة محاولة لمدة ثانية تقريبًا إذا كان برنامج حماية أو فهرسة أو OneDrive يمسك الملف.

## لماذا SHA-256 وليس CRC32C؟

المثال الأول في ADR-0004 ذكر `crc32c`. اخترنا SHA-256، وسُجِّل القرار في [ADR-0004](adr/0004-state-file-format.md#البصمة-sha-256-وليس-crc32c-حُسم-في-m1)، لأن المحرّك يحتاج مكتبة `sha2` أصلًا لبصمات الملفات، فلا نضيف مكتبة ثانية. وحجم ملف الحالة صغير، فالفرق في السرعة لا يُذكر.

</div>

---

## English summary

Schema v1 of `<name>.zajil-state`: a JSON file next to `<name>.zajil-part`. Fields: `format`, `schema_version`, `url`, `final_url`, `validator {etag, last_modified, date}`, `content_length`, `accept_ranges`, `had_credentials` (optional, `false` when absent; never the credentials themselves), `segments [{start, end (inclusive), done}]`, `created_at`, `updated_at` and `checksum`. Only its owner may read it (mode `0600`, or a protected access list for the current user on Windows), because the URL may hold a token that resuming needs; the token is never printed. The checksum is `sha256:` plus the hex SHA-256 of the compact JSON of the state with an empty checksum. Any parse error, unknown field, unknown version, checksum mismatch or impossible segment rejects the file. Writes go through a temporary file, `fsync`, an atomic rename and a folder sync.
