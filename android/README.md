<div dir="rtl">

# زاجِل لأندرويد

تطبيق Kotlin مع Jetpack Compose، فوق محرّك زاجِل نفسه (`zajil-core`) عبر `crates/zajil-android` وUniFFI. القرار وأسبابه في [ADR-0012](../docs/adr/0012-android.md)، والمراحل A1 إلى A5 في [ROADMAP](../docs/ROADMAP.md).

> **English summary.** A Gradle project (AGP 9.4.1, Gradle 9.8.2, Kotlin 2.4.21, JDK 17). `./gradlew assembleDebug` also builds the Rust engine for arm64-v8a with cargo-ndk and generates its Kotlin bindings (`build-engine.sh`). Every build checks the merged manifest against ADR-0012's permissions and exported components. Emulator tests use the local `zajil-testserver` through `adb reverse`, never the internet.

## ما فيه (A1)

| الملف | ما يفعله |
|---|---|
| `build-engine.sh` | يبني المحرّك (`libzajil_android.so`، release، `arm64-v8a`، API 26) ويولّد ملف Kotlin منه بـ `uniffi-bindgen`. يشغّله Gradle قبل كل بناء (المهمة `buildEngine`) |
| `app/build.gradle.kts` | البناء، و`CheckManifest`: يفشل البناء إن طلب الـ manifest المدموج صلاحية أو صدّر مكوّنًا غير ما في ADR-0012 (القسم 11) |
| `ZajilApp.kt` | يحمّل المحرّك، ويعطي `Context` لفاحص الشهادات (`Tls.init`) |
| `downloads/DownloadCenter.kt` | قائمة التحميلات (`files/downloads.json`، كتابة آمنة بـ `AtomicFile`)، والمحرّك، ونقل الملف المكتمل |
| `downloads/DownloadService.kt` | الخدمة الأمامية (`dataSync`) وإشعار التقدم بأزرار «إيقاف مؤقت» و«إلغاء التحميل». تعمل فقط ما دام تحميل يعمل |
| `downloads/BootReceiver.kt` | بعد إعادة تشغيل الهاتف: يكمل وحده قبل أندرويد 15، ومن 15 إشعار «اضغط للاستكمال» |
| `downloads/Publisher.kt` | ينقل الملف المكتمل إلى `Download/Zajil` (MediaStore، وبالمسار في أندرويد 8 و9) |
| `ui/` | الشاشة الواحدة: الرابط، و«تحميل» و«لصق»، وقائمة التحميلات |
| `make-icons.py` | يكتب أيقونات التطبيق والإشعار من `assets/icon/zajil-mark.svg` نفسه، بلا إعادة رسم |

النصوص في `res/values/strings.xml` (العربية، الافتراضية) و`res/values-en/strings.xml`. ولغة التطبيق تُختار أيضًا من إعدادات أندرويد (لغة لكل تطبيق).

## البناء

يحتاج: Android SDK (منصة 37، وNDK ‏29.0.14206865)، وJDK 17، وRust بهدف `aarch64-linux-android`، و`cargo-ndk`:

```bash
rustup target add aarch64-linux-android
```

```bash
cargo install cargo-ndk --version 4.1.2 --locked
```

ثم من مجلد `android/` (وفيه `local.properties` بسطر `sdk.dir=…`، ولا يُرفع إلى المستودع):

```bash
JAVA_HOME=/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home ./gradlew assembleDebug
```

نسخة التطوير `io.github.motalim_ai.zajil.debug`، فتعيش بجانب النسخة المثبّتة. ونسخة release (`assembleRelease`) بلا توقيع: التوقيع في الـ CI فقط (A4).

## التجربة على المحاكي

الاختبارات لا تتصل بالإنترنت أبدًا: سيرفر الاختبار `zajil-testserver` يعمل على الجهاز المضيف، والتطبيق يصل إليه على `127.0.0.1` عبر `adb reverse`. (لا عبر `10.0.2.2`: من أندرويد 17 يحتاج التطبيق صلاحية «الشبكة المحلية» لعنوان خاص، وزاجِل لا يطلبها. ADR-0012، القسم 12.)

1. شغّل السيرفر من أعلى المستودع، ومعه ملف يُكتب فيه كل طلب كل ثانية:

   ```bash
   cargo run -p zajil-android --example emulator_server -- /tmp/zajil-requests.log
   ```

   يطبع بصمة SHA-256 للملف (50 ميجابايت عشوائية، وكل اتصال بسرعة 256 كيلوبايت/ث)، ورابطه، ورابط HTTPS بشهادة موقّعة ذاتيًا.
2. وصّل المنفذ:

   ```bash
   adb reverse tcp:8099 tcp:8099
   ```

3. ثبّت التطبيق والصق الرابط المطبوع واضغط «تحميل».

**ما جُرِّب في A1 (يدويًا، أندرويد 17، API 37، المحاكي `Adawat_Phone`):**

| الحالة | الخطوة | النتيجة |
|---|---|---|
| تحميل كامل | «تحميل» | البصمة تطابق، والملف في `Download/Zajil`، والإشعار يتغير مع التقدم |
| إيقاف مؤقت | «إيقاف مؤقت» عند 77%، ثم «استكمال» | توقفت الخدمة الأمامية، ثم طلب كل اتصال ما بعد آخر بايت محفوظ فقط، والبصمة تطابق |
| قتل العملية | `adb shell run-as <الحزمة> kill -9 <pid>` | أعاد أندرويد تشغيل الخدمة بعد ثانية، فكمل التحميل وحده، والبصمة تطابق |
| إيقاف إجباري | `adb shell am force-stop <الحزمة>`، ثم فتح التطبيق | بقي متوقفًا حتى الفتح (كما يريد المستخدم)، ثم كمل من آخر بايت محفوظ |
| إعادة تشغيل المحاكي | `adb reboot` أثناء التحميل | إشعار «تحميل متوقف، اضغط للاستكمال من حيث توقف»، و«استكمال» فيه كمل التحميل، والبصمة تطابق |
| شهادة موقّعة ذاتيًا | رابط HTTPS المطبوع (بعد `adb reverse` لمنفذه) | رُفض بـ `UnknownIssuer` من مخزن أندرويد: «شهادة الموقع غير صالحة، فلم يُحمَّل شيء» |
| نسخة release | `assembleRelease` موقّعة محليًا بمفتاح التطوير للتجربة فقط | R8 لم يكسر شيئًا: التحميل كمل، والحجم 6.2 ميجابايت |
| اللغتان | `adb shell cmd locale set-app-locales <الحزمة> --locales en` | الواجهة من اليسار إلى اليمين بالإنجليزية، ومن اليمين إلى اليسار بالعربية |

**لم يُجرَّب بعد:** أندرويد 8 إلى 14 (لا صورة محاكي لها على الجهاز)، وأي اختبار آلي على المحاكي (كلها يدوية الآن)، ونجاح HTTPS على الجهاز نفسه (النجاح بجهة الاختبار في اختبارات Rust فقط: `crates/zajil-android/tests/downloads.rs`).

</div>
