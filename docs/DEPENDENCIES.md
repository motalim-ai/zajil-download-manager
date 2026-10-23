<div dir="rtl">

# سجل المكتبات وتراخيصها

**أي مكتبة جديدة تُضاف إلى هذا الجدول في نفس الـ Pull Request الذي يضيفها**، مع:
- ترخيصها.
- سبب الحاجة إليها.
- تأكيد أن الترخيص متوافق مع GPL-3.0 (انظر [ADR-0002](adr/0002-license.md)).

الـ CI يفحص التراخيص تلقائيًا بأداة `cargo-deny` حسب الإعدادات في [`deny.toml`](../deny.toml)، **لمعماريات الأنظمة الثلاثة فقط** (x86_64 وarm64). مثلًا `webpki-root-certs` (ترخيص CDLA-Permissive-2.0) يستخدمه `rustls-platform-verifier` في wasm32 فقط، فلا يدخل في أي نسخة من زاجِل. لكن هذا السجل يبقى ضروريًا، لأنه يوثّق **سبب** اختيار كل مكتبة، ويشمل أيضًا ما لا يفحصه `cargo-deny`، مثل مكتبات الواجهة (npm)، والأدوات الخارجية (ffmpeg وyt-dlp)، والخطوط والأيقونات.

## مكتبات Rust

