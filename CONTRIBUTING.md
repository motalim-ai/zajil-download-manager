<div dir="rtl">

# المساهمة في زاجِل

شكرًا لاهتمامك! هذا الملف يشرح كيف تساهم. **اقرأ قاعدة الغرفة النظيفة أولًا، فهي إلزامية.**

## قاعدة الغرفة النظيفة

زاجِل يُكتب **من الصفر**. واتفاقية ترخيص IDM تمنع صراحة الهندسة العكسية وفك البرنامج وتفكيكه (البند 1(c) في الاتفاقية المرفقة مع ملف التثبيت). لذلك:

### ممنوع
- تحليل ملفات IDM، أو فكّها، أو تفكيكها، أو تشغيلها تحت أدوات تتبع لفهم طريقة عملها. وهذا يشمل ملف التثبيت، والبرنامج، والمكتبات، والإضافة، وملفات اللغات، والموارد.
- نقل **أي** كود أو أيقونة أو صورة أو تصميم واجهة أو نصوص أو اسم من IDM.
- إضافة أي ملف من IDM إلى المستودع، ولو للمقارنة.
- المساهمة بكود رأيته في كود IDM المفكوك، أو في أي مصدر مسرّب أو مقرصن.

### مصادرنا المسموحة
1. **المعايير المنشورة**:
   - [RFC 9110](https://www.rfc-editor.org/rfc/rfc9110): طلبات Range، و`If-Range`، و`ETag`.
   - [RFC 6266](https://www.rfc-editor.org/rfc/rfc6266): `Content-Disposition`.
   - [RFC 3659](https://www.rfc-editor.org/rfc/rfc3659): FTP و`REST`.
2. **وثائق المتصفحات الرسمية**: WebExtensions، وNative Messaging، وواجهة `downloads`، في توثيق [Chrome](https://developer.chrome.com/docs/extensions) و[MDN](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions).
3. **وثائق IDM العامة** (موقعه وأسئلته الشائعة)، **لمعرفة الميزات من منظور المستخدم فقط**: ماذا يفعل البرنامج، وليس كيف يفعله. والنتيجة مسجلة في [docs/PARITY.md](docs/PARITY.md).
4. **كود مفتوح المصدر بترخيص متوافق مع GPL-3.0**، مع ذكر المصدر وصاحب الحقوق. أما الكود بترخيص غير متوافق فممنوع نسخه، ويمكن قراءة فكرته العامة فقط.

**إذا لم تكن متأكدًا** من أن مصدرًا مسموح، اسأل في الـ Issue قبل أن تكتب سطرًا واحدًا.

## المكتبات وتراخيصها

**قبل إضافة أي مكتبة:**
1. افحص ترخيصها. يجب أن يكون متوافقًا مع GPL-3.0، والتراخيص المقبولة مذكورة في [ADR-0002](docs/adr/0002-license.md) وفي [`deny.toml`](deny.toml).
2. سجّلها في [docs/DEPENDENCIES.md](docs/DEPENDENCIES.md): الاسم، والإصدار، والترخيص، وسبب الحاجة إليها.
3. الـ CI يفحص التراخيص تلقائيًا (`cargo deny check`)، وأي ترخيص غير مقبول يوقف الـ PR.

## طريقة العمل

1. **كل مرحلة لها Issue**، وكل تغيير يبدأ من Issue.
2. **فرع جديد لكل تغيير**، مثل `feat/resume-if-range` أو `docs/adr-0007`. **ممنوع الدفع مباشرة إلى `main`.**
3. **Pull Request صغير يفعل شيئًا واحدًا.**
4. **رسائل الـ commits وعناوين الـ PR بصيغة [Conventional Commits](https://www.conventionalcommits.org/)**:
   - `feat:` ميزة جديدة.
   - `fix:` إصلاح.
   - `docs:` وثائق.
   - `test:` اختبارات.
   - `refactor:` إعادة هيكلة.
   - `ci:` الـ CI.
   - `chore:` أعمال صيانة.
   - `build:` البناء.
   - `perf:` الأداء.
5. **كل ميزة معها اختباراتها في نفس الـ PR.** واختبارات المحرّك تعمل ضد السيرفر التجريبي المحلي، ولا تتصل بالإنترنت.
6. **لا يُدمج PR إلا بعد**: نجاح الـ CI على **ويندوز وماك ولينكس**، ومراجعة واحدة على الأقل.
7. **أي ميزة لا تعمل على الأنظمة الثلاثة لا تُقبل** ([ADR-0006](docs/adr/0006-platform-abstraction.md)).

## قبل أن ترسل الـ PR

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
```

## الأمان

- **لا تعطّل التحقق من شهادات TLS أبدًا**، ولا حتى في الاختبارات. استخدم شهادات اختبار محلية.
- **نظّف كل اسم ملف أو مسار** يأتي من الشبكة أو من الإضافة.
- **لا تتبع ولا إحصاءات.**
- **الثغرات الأمنية لا تُفتح كـ Issue عام.** اتبع [SECURITY.md](SECURITY.md).

## الترجمة

تُضاف الترجمة ابتداءً من M4 بنظام Fluent. التعليمات تُكتب حينها.

## قواعد السلوك

بمشاركتك أنت توافق على [قواعد السلوك](CODE_OF_CONDUCT.md).

## الرخصة

مساهمتك تُنشر بترخيص [GPL-3.0](LICENSE)، مثل باقي المشروع.

</div>

---

## English summary

**Clean-room rule (mandatory):** IDM's EULA forbids reverse engineering.

**Never:**
- analyze, decompile or disassemble any IDM file
- copy any IDM code, icon, design, text or name
- add IDM files to this repository

**Allowed sources:**
- published standards (RFC 9110, RFC 6266, RFC 3659)
- official browser docs (WebExtensions, Native Messaging, the `downloads` API)
- IDM's public docs, only to learn its features from a user's point of view
- GPL-3.0-compatible open-source code, with attribution

**Before adding a dependency:** check that its license is GPL-3.0-compatible and record it in `docs/DEPENDENCIES.md`. CI enforces this with `cargo-deny`.

**Workflow:**
- Every change starts from an issue and gets its own branch and a small PR.
- Use Conventional Commits.
- No direct pushes to `main`.
- Tests go in the same PR as the feature.
- Merge only after CI passes on Windows, macOS and Linux and the PR has been reviewed.
