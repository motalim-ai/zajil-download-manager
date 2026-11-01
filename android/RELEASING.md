<div dir="rtl">

# إصدار زاجِل لأندرويد (A4)

هذه الخطوات **للمالك وحده**. مفتاح توقيع الـ APK لا يراه أي برنامج ولا أي جلسة عمل: يصنعه المالك على جهازه، ويضعه في secrets الـ environment ‏`release` على GitHub بيده. والقرار في [ADR-0012](../docs/adr/0012-android.md) (القسمان 8 و15).

> **English summary.** The owner makes the APK signing key himself (keytool), keeps an offline backup, commits the certificate's SHA-256 to `android/signing-cert.sha256`, and adds four secrets to the `release` environment. A tag `android-vX.Y.Z` on main builds, signs and drafts the release (`.github/workflows/android.yml`); after trying it he publishes it and runs the workflow's `channel` job. Nothing here runs until the owner lifts the Android hold.

## 1. مفتاح توقيع الـ APK (مرة واحدة)

**على جهاز المالك، لا على أي جهاز آخر:**

```bash
keytool -genkeypair -v -keystore zajil-android.jks -alias zajil -keyalg RSA -keysize 4096 -validity 10000 -dname "CN=Zajil, O=Motalim"
```

- يسأل عن كلمة مرور: اختر كلمة قوية، واحفظها في مدير كلمات المرور.
- **نسخة احتياطية** من `zajil-android.jks` وكلمة مروره خارج GitHub (مثل فلاشة التحديث نفسها). **إن ضاع المفتاح، لا يستطيع أي مستخدم التحديث** إلا بحذف زاجِل وتثبيته من جديد.
- لا يوضع الملف في المستودع أبدًا (`*.jks` في `android/.gitignore`).

## 2. بصمة الشهادة (عامة، تُنشر)

```bash
keytool -list -v -keystore zajil-android.jks -alias zajil | grep 'SHA256:'
```

- ضع البصمة (بالنقطتين أو بدونهما) في ملف `android/signing-cert.sha256`، سطرًا واحدًا، في PR.
- workflow الإصدار يرفض أي APK لا تطابق شهادته هذه البصمة.
- وتُكتب في README وصفحة التحميل، ليتحقق منها المستخدم بـ `apksigner verify --print-certs`.

## 3. الـ secrets في الـ environment ‏`release`

من GitHub: Settings ← Environments ← `release` ← Environment secrets. أو من الطرفية:

```bash
base64 -i zajil-android.jks | GH_TOKEN=$(gh auth token -u motalim-ai) gh secret set ZAJIL_ANDROID_KEYSTORE_B64 --env release --repo motalim-ai/zajil-download-manager
```

```bash
GH_TOKEN=$(gh auth token -u motalim-ai) gh secret set ZAJIL_ANDROID_KEYSTORE_PASSWORD --env release --repo motalim-ai/zajil-download-manager
```

```bash
GH_TOKEN=$(gh auth token -u motalim-ai) gh secret set ZAJIL_ANDROID_KEY_PASSWORD --env release --repo motalim-ai/zajil-download-manager
```

```bash
echo zajil | GH_TOKEN=$(gh auth token -u motalim-ai) gh secret set ZAJIL_ANDROID_KEY_ALIAS --env release --repo motalim-ai/zajil-download-manager
```

- الأمران الثاني والثالث يسألان عن القيمة بلا عرضها. ومع `keytool` الحديث، كلمة مرور المفتاح هي كلمة مرور الملف نفسها.
- `TAURI_SIGNING_PRIVATE_KEY` وكلمة مروره موجودان أصلًا (ADR-0011): بهما يُوقَّع ملف التحديث وقائمة yt-dlp.

## 4. كل إصدار

1. ارفع النسخة في `android/version.txt` (مثل `0.2.0`)، واكتب ما تغيّر في `android/release-notes/0.2.0.txt` (يظهر للمستخدم في بطاقة التحديث). وادمج الـ PR.
2. على `main`: tag ‏`android-v0.2.0` وادفعه. يبني الـ CI الـ APK ويختبره، ثم ينتظر موافقتك على الـ environment ‏`release`.
3. بعد الموافقة: يوقّع الـ APK ويتأكد من البصمة، ويكتب `latest-android.json` و`SHA256SUMS-android` موقّعين، وينشئ **مسودة** إصدار.
4. جرّب الـ APK من المسودة على هاتفك. ثم انشر الإصدار.
5. شغّل الـ workflow ‏«Android» يدويًا: `job = channel` و`version = 0.2.0`. فيضع ملف التحديث على `android-channel`، ومن هنا يرى زاجِل المثبّت التحديث.

## 5. تحديث yt-dlp دون إصدار كامل

1. في PR: غيّر `tools/yt-dlp/pins-android.txt` (النسخة والـ SHA-256 من `SHA2-256SUMS` في إصدار yt-dlp)، وارفع الرقم في `tools/sequence-android.txt`.
2. بعد الدمج: workflow ‏«Android» يدويًا من `main`: `job = tools-android`. يتحقق من الملف، ويوقّع `tools-android.json`، ويضعه على `android-channel`.

## 6. رفع التوقف

كل هذا محلي حتى يقرر المالك: الفروع `docs/android-a0` و`feat/android-a1` إلى `feat/android-a4`، وملف الـ workflow. ولا شيء منه يعمل على GitHub قبل الدفع.

</div>
