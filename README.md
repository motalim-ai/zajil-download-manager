<div dir="rtl">

# زاجِل لإدارة التحميل — Zajil Download Manager

[![CI](https://github.com/motalim-ai/zajil-download-manager/actions/workflows/ci.yml/badge.svg)](https://github.com/motalim-ai/zajil-download-manager/actions/workflows/ci.yml)
[![License: GPL-3.0](https://img.shields.io/badge/License-GPL--3.0-blue.svg)](LICENSE)

> **الحالة: قيد البناء.** المشروع الآن في المرحلة **M0 (ملفات الأساس)**: المستودع فيه الوثائق والقرارات والـ CI فقط، ولا يوجد برنامج يمكن تشغيله بعد.

**زاجِل** برنامج مجاني ومفتوح المصدر لإدارة التحميل. يعمل على **ويندوز وماك ولينكس**، وواجهته **عربية أولًا** (من اليمين إلى اليسار)، والإنجليزية بضغطة زر.

الاسم مأخوذ من **الحمام الزاجل** الذي كان يوصل الرسائل مهما بعدت المسافة. وهذا ما نريده من البرنامج: أن يوصل ملفك كاملًا وسليمًا، حتى لو انقطع الاتصال في الطريق.

## الفكرة

برامج التحميل القوية الموجودة اليوم إما مدفوعة وتعمل على نظام واحد، أو مجانية لكن واجهتها العربية ضعيفة أو غير موجودة. زاجِل يجمع:

- **سرعة أعلى**: يقسم الملف إلى أجزاء ويحمّلها بالتوازي (حتى 32 اتصالًا)، ويعيد تقسيم الأجزاء أثناء التحميل حتى لا يبقى اتصال بلا عمل.
- **لا يضيع شيء**: يكمل التحميل بعد انقطاع الإنترنت أو إغلاق الجهاز، ويتأكد أن الملف على السيرفر لم يتغير.
- **التقاط من المتصفح**: إضافة لـ Chrome وEdge وFirefox ترسل التحميلات إلى زاجِل مباشرة.
- **تنظيم**: قوائم انتظار، وجدولة، وحدود للسرعة، وفئات تلقائية للملفات.
- **عربي حقيقي**: الواجهة مصممة من اليمين إلى اليسار من البداية، وليست ترجمة أُضيفت لاحقًا.
- **خصوصية كاملة**: لا تتبع، ولا إحصاءات، ولا إعلانات، ولا برامج مرفقة.

## ما لن يفعله زاجِل

- لن يتجاوز أي حماية للمحتوى (DRM).
- لن يجمع أي بيانات عنك.
- لن يعتمد على تعريف داخل النظام (kernel driver).

التفاصيل في [وثيقة المتطلبات](docs/PRD.md).

## الوثائق

| الوثيقة | المحتوى |
|---|---|
| [docs/PRD.md](docs/PRD.md) | المتطلبات، ومن المستخدم، وما يفعله البرنامج وما لا يفعله |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | بنية البرنامج: المحرّك، وسطر الأوامر، والواجهة، والإضافة |
| [docs/ROADMAP.md](docs/ROADMAP.md) | المراحل من M1 إلى M8 (ومعها M5.5 والإصدار التجريبي 0.1) وشروط نجاح كل مرحلة |
| [docs/PARITY.md](docs/PARITY.md) | مقارنة ميزات IDM العامة بما سيقدمه زاجِل |
| [docs/adr/](docs/adr/) | سجل القرارات المعمارية |
| [docs/DEPENDENCIES.md](docs/DEPENDENCIES.md) | سجل تراخيص المكتبات |
| [CONTRIBUTING.md](CONTRIBUTING.md) | كيف تساهم، وقاعدة الغرفة النظيفة |
| [AGENTS.md](AGENTS.md) | قواعد العمل لوكلاء الذكاء الاصطناعي الذين يبنون المشروع |
| [SECURITY.md](SECURITY.md) | كيف تبلّغ عن ثغرة أمنية |

## الرخصة

زاجِل برنامج حر بترخيص [GPL-3.0-or-later](LICENSE).

## تنبيه

زاجِل مشروع مستقل، ولا علاقة له بشركة Tonec ولا ببرنامج Internet Download Manager (IDM). كُتب من الصفر، اعتمادًا على المعايير المنشورة ووثائق المتصفحات الرسمية فقط. التفاصيل في [قاعدة الغرفة النظيفة](CONTRIBUTING.md#قاعدة-الغرفة-النظيفة).

</div>

---

## English summary

**Zajil Download Manager** is a free, open-source (GPL-3.0-or-later) download manager for **Windows, macOS and Linux**, designed **Arabic-first (RTL)** with one-click English.

**Status: under construction.** The project is in phase **M0 (foundation)**. The repository holds docs, decisions and CI only; there is no runnable program yet.

**Planned for 1.0:**
- Segmented parallel downloads (up to 32 connections) with dynamic re-segmentation.
- Safe resume using `Range`, `If-Range` and `ETag`.
- Queues, a scheduler and speed limits.
- Automatic categories.
- A Manifest V3 browser extension for Chrome, Edge and Firefox, connected through Native Messaging.
- Video download that never bypasses DRM, and a site grabber.
- Signed installers and signed auto-updates.

No telemetry, no ads, no bundled software.

Zajil is an independent clean-room project. It is not affiliated with Tonec or Internet Download Manager (IDM).

See [docs/ROADMAP.md](docs/ROADMAP.md) and [CONTRIBUTING.md](CONTRIBUTING.md).
