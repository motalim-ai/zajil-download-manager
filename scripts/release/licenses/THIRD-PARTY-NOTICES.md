# زاجِل: التراخيص — Zajil: licenses

<div dir="rtl">

## زاجِل

زاجِل برنامج حر بترخيص GPL-3.0-or-later (ملف `LICENSE` بجانب هذا الملف). ولا يطلب الترخيص أي موافقة، ولا يمنع الهندسة العكسية.

## الأدوات التي ينزّلها زاجِل لتحميل الفيديو

لا تأتي هذه الأدوات مع المثبّت. ينزّلها زاجِل عند أول تحميل فيديو، بعد موافقتك، ويتحقق من بصمة كل ملف قبل كل تشغيل.

### FFmpeg (ffmpeg وffprobe)

يستعمل هذا البرنامج شيفرة من مشروع FFmpeg بترخيص LGPL 2.1 أو أحدث (نصه في `LGPL-2.1.txt` بجانب هذا الملف)، ويمكن تنزيل مصدرها من [إصدار tools-ffmpeg-8.1.3-1](https://github.com/motalim-ai/zajil-download-manager/releases/tag/tools-ffmpeg-8.1.3-1)، ومعه طريقة البناء كاملة.

- النسخة: FFmpeg 8.1.3، من بناء زاجِل (`8.1.3-1`)، من المصدر الرسمي دون أي تعديل، وبلا `--enable-gpl` ولا `--enable-nonfree` ولا أي مكتبة خارجية.
- FFmpeg ملك أصحابه، وليس ملكًا لزاجِل: [ffmpeg.org](https://ffmpeg.org).

### yt-dlp

الملف التنفيذي الذي ينزّله زاجِل بترخيص GPLv3 أو أحدث (والكود المصدري بترخيص Unlicense). وهو ملك أصحابه: [github.com/yt-dlp/yt-dlp](https://github.com/yt-dlp/yt-dlp).

### QuickJS-NG

بترخيص MIT، لتحميل فيديو يوتيوب فقط إن لم يوجد Deno أو Node، ويعمل في صندوق مغلق. وهو ملك أصحابه: [github.com/quickjs-ng/quickjs](https://github.com/quickjs-ng/quickjs).

## المكتبات داخل البرنامج

تراخيص المكتبات المبنية داخل زاجِل (Rust وnpm)، وسبب اختيار كل منها، في [docs/DEPENDENCIES.md](https://github.com/motalim-ai/zajil-download-manager/blob/main/docs/DEPENDENCIES.md).

</div>

## Zajil

Zajil is free software under GPL-3.0-or-later (the `LICENSE` file next to this one). The licence asks for no acceptance and does not forbid reverse engineering.

## The tools Zajil downloads for video

These tools are not in the installer. Zajil downloads them at the first video download, after you agree, and checks each file's fingerprint before every run.

### FFmpeg (ffmpeg and ffprobe)

This software uses code of FFmpeg licensed under the LGPLv2.1 (or later; the text is in `LGPL-2.1.txt` next to this file) and its source can be downloaded [here](https://github.com/motalim-ai/zajil-download-manager/releases/tag/tools-ffmpeg-8.1.3-1), with the whole way it was built.

- Version: FFmpeg 8.1.3, Zajil's build `8.1.3-1`, from the official source unchanged, without `--enable-gpl`, `--enable-nonfree` or any outside library.
- FFmpeg belongs to its authors, not to Zajil: [ffmpeg.org](https://ffmpeg.org).

### yt-dlp

The executable Zajil downloads is under GPLv3 or later (its source code is under the Unlicense). It belongs to its authors: [github.com/yt-dlp/yt-dlp](https://github.com/yt-dlp/yt-dlp).

### QuickJS-NG

MIT. Used only for YouTube when neither Deno nor Node is installed, in a closed sandbox. It belongs to its authors: [github.com/quickjs-ng/quickjs](https://github.com/quickjs-ng/quickjs).

## The libraries inside the program

The licences of the libraries built into Zajil (Rust and npm), and why each was chosen, are in [docs/DEPENDENCIES.md](https://github.com/motalim-ai/zajil-download-manager/blob/main/docs/DEPENDENCIES.md).
