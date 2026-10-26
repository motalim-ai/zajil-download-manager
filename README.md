<div dir="rtl">

# زاجِل لإدارة التحميل — Zajil Download Manager

[![CI](https://github.com/motalim-ai/zajil-download-manager/actions/workflows/ci.yml/badge.svg)](https://github.com/motalim-ai/zajil-download-manager/actions/workflows/ci.yml)
[![License: GPL-3.0](https://img.shields.io/badge/License-GPL--3.0-blue.svg)](LICENSE)

> **الحالة:** [زاجِل 1.0](https://github.com/motalim-ai/zajil-download-manager/releases) أول إصدار كامل. والتحديث التلقائي الموقّع يأتي في 1.1، وما بعده في [ROADMAP](docs/ROADMAP.md).

**زاجِل** برنامج مجاني ومفتوح المصدر لإدارة التحميل. يعمل على **ويندوز وماك ولينكس**، وواجهته **عربية أولًا** (من اليمين إلى اليسار)، والإنجليزية بضغطة زر.

الاسم مأخوذ من **الحمام الزاجل** الذي كان يوصل الرسائل مهما بعدت المسافة. وهذا ما نريده من البرنامج: أن يوصل ملفك كاملًا وسليمًا، حتى لو انقطع الاتصال في الطريق.

## صور

صور حقيقية من الـ CI (بيانات تجريبية فقط):

| النافذة الرئيسية (فاتح) | النافذة الرئيسية (داكن) |
|---|---|
| ![النافذة الرئيسية بالعربية، الوضع الفاتح](docs/screenshots/main-ar-light.png) | ![النافذة الرئيسية بالعربية، الوضع الداكن](docs/screenshots/main-ar-dark.png) |
| **نافذة التقدم بالأجزاء** | **معلومات ملف التحميل** |
| ![نافذة التقدم: الأجزاء والاتصالات الثمانية](docs/screenshots/progress-ar.png) | ![نافذة معلومات ملف التحميل](docs/screenshots/file-info-ar.png) |
| **نافذة «جودة الفيديو»** | **زر «نزّل الفيديو» فوق الفيديو (الإضافة)** |
| ![نافذة جودة الفيديو: الجودات والحجم واسم الملف](docs/screenshots/video-quality-ar.png) | ![زر نزّل الفيديو فوق فيديو تجريبي في المتصفح](docs/screenshots/video-button-ar.png) |

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

لتحميل الفيديو، ينزّل زاجِل بموافقتك ffmpeg وffprobe من بنائه الخاص لمشروع FFmpeg. FFmpeg ملك أصحابه ([ffmpeg.org](https://ffmpeg.org))، وترخيصه LGPL 2.1 أو أحدث، ومصدر النسخة التي ينزّلها زاجِل وطريقة بنائها في [إصدار tools-ffmpeg-8.1.3-1](https://github.com/motalim-ai/zajil-download-manager/releases/tag/tools-ffmpeg-8.1.3-1).

This software uses code of [FFmpeg](https://ffmpeg.org) licensed under the [LGPLv2.1](https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html) and its source can be downloaded [here](https://github.com/motalim-ai/zajil-download-manager/releases/tag/tools-ffmpeg-8.1.3-1).

## تنبيه

زاجِل مشروع مستقل، ولا علاقة له بشركة Tonec ولا ببرنامج Internet Download Manager (IDM). كُتب من الصفر، اعتمادًا على المعايير المنشورة ووثائق المتصفحات الرسمية فقط. التفاصيل في [قاعدة الغرفة النظيفة](CONTRIBUTING.md#قاعدة-الغرفة-النظيفة).

</div>

---

## English summary

**Zajil Download Manager** is a free, open-source (GPL-3.0-or-later) download manager for **Windows, macOS and Linux**, designed **Arabic-first (RTL)** with one-click English.

**Status:** [Zajil 1.0](https://github.com/motalim-ai/zajil-download-manager/releases) is the first full release. Signed automatic updates come in 1.1; what comes next is in [docs/ROADMAP.md](docs/ROADMAP.md).

## Screenshots

Real screenshots from CI (test data only):

| Main window (light) | Main window (dark) |
|---|---|
| ![The main window in Arabic, light](docs/screenshots/main-ar-light.png) | ![The main window in Arabic, dark](docs/screenshots/main-ar-dark.png) |
| **The progress window with its parts** | **Download file info** |
| ![The progress window: its parts and eight connections](docs/screenshots/progress-ar.png) | ![The Download file info window](docs/screenshots/file-info-ar.png) |
| **The «Video quality» window** | **The «Download video» button over a video (the extension)** |
| ![The Video quality window: the formats, their size and the file name](docs/screenshots/video-quality-ar.png) | ![The Download video button over a test video in the browser](docs/screenshots/video-button-ar.png) |

**Planned for 1.0:**
- Segmented parallel downloads (up to 32 connections) with dynamic re-segmentation.
- Safe resume using `Range`, `If-Range` and `ETag`.
- Queues, a scheduler and speed limits.
- Automatic categories.
- A Manifest V3 browser extension for Chrome, Edge and Firefox, connected through Native Messaging.
- Video download that never bypasses DRM, and a site grabber.
- Installers checked by published SHA-256 sums (never signed by Apple or Microsoft; macOS ad-hoc only), and auto-updates signed with the project's own key.

No telemetry, no ads, no bundled software.

Zajil is an independent clean-room project. It is not affiliated with Tonec or Internet Download Manager (IDM).

See [docs/ROADMAP.md](docs/ROADMAP.md) and [CONTRIBUTING.md](CONTRIBUTING.md).
