<script lang="ts">
  // ui-test builds only (loaded behind __UI_TEST__; CI checks that a
  // release build has no `data-zajil-ui-test`). The image test of mixed
  // names (owner's decision on the M4 plan): for each name, the real
  // component, a reference built from pieces of one direction each placed
  // in an explicit visual order, and the wrong renderings, which must look
  // different. scripts/compare-names.mjs compares them in the screenshot of
  // the real window.
  import { onMount, tick } from "svelte";
  import FileName from "../lib/components/FileName.svelte";
  import { api, windowRect } from "../lib/api";

  type Piece = { dir: "ltr" | "rtl"; text: string };
  type Case = { name: string; reference: Piece[]; naiveDiffers: boolean; extensionOnlyDiffers: boolean };

  // The expected look, written by hand from left to right on screen.
  const cases: Case[] = [
    {
      name: "محاضرة 12 - الخوارزميات (Part 2).mp4",
      reference: [{ dir: "ltr", text: "(Part 2).mp4" }, { dir: "rtl", text: "محاضرة 12 - الخوارزميات " }],
      naiveDiffers: true,
      extensionOnlyDiffers: true,
    },
    {
      name: "تقرير المبيعات Q3-2026 النهائي.pdf",
      reference: [{ dir: "ltr", text: "pdf" }, { dir: "rtl", text: "تقرير المبيعات Q3-2026 النهائي." }],
      naiveDiffers: false,
      extensionOnlyDiffers: true,
    },
    {
      name: "أغاني فيروز - Live 1971.flac",
      reference: [{ dir: "ltr", text: "Live 1971.flac" }, { dir: "rtl", text: "أغاني فيروز - " }],
      naiveDiffers: false,
      extensionOnlyDiffers: true,
    },
    {
      name: "صور الرحلة 2025 (الجزء 3).zip",
      reference: [{ dir: "ltr", text: "zip" }, { dir: "rtl", text: "صور الرحلة 2025 (الجزء 3)." }],
      naiveDiffers: false,
      extensionOnlyDiffers: true,
    },
    {
      name: "Blender 4.5 تعليم النمذجة.mkv",
      reference: [
        { dir: "ltr", text: "Blender 4.5 " },
        { dir: "rtl", text: "تعليم النمذجة" },
        { dir: "ltr", text: ".mkv" },
      ],
      naiveDiffers: true,
      extensionOnlyDiffers: false,
    },
    {
      name: "backup_2026-10-20 النسخة الكاملة.tar.gz",
      reference: [
        { dir: "ltr", text: "backup_2026-10-20 " },
        { dir: "rtl", text: "النسخة الكاملة" },
        { dir: "ltr", text: ".tar.gz" },
      ],
      naiveDiffers: true,
      extensionOnlyDiffers: false,
    },
  ];

  // The rendering the owner rejected: the extension isolated on its own.
  const extensionOnly = (name: string) => {
    const m = /^(.*?)((?:\.tar)?\.[A-Za-z0-9]{1,8})$/.exec(name);
    return { stem: m?.[1] ?? name, ext: m?.[2] ?? "" };
  };

  const rect = (el: Element) => {
    const r = el.getBoundingClientRect();
    return [r.x, r.y, r.width, r.height];
  };

  onMount(async () => {
    await tick();
    await document.fonts.ready;
    await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
    const rows = cases.map((c, i) => ({
      name: c.name,
      naiveDiffers: c.naiveDiffers,
      extensionOnlyDiffers: c.extensionOnlyDiffers,
      real: rect(document.getElementById(`real-${i}`)!),
      reference: rect(document.getElementById(`ref-${i}`)!),
      naive: rect(document.getElementById(`naive-${i}`)!),
      extensionOnly: rect(document.getElementById(`ext-${i}`)!),
    }));
    await api.uiTestReport({
      scenario: "names",
      dpr: window.devicePixelRatio,
      window: await windowRect(),
      markers: { a: rect(document.getElementById("marker-a")!), b: rect(document.getElementById("marker-b")!) },
      rows,
    });
  });
</script>

<div class="page" data-zajil-ui-test>
  <i id="marker-a" class="marker" style:inset-inline-start="8px"></i>
  <i id="marker-b" class="marker" style:inset-inline-start="808px"></i>
  {#each cases as c, i (c.name)}
    <div class="group">
      <div class="box" dir="rtl" id="real-{i}"><FileName name={c.name} /></div>
      <div class="box reference" dir="ltr" id="ref-{i}">
        {#each c.reference as piece, p (p)}<span dir={piece.dir}>{piece.text}</span>{/each}
      </div>
      <div class="box" dir="rtl" id="naive-{i}"><span>{c.name}</span></div>
      <div class="box" dir="rtl" id="ext-{i}">
        <bdi dir="auto">{extensionOnly(c.name).stem}<bdi dir="ltr">{extensionOnly(c.name).ext}</bdi></bdi>
      </div>
    </div>
  {/each}
</div>

<style>
  .page {
    position: relative;
    direction: ltr;
    padding: 28px 8px 8px;
    background: #ffffff;
    color: #000000;
    height: 100%;
    overflow: hidden;
    font: 400 22px/1 "IBM Plex Sans Arabic", system-ui, sans-serif;
  }
  .marker {
    position: absolute;
    top: 8px;
    width: 10px;
    height: 10px;
    background: #ff00ff;
  }
  .group {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 4px 16px;
    margin-bottom: 10px;
  }
  /* Two columns of 470 px fit the 1024 px screens of the macOS and
     Windows CI machines (560 px cut the second column off there). The
     longest name is about 390 px wide. */
  .box {
    height: 34px;
    width: 470px;
    display: flex;
    align-items: center;
    justify-content: flex-start;
    white-space: pre;
    overflow: hidden;
  }
  .box :global(.fn) {
    overflow: visible;
  }
  .reference {
    justify-content: flex-end;
  }
  .reference span {
    unicode-bidi: isolate;
    white-space: pre;
  }
</style>
