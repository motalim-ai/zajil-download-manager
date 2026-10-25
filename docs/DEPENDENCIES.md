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
| `clap` | 4.6.7 | MIT أو Apache-2.0 | `zajil-cli` | تحليل أوامر سطر الأوامر ورسائل المساعدة | M1 |
| `arboard` | 3.7.0 | MIT أو Apache-2.0 | `zajil-platform` (ماك ولينكس فقط) | الحافظة: NSPasteboard في ماك، وX11 أو Wayland (data-control، عبر `wl-clipboard-rs`) في لينكس، لمراقبة الحافظة. بلا دعم الصور. وفي ويندوز نستعمل Win32 مباشرة، فلا تدخل واجهتها في ويندوز (`clipboard-win` و`error-code`، بترخيص BSL-1.0) أي بناء، وهما مستثنيتان من رسم cargo-deny بسببهما في `deny.toml` | M5 |
| `drag` | 2.1.1 | Apache-2.0 أو MIT | `zajil-desktop` | سحب الملف المكتمل من نافذة «اكتمل التحميل» إلى أي برنامج أو مجلد، بسحب النظام نفسه (M5.5). يستدعيه Rust وحده على ملف التحميل نفسه؛ الصفحة تسمّي التحميل ولا تمرّر مسارًا. ومعه `core-graphics` (Apache-2.0 أو MIT) و`objc2-core-video` (Zlib أو Apache-2.0 أو MIT) في ماك، وإصدار آخر من `windows` و`windows-core` وما معهما (Apache-2.0 أو MIT) في ويندوز | M5.5 |
| `objc2-app-kit` | 0.3.2 | Zlib أو Apache-2.0 أو MIT | `zajil-platform` (ماك فقط) | `NSPasteboard.changeCount`، فلا تُقرأ الحافظة إلا بعد نسخة (مراقبة الحافظة). وكانت في الشجرة تحت `arboard` | M5 |
| `getrandom` | 0.4.3 | MIT أو Apache-2.0 | `zajil-platform` و`zajil-core` | أرقام عشوائية من النظام: لاسم الـ named pipe في ويندوز (قناة الإضافة، M5)، ولأسماء مجلدات تنزيل أدوات الفيديو وتشغيلها (M6) | M5 |
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
| `landlock` | 0.4.7 | MIT أو Apache-2.0 | `zajil-platform` (لينكس فقط) | صندوق QuickJS-NG ليوتيوب (`zajil_platform::sandbox`، ADR-0009): يمنع الكتابة خارج مجلده والشبكة. من مشروع Landlock نفسه، بدل كتابة واجهة النواة بـ `unsafe` | M6 |
| `libc` | 0.2.191 | MIT أو Apache-2.0 | `zajil-platform` (ماك ولينكس فقط) | `O_NOFOLLOW`، وإعادة التسمية دون استبدال (`renameat2` و`renamex_np`). ومنذ M2: حجز المساحة (`fallocate` و`F_PREALLOCATE`)، والمساحة الحرة (`statvfs` و`statfs`). ومنذ M6: شجرة عمليات أداة الفيديو (`umask` و`kill` لمجموعة العمليات، و`PR_SET_PDEATHSIG` في لينكس، و`sysctl` و`proc_listallpids` في ماك لإيجاد عمليات الشجرة) | M1 |
| `windows-sys` | 0.61.2 | MIT أو Apache-2.0 | `zajil-platform` (ويندوز فقط) | `MoveFileExW` مع `MOVEFILE_WRITE_THROUGH`، و`FILE_FLAG_OPEN_REPARSE_POINT`، وعدد الروابط الصلبة. ومنذ M2 (الميزتان `Win32_System_IO` و`Win32_System_Ioctl`): الملف المتناثر (`FSCTL_SET_SPARSE`)، والمساحة الحرة (`GetDiskFreeSpaceExW`). ومنذ M6 (الميزتان `Win32_System_JobObjects` و`Win32_System_Diagnostics_ToolHelp`): شجرة عمليات أداة الفيديو في Job Object تنتهي بانتهاء زاجِل، وقائمة عمليات الشجرة | M1 |
| `dunce` | 1.0.5 | CC0-1.0 أو MIT-0 أو Apache-2.0 | `zajil-core` | مسار مجلد التحميل الحقيقي دون بادئة `\\?\` في ويندوز حين لا تلزم، حتى تبقى المسارات المعروضة للمستخدم مقروءة | M1 |
| `icu_properties` | 2.4.0 | Unicode-3.0 | `zajil-core` | بيانات يونيكود (Joining_Type وCanonical_Combining_Class) لقواعد ContextJ في RFC 5892: متى يبقى ZWNJ وZWJ في اسم الملف. موجودة أصلًا في البرنامج عبر `reqwest` ← `url` ← `idna`، فلا تضيف كودًا جديدًا إلى النسخة النهائية | M1 |
| `thiserror` | 2.0.21 | MIT أو Apache-2.0 | كل المكتبات | تعريف أنواع الأخطاء | M1 |
| `jiff` | 0.2.38 | Unlicense أو MIT | `zajil-core` و`zajil-cli` (اختبارات) | الوقت المحلي والمنطقة الزمنية للنظام وقواعد التوقيت الصيفي، لجدولة القوائم. بميزاتها الافتراضية: منطقة النظام على الأنظمة الثلاثة، وقاعدة بيانات المناطق مضمّنة في ويندوز فقط (`jiff-tzdb-platform` و`jiff-tzdb`، بترخيص Unlicense أو MIT، والبيانات نفسها من IANA في الملكية العامة). ومعها `jiff-static` و`jiff-core` (Unlicense أو MIT)، و`portable-atomic` و`portable-atomic-util` (Apache-2.0 أو MIT) | M3 |
| `fluent-bundle` | 0.16.0 | Apache-2.0 أو MIT | `zajil-i18n` | تنسيق رسائل Fluent (ADR-0003) في جهة Rust: قائمة شريط النظام والإشعارات. ومعه `fluent-langneg` و`intl-memoizer` و`type-map` و`rustc-hash` (Apache-2.0 أو MIT)، و`self_cell` (Apache-2.0 أو GPL-2.0-only: نستعمله بـ Apache-2.0)، و`intl_pluralrules` 7.0.2 (Apache-2.0 أو MIT). **وهذه الأخيرة تخطئ في الجمع العربي فوق 100** (103 و111 تعطي «other»)، فلا نختار صيغ الجمع بها: انظر `icu_plurals` | M4 |
| `fluent-syntax` | 0.12.0 | Apache-2.0 أو MIT | `zajil-i18n` (عبر `fluent-bundle`، واختبارات) | قراءة ملفات `.ftl`. والاختبارات تمر بها على كل مفتاح ومتغير | M4 |
| `unic-langid` | 0.9.6 | MIT أو Apache-2.0 | `zajil-i18n` | معرّف اللغة الذي يطلبه `fluent-bundle`. ومعه `unic-langid-impl` (MIT أو Apache-2.0) | M4 |
| `icu_plurals` | 2.4.0 | Unicode-3.0 | `zajil-i18n` | فئات الجمع من CLDR للدالة `PLURAL()` في ملفات الترجمة. من عائلة ICU4X نفسها التي نستعملها (`icu_properties`). ومعها `icu_plurals_data` و`fixed_decimal` و`icu_locale_fallback` و`icu_locale_fallback_data` (Unicode-3.0) | M4 |
| `icu_locale_core` | 2.4.0 | Unicode-3.0 | `zajil-i18n` | معرّف اللغة لـ `icu_plurals`. موجودة أصلًا في البرنامج عبر `url` ← `idna` | M4 |
| `tauri` | 2.12.5 | Apache-2.0 أو MIT | `zajil-desktop` | نافذة الواجهة فوق محرّك الويب الموجود في النظام (ADR-0003): WebView2 في ويندوز، وWKWebView في ماك، وWebKitGTK 4.1 في لينكس (LGPL، يُربط ديناميكيًا، والحد الأدنى Ubuntu 22.04 وDebian 12 بقرار المالك). ومعه `tauri-build` 2.7.4 ومكتباتهما (wry وtao وmuda، وgtk-rs 0.18 في لينكس). كلها في مساحة عمل `apps/desktop/src-tauri` المستقلة، ويفحصها `cargo deny` في الـ CI. **استثناء واحد في `deny.toml`**: RUSTSEC-2024-0370، أي `proc-macro-error` «غير مصونة» (وليست ثغرة)، وهي ماكرو يعمل وقت البناء فقط عبر `glib-macros` 0.18 في لينكس، ولا شيء منها في البرنامج | M4 |
| `tauri-plugin-autostart` | 2.7.0 | Apache-2.0 أو MIT | `zajil-desktop` | التشغيل مع بدء النظام (استثناء من ADR-0006 في تطبيق سطح المكتب): LaunchAgent في ماك، ومفتاح الريجستري في ويندوز، وملف autostart في لينكس | M4 |
| `tauri-plugin-notification` | 2.7.0 | Apache-2.0 أو MIT | `zajil-desktop` | إشعارات النظام عند اكتمال تحميل أو فشله (استثناء من ADR-0006 في تطبيق سطح المكتب، انظر ARCHITECTURE). **ثلاثة استثناءات في `deny.toml`** (RUSTSEC-2026-0370 و0371 و0372)، وهي «غير مصونة» لا ثغرات: `zbus_names` و`zvariant` و`zvariant_derive` دُمجت في `zbus` نفسها فلن تصدر لها نسخ، و`zbus` 5 (D-Bus في لينكس) ما زال يستخدمها عبر هذه الإضافة. ولا ترقية متاحة بعد | M4 |
| `tauri-plugin-dialog` | 2.8.3 | Apache-2.0 أو MIT | `zajil-desktop` | نوافذ الفتح والحفظ التي يعرضها النظام نفسه: اختيار مجلد التحميل، وملف الاستيراد والتصدير. الصفحة لا تقرأ ولا تكتب ملفات بنفسها، بل تمرر المسار المختار إلى أوامر زاجِل | M4 |
| `tauri-plugin-opener` | 2.7.0 | Apache-2.0 أو MIT | `zajil-desktop` | «فتح» التحميل المكتمل بالبرنامج الافتراضي و«فتح المجلد» مع تحديد الملف (M5.5). يستدعيه Rust وحده على مسار التحميل نفسه؛ الصفحة لا تملك صلاحيات هذه الإضافة. ومعه `open` و`is-wsl` و`is-docker` (MIT) | M5.5 |
| `tauri-plugin-single-instance` | 2.5.4 | Apache-2.0 أو MIT | `zajil-desktop` | نسخة واحدة من زاجِل للمستخدم: حين تكون النافذة مخفية في شريط النظام، فتح زاجِل مرة أخرى يُظهرها بدل أن يبدأ برنامجًا ثانيًا على الطوابير نفسها. لا يُستعمل مع `--data-dir` (الاختبارات والصور). لا يجلب حزمًا أخرى | M5.5 |
| `ts-rs` | 12.0.1 | MIT | `zajil-desktop` | توليد أنواع TypeScript من أنواع Rust التي تتبادلها الواجهة والمحرّك (ADR-0003)، والـ CI يتأكد أن الملفات المولَّدة محدّثة | M4 |
| `tempfile` | 3.27.0 | MIT أو Apache-2.0 | الاختبارات فقط | مجلدات مؤقتة للاختبارات | M1 |

## مكتبات الواجهة والإضافة (npm)

مكتبات تطبيق سطح المكتب في القسم «مكتبات الواجهة (npm)، في `apps/desktop`» أدناه.

**الإضافة (`apps/extension`، M5) بلا أي مكتبة:** كل ما فيها من المستودع. وهذه أدوات بنائها وفحصها فقط، لا تدخل الإضافة، والـ CI يفحص تراخيصها بقائمة أدوات `apps/desktop/scripts/check-licenses.mjs` ويشغّل `npm audit`:

| الأداة | الإصدار | الترخيص | السبب | أُضيفت في |
|---|---|---|---|---|
| `typescript` | 6.0.3 | Apache-2.0 | فحص الأنواع في ملفات JavaScript (`checkJs`) | M5 |
| `vitest` | 5.0.5 | MIT | اختبارات الإضافة | M5 |
| `addons-linter` | 10.14.0 | MPL-2.0 | فاحص AMO، وهو ما يشغّله `web-ext lint`. لم نستعمل `web-ext` نفسه لأن جزءه الخاص بأندرويد يجلب `node-forge` بتحذير أمني عالٍ لا إصلاح له بعد | M5 |
| `playwright-core` | 1.63.0 | Apache-2.0 | يشغّل متصفحًا حقيقيًا في اختبار زر «نزّل الفيديو» (Chrome الموجود على جهاز الـ CI، أو Chromium محليًا). لا ينزّل متصفحًا بنفسه | M6 |

## أدوات خارجية تُشغَّل كبرامج منفصلة

| الأداة | الترخيص | ملاحظات | الحالة |
|---|---|---|---|
| ffmpeg وffprobe | LGPL 2.1 أو أحدث (بناؤنا من المصدر الرسمي، بلا `--enable-gpl` ولا `--enable-nonfree` ولا أي مكتبة خارجية) | يُبنيان في الـ CI من المصدر الرسمي الموقّع (`tools/ffmpeg`)، ويُنشران مع مصدرهما في إصدار `tools-ffmpeg-*` من هذا المستودع ([ADR-0009](adr/0009-video-tools.md)). وسينزّلهما زاجِل عند أول تحميل فيديو بموافقة المستخدم (في PR أدوات الفيديو) | معتمدة (M6) |
| yt-dlp | GPLv3+ للملف المستقل كما يُبنى (المصدر Unlicense) | تُنزَّل عند أول تحميل فيديو بموافقة المستخدم، ببصمة مثبّتة من `SHA2-256SUMS` الموقّع (`tools/yt-dlp/pins.txt`) | معتمدة (M6) |
| QuickJS-NG (`qjs`) | MIT | ليوتيوب فقط، إن لم يوجد Deno أو Node. تُنزَّل مع الأدوات بموافقة المستخدم، ببصمة مثبّتة (`tools/quickjs/pins.txt`؛ لا ينشر QuickJS-NG بصمات ولا توقيعات). ولا يشغّلها yt-dlp إلا عبر `zajil-jsbox`، في صندوق وبحدود (ADR-0009، القسم 3) | معتمدة (M6) |
| Deno 2.3+ أو Node 22+ | MIT | ليوتيوب، إن ثبّتها المستخدم بنفسه: لا نوزّعها ولا ننزّلها. وتُقبل فقط إن لم يستطع غير المستخدم أو النظام تغييرها | استثناء بقرار المالك (ADR-0009) |

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
| actions/upload-artifact (v7.0.2) وactions/download-artifact (v8.0.2) | MIT | نقل الملفات بين مهام الـ CI (بناء FFmpeg، M6) |
| actions/cache (v6.1.0) | MIT | تخزين `ffmpeg-testgen` المؤقت في الـ CI (M6) |
| msys2/setup-msys2 (v2.33.0) | MIT | بيئة بناء FFmpeg في ويندوز (M6) |
| GnuPG | GPL-3.0 أو أحدث | التحقق من توقيع مصدر FFmpeg في الـ CI (موجود على أجهزة الـ CI) |
| مُترجما FFmpeg في ويندوز (MSYS2: gcc لـ x64، وclang لـ arm64) | GPL-3.0 مع استثناء مكتبات التشغيل (libgcc)، وApache-2.0 مع استثناء LLVM (compiler-rt)، والملكية العامة/ZPL لكود بدء التشغيل في MinGW-w64 | الربط الثابت يضع في `ffmpeg.exe` كود بدء التشغيل وlibgcc (x64) أو compiler-rt (arm64). والاستثناءات تعفي الملف الناتج من شروط تلك التراخيص ومن إلزام نشر إشعاراتها. أما winpthreads (ترخيصها يطلب إشعارًا) فلا تدخل: `build.sh` يفرض خيوط ويندوز (`HAVE_W32THREADS 1` و`HAVE_PTHREADS 0`) |
| Rust toolchain 1.99.1 (`rust-toolchain.toml`) | MIT أو Apache-2.0 | المترجم والأدوات |

</div>

<div dir="rtl">

## مكتبات الواجهة (npm)، في `apps/desktop`

الإصدارات مثبتة في `package-lock.json`، و`npm ci` يقرأ منه فقط. والـ CI يفحص التراخيص بقائمة مسموحة (`scripts/check-licenses.mjs`): للمكتبات التي تدخل البرنامج قائمة، ولأدوات البناء والاختبار قائمة أوسع قليلًا. ويشغّل `npm audit` أيضًا.

| المكتبة | الإصدار | الترخيص | تدخل البرنامج؟ | السبب |
|---|---|---|---|---|
| `svelte` | 5.57.2 | MIT | نعم (وقت التشغيل) | مكتبة الواجهة (ADR-0008) |
| `@fluent/bundle` | 0.19.1 | Apache-2.0 | نعم | تنسيق رسائل Fluent في الواجهة، مع الدالة `PLURAL` |
| `@tauri-apps/api` | 2.12.4 | Apache-2.0 أو MIT | نعم | أوامر المحرّك وأحداثه، وعنوان النافذة |
| `@fontsource/ibm-plex-sans-arabic` | 5.3.0 | OFL-1.1 (الخط)، MIT (الحزمة) | نعم (ملفات الخط) | خط الواجهة داخل البرنامج، لا يُحمَّل من الإنترنت |
| أيقونات Lucide (`lucide-static` 1.61.0) | 1.61.0 | ISC، وما جاء منها من Feather بترخيص MIT | نعم (منسوخة في `src/lib/lucide.ts`) | أيقونات تصميم M5.5 (#83). **ليست حزمة npm**: 41 أيقونة منسوخة كما هي، ونص ترخيص الحزمة في رأس الملف وفي `public/lucide-LICENSE.txt`، الذي يُنسخ كما هو إلى البناء فيصل مع البرنامج (مراجعة #96: تعليق الترخيص وحده يحذفه البناء). تُضاف الأيقونات بنسخها من الحزمة نفسها لا من مكان آخر |
| `vite` | 8.3.5 | MIT | لا (بناء) | بناء الواجهة |
| `@sveltejs/vite-plugin-svelte` | 7.3.1 | MIT | لا | ربط Svelte بـ Vite |
| `typescript` | 6.0.3 | Apache-2.0 | لا | الأنواع (الإصدار 7 لا يدعمه `svelte-check` بعد) |
| `svelte-check` | 4.7.7 | MIT | لا | فحص الأنواع وتحذيرات الوصول (أخطاء في الـ CI) |
| `@tsconfig/svelte` | 5.0.8 | MIT | لا | إعدادات TypeScript |
| `vitest` | 5.0.5 | MIT | لا (اختبار) | اختبارات منطق الواجهة |
| `@tauri-apps/cli` | 2.12.4 | Apache-2.0 أو MIT | لا (بناء) | بناء التطبيق وتوليد الأيقونات |
| `pngjs` | 7.0.0 | MIT | لا (اختبار) | قراءة صور الواجهة في اختبار الأسماء المختلطة |
| `@tauri-apps/plugin-dialog` | 2.8.3 | Apache-2.0 أو MIT | نعم | واجهة الصفحة لنوافذ الفتح والحفظ (`tauri-plugin-dialog`) |
| `@tauri-apps/plugin-notification` | 2.7.0 | Apache-2.0 أو MIT | نعم | إرسال الإشعار من الصفحة |
| `@tauri-apps/plugin-autostart` | 2.7.0 | Apache-2.0 أو MIT | نعم | تفعيل التشغيل مع بدء النظام وإيقافه من الإعدادات |
| `@resvg/resvg-js` | 2.6.2 | MPL-2.0 | لا (أداة) | رسم أيقونة شريط النظام في ماك من `zajil-mark.svg` (`scripts/tray-icon.mjs`). والصورة الناتجة في المستودع |
| `axe-core` | 4.14.0 | MPL-2.0 | لا (اختبار) | فحص إمكانية الوصول الآلي في بناء `ui-test` فقط. والـ CI يتأكد أن صفحة الإصدار ليس فيها شيء منه |

</div>