| المكتبة | الإصدار | الترخيص | تُستخدم في | السبب | أُضيفت في |
|---|---|---|---|---|---|
| `dirs` | 7.0.0 | MIT أو Apache-2.0 | `zajil-platform` | مسارات النظام القياسية: Known Folders في ويندوز، وXDG في لينكس (يعتمد على `option-ext` بترخيص MPL-2.0) | M1 |
| `serde` | 1.0.229 | MIT أو Apache-2.0 | `zajil-core` | قراءة ملف الحالة وكتابته | M1 |
| `serde_json` | 1.0.154 | MIT أو Apache-2.0 | `zajil-core` | صيغة JSON لملف الحالة (ADR-0004) | M1 |
| `sha2` | 0.11.1 | MIT أو Apache-2.0 | `zajil-core` | بصمة ملف الحالة، وبصمات الملفات | M1 |
| `tokio` | 1.54.1 | MIT | `zajil-core` و`zajil-testserver` | التشغيل غير المتزامن (async) | M1 |
| `hyper` | 1.12.0 | MIT | `zajil-testserver` (اختبار فقط) | سيرفر HTTP/1.1 للحالات الصعبة | M1 |
| `hyper-util` | 0.1.21 | MIT | `zajil-testserver` (اختبار فقط) | ربط hyper بـ tokio | M1 |
| `http-body-util` | 0.1.5 | MIT | `zajil-testserver` (اختبار فقط) | إرسال الجسم على دفعات، وقطعه في المنتصف | M1 |
| `bytes` | 1.12.1 | MIT | `zajil-testserver` (اختبار فقط) | مخازن البيانات | M1 |
| `reqwest` | 0.13.5 | MIT أو Apache-2.0 | `zajil-core` | عميل HTTP: التحويلات، والمهلات، والبث. **دون الميزات الافتراضية**: rustls دون مزوّد مدمج (نضع `ring`)، ودون بروكسي النظام (M3)، ودون فك الضغط | M1 |
| `rustls-platform-verifier` | 0.7.1 | MIT أو Apache-2.0 | عبر `reqwest` | التحقق من الشهادات بمخزن النظام وقواعده (ADR-0003). `reqwest` يستخدمه تلقائيًا، ونحن نضع مزوّد التشفير `ring`. يعتمد في لينكس على `rustls-native-certs` و`openssl-probe` (MIT أو Apache-2.0)، ولا يستخدم OpenSSL نفسه | M1 |
| `percent-encoding` | 2.3.2 | MIT أو Apache-2.0 | `zajil-core` | فك ترميز اسم الملف من الرابط ومن `filename*` (RFC 8187) | M1 |
| `rustls` | 0.23.46 | Apache-2.0 أو ISC أو MIT | `zajil-core` و`zajil-testserver` | TLS. **بمزوّد التشفير `ring` فقط** | M1 |
| `ring` | 0.17.14 | Apache-2.0 و ISC | عبر `rustls` و`rcgen` | مزوّد التشفير. اخترناه بدل `aws-lc-rs` لأن ترخيص `aws-lc-sys` يتضمن رخصة OpenSSL غير المتوافقة مع GPL-3.0 | M1 |
| `tokio-rustls` | 0.26.6 | MIT أو Apache-2.0 | `zajil-testserver` (اختبار فقط) | سيرفر HTTPS للاختبار | M1 |
| `rcgen` | 0.14.10 | MIT أو Apache-2.0 | `zajil-testserver` (اختبار فقط) | ينشئ في كل تشغيل للاختبارات جهة إصدار (CA) مؤقتة في الذاكرة، ويوقّع بها شهادات السيرفر التجريبي (صالحة، ومنتهية)، وشهادة موقّعة ذاتيًا. **لا توجد أي شهادة أو مفتاح في المستودع**. بمزوّد `ring` فقط | M1 |
| `libc` | 0.2.191 | MIT أو Apache-2.0 | `zajil-platform` (ماك ولينكس فقط) | `O_NOFOLLOW`، وإعادة التسمية دون استبدال (`renameat2` و`renamex_np`) | M1 |
| `windows-sys` | 0.61.2 | MIT أو Apache-2.0 | `zajil-platform` (ويندوز فقط) | `MoveFileExW` مع `MOVEFILE_WRITE_THROUGH`، و`FILE_FLAG_OPEN_REPARSE_POINT`، وعدد الروابط الصلبة | M1 |
| `dunce` | 1.0.5 | CC0-1.0 أو MIT-0 أو Apache-2.0 | `zajil-core` | مسار مجلد التحميل الحقيقي دون بادئة `\\?\` في ويندوز حين لا تلزم، حتى تبقى المسارات المعروضة للمستخدم مقروءة | M1 |
| `icu_properties` | 2.4.0 | Unicode-3.0 | `zajil-core` | بيانات يونيكود (Joining_Type وCanonical_Combining_Class) لقواعد ContextJ في RFC 5892: متى يبقى ZWNJ وZWJ في اسم الملف. موجودة أصلًا في البرنامج عبر `reqwest` ← `url` ← `idna`، فلا تضيف كودًا جديدًا إلى النسخة النهائية | M1 |
| `thiserror` | 2.0.21 | MIT أو Apache-2.0 | كل المكتبات | تعريف أنواع الأخطاء | M1 |
| `tempfile` | 3.27.0 | MIT أو Apache-2.0 | الاختبارات فقط | مجلدات مؤقتة للاختبارات | M1 |

## مكتبات الواجهة والإضافة (npm)

| المكتبة | الإصدار | الترخيص | السبب | أُضيفت في |
|---|---|---|---|---|
| — | — | — | تبدأ من M4 | — |

## أدوات خارجية تُشغَّل كبرامج منفصلة

| الأداة | الترخيص | ملاحظات | الحالة |
|---|---|---|---|
| ffmpeg | LGPL أو GPL حسب طريقة البناء | **تُراجع قبل M6** | لم تُعتمد |
| yt-dlp | Unlicense (يحتاج تحقق عند المراجعة) | **تُراجع قبل M6، مع شروط استخدام المواقع** | لم تُعتمد |

## الخطوط والأيقونات والصور

| المورد | الترخيص | المصدر | ملاحظات |
|---|---|---|---|
| — | — | — | يبدأ من M4. **ممنوع أي مورد من IDM** |

## أدوات البناء والـ CI (لا تُوزَّع مع البرنامج)

| الأداة | الترخيص | الاستخدام |
|---|---|---|
| cargo-deny | MIT أو Apache-2.0 | فحص التراخيص والثغرات في الـ CI |
| actions/checkout (v7.0.1) | MIT | سحب الكود في الـ CI |
| Swatinem/rust-cache (v2.9.3) | LGPL-3.0 | تخزين مؤقت لبناء Rust في الـ CI |
| EmbarkStudios/cargo-deny-action (v2.1.1) | Apache-2.0 | تشغيل cargo-deny في الـ CI |
| Rust toolchain 1.99.1 (`rust-toolchain.toml`) | MIT أو Apache-2.0 | المترجم والأدوات |

</div>
