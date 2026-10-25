<div dir="rtl">

# ADR-0005: طريقة اتصال إضافة المتصفح بالبرنامج

| البند | القيمة |
|---|---|
| الحالة | **مقبول** (وافق المالك في 2026-10-23) |
| التاريخ | 2026-10-23 |
| صاحب القرار | Motalim (@motalim-ai) |

## السياق

الإضافة (Manifest V3 لـ Chrome وEdge وFirefox) تلتقط التحميل وترسل إلى البرنامج: الرابط، والاسم المقترح، والـ Referer، وكوكيز هذا الرابط فقط. وتطلب أيضًا «تحديث رابط التحميل». ويجب أن:

- يعمل على الأنظمة الثلاثة.
- **لا يستطيع أي موقع إنترنت ولا أي إضافة أخرى** أن يرسل أوامر لزاجِل.
- يعمل مع قيود MV3: لا يوجد صفحة خلفية دائمة، بل service worker في Chrome وEdge.

## الخيارات
| الخيار | المزايا | العيوب |
|---|---|---|
| **Native Messaging** (الطريقة الرسمية في المتصفحات) | المتصفح نفسه يضمن أن إضافتنا فقط تتصل (عبر `allowed_origins` أو `allowed_extensions`). لا يفتح أي منفذ شبكة. مدعوم في Chrome وEdge وFirefox | يحتاج تسجيل manifest مختلفًا لكل نظام ولكل متصفح. والمتصفح يشغّل عملية جديدة مع كل اتصال |
| سيرفر HTTP أو WebSocket على `localhost` | سهل البرمجة | **أي موقع** يستطيع إرسال طلبات إلى `localhost`. يحتاج حماية معقدة من هجمات CSRF وDNS rebinding، ويفتح منفذًا يمكن لأي برنامج على الجهاز الوصول إليه |
| رابط بروتوكول خاص (`zajil://…`) | بسيط | اتجاه واحد فقط، ولا رد. ويستطيع أي موقع فتحه، وتظهر رسالة تأكيد مزعجة |
| مراقبة مجلد التنزيلات | لا يحتاج إضافة | متأخر جدًا، لأن المتصفح بدأ التحميل فعلًا، ولا يعرف الكوكيز |

## القرار

**Native Messaging مع مضيف وسيط صغير**:

```text
الإضافة ──connectNative("io.github.motalim_ai.zajil.host")──▶ zajil-host ──IPC محلي──▶ برنامج زاجِل (المحرّك)
              (stdin/stdout, JSON)                     (Named Pipe / Unix socket)
```

1. **`zajil-host`**: ملف تنفيذي صغير (Rust). يشغّله المتصفح ويتصل به عبر stdin/stdout:
   - كل رسالة JSON مسبوقة بطولها (4 بايت).
   - حجم الرسالة من المضيف إلى الإضافة محدود بـ 1 ميجابايت حسب وثائق Chrome وMDN.
2. **وظيفة المضيف**:
   - يتحقق من كل رسالة: مخطط ثابت، وحد أقصى للحجم، ورفض أي حقل غير معروف.
   - يمررها إلى البرنامج الرئيسي عبر قناة IPC **خاصة بالمستخدم الحالي**: Named Pipe في ويندوز، وUnix socket بصلاحية 0700 في ماك ولينكس.
   - إذا لم يكن البرنامج مفتوحًا، يشغّله.
3. **التسجيل**: يتم عبر `zajil-platform` (ADR-0006).
   - ملف manifest يحتوي `allowed_origins` بمعرّفات إضافتنا فقط في Chrome وEdge، و`allowed_extensions` في Firefox.
   - مكان التسجيل: الريجستري في ويندوز، ومسارات الملفات الخاصة بكل متصفح في ماك ولينكس.
4. **الرسائل لها رقم إصدار للبروتوكول** (`protocol_version`)، حتى تعمل إضافة قديمة مع برنامج أحدث أو العكس، أو ترفض الاتصال برسالة واضحة.
5. **الكوكيز**: تُرسل كوكيز الرابط المطلوب فقط، ولا تُحفظ على القرص إلا للضرورة (ADR-0004). **وقرار المالك على #7 (2026-10-25): في الذاكرة فقط، ولا كوكيز على القرص أبدًا**، والاستثناء الوحيد ملف yt-dlp المؤقت في M6 (ROADMAP).

## العواقب
- المثبّت يسجل المضيف لكل متصفح مدعوم، والبرنامج يتيح «إصلاح التكامل مع المتصفح» من الإعدادات.
- **معرّفات الإضافة** في المتاجر الثلاثة يجب أن تكون معروفة قبل كتابة ملف الـ manifest. وفي Firefox نحدد المعرّف بأنفسنا (`browser_specific_settings.gecko.id`).
- **مخاطر تحتاج تحقق في M5**:
  - Firefox المثبّت عبر Snap أو Flatpak في لينكس قد لا يرى المضيف بسبب العزل (sandbox). يحتاج اختبارًا وحلًا موثقًا.
  - في MV3، الـ service worker قد يتوقف. نتأكد عمليًا من سلوك الاتصال المفتوح بالمضيف في Chrome وEdge.
  - واجهة `downloads` تختلف بين Chrome وFirefox. مثلًا `onDeterminingFilename` موجودة في Chrome فقط حسب آخر ما نعرف. نكتب طبقة توافق صغيرة داخل الإضافة.

## المصادر
- Chrome Native Messaging: https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging
- MDN Native messaging: https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/Native_messaging
- Chrome downloads API: https://developer.chrome.com/docs/extensions/reference/api/downloads
- MDN downloads API: https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/API/downloads

</div>
