<script lang="ts">
  import { onMount, tick } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import Icon from "../lib/components/Icon.svelte";
  import { api } from "../lib/api";
  import { locale } from "../lib/i18n.svelte";
  import { size } from "../lib/format";
  import { startWindow } from "../lib/windowStart";
  import { fileName, options, picked, type Option } from "../lib/videoChoices";
  import type { ToolView } from "../lib/bindings/ToolView";
  import type { VideoInfoView } from "../lib/bindings/VideoInfoView";
  import type { VideoErrorView } from "../lib/bindings/VideoErrorView";

  // «Video quality» (M6, #149, ADR-0009): the tools asked for once (nothing
  // is downloaded before «OK»), then the page's formats or its playlist,
  // the file's name and folder, and the download with its progress.
  type Step =
    | { kind: "link" }
    | { kind: "consent"; tools: ToolView[] }
    | { kind: "installing"; tool: string; done: number; total: number | null }
    | { kind: "probing" }
    | { kind: "video"; info: Extract<VideoInfoView, { kind: "video" }>; list: Option[] }
    | { kind: "playlist"; info: Extract<VideoInfoView, { kind: "playlist" }>; chosen: boolean[] }
    | { kind: "downloading"; label: string; done: number; total: number | null; speed: number | null; id: number | null }
    | { kind: "done"; file: string }
    | { kind: "failed"; error: VideoErrorView | string };

  // The UI test scenes («tools», «quality») start at a step, with demo
  // data, and ask Rust nothing.
  let { scene = null }: { scene?: "tools" | "quality" | null } = $props();
  let step = $state<Step>({ kind: "probing" });
  let url = $state("");
  let typed = $state("");
  let folder = $state("");
  let name = $state("");
  let pick = $state(0);
  let unlisten: (() => void)[] = [];

  onMount(() => {
    if (scene) {
      showScene(scene);
      return;
    }
    void (async () => {
      await startWindow();
      void getCurrentWindow().setTitle(locale.t("video-title")).catch(() => {});
      folder = await api.videoFolder().catch(() => "");
      unlisten.push(await api.onVideoToolsProgress((p) => step.kind === "installing" && (step = { kind: "installing", tool: p.tool, done: p.done, total: p.total })));
      unlisten.push(await api.onVideoProgress((p) => step.kind === "downloading" && p.id === step.id && (step = { ...step, done: p.done, total: p.total, speed: p.speed })));
      const pending = await api.videoPending().catch(() => null);
      if (pending) {
        url = pending;
        await begin();
      } else step = { kind: "link" };
    })();
    return () => unlisten.forEach((u) => u());
  });

  function showScene(which: "tools" | "quality") {
    folder = "~/Downloads";
    if (which === "tools") {
      step = {
        kind: "consent",
        tools: [
          { tool: "yt-dlp", installed: false, version: "2026.10.10", host: "github.com", license: "GPLv3+" },
          { tool: "ffmpeg", installed: false, version: "7.1", host: "github.com", license: "LGPL 2.1+" },
          { tool: "ffprobe", installed: false, version: "7.1", host: "github.com", license: "LGPL 2.1+" },
          { tool: "qjs", installed: false, version: "0.10.1", host: "github.com", license: "MIT" },
        ],
      };
      return;
    }
    const MIB = 1024 * 1024;
    const info = {
      kind: "video" as const,
      title: "دورة Rust للمبتدئين (Part 2)",
      duration: 1830,
      best: { kind: "merge" as const, video: "137", audio: "140" },
      formats: [
        { id: "137", ext: "mp4", width: 1920, height: 1080, fps: 30, size: 412 * MIB, kind: "video", hls: false },
        { id: "136", ext: "mp4", width: 1280, height: 720, fps: 30, size: 218 * MIB, kind: "video", hls: false },
        { id: "18", ext: "mp4", width: 640, height: 360, fps: 30, size: 64 * MIB, kind: "both", hls: false },
        { id: "140", ext: "m4a", width: null, height: null, fps: null, size: 28 * MIB, kind: "audio", hls: false },
      ],
    };
    const list = options(info.formats);
    pick = picked(list, info.best);
    name = fileName(info.title, list[pick]);
    step = { kind: "video", info, list };
  }

  /** The tools first (asking once), then the page. */
  async function begin() {
    const tools = await api.videoToolsStatus().catch(() => []);
    if (tools.some((t) => !t.installed)) {
      step = { kind: "consent", tools };
      return;
    }
    await probe();
  }

  async function install() {
    step = { kind: "installing", tool: "", done: 0, total: null };
    try {
      await api.videoToolsInstall();
      await probe();
    } catch (e) {
      step = { kind: "failed", error: String(e) };
    }
  }

  async function probe() {
    step = { kind: "probing" };
    try {
      const info = await api.videoProbe(url);
      if (info.kind === "video") {
        const list = options(info.formats);
        pick = picked(list, info.best);
        name = fileName(info.title, list[pick]);
        step = { kind: "video", info, list };
      } else step = { kind: "playlist", info, chosen: info.entries.map(() => true) };
      await tick();
      document.querySelector<HTMLElement>("[data-first]")?.focus();
    } catch (e) {
      step = { kind: "failed", error: e as VideoErrorView };
    }
  }

  async function download(choice: Option["choice"], file: string, label: string): Promise<boolean> {
    step = { kind: "downloading", label, done: 0, total: null, speed: null, id: null };
    const id = await api.videoDownload(url, choice, folder, file).catch((e) => {
      step = { kind: "failed", error: String(e) };
      return null;
    });
    if (id === null) return false;
    if (step.kind === "downloading") step = { ...step, id };
    return await new Promise<boolean>((done) => {
      const offs: (() => void)[] = [];
      void api.onVideoFinished((f) => {
        if (f.id !== id) return;
        offs.forEach((o) => o());
        step = { kind: "done", file: f.file };
        done(true);
      }).then((o) => offs.push(o));
      void api.onVideoFailed((f) => {
        if (f.id !== id) return;
        offs.forEach((o) => o());
        step = { kind: "failed", error: f.error };
        done(false);
      }).then((o) => offs.push(o));
    });
  }

  /** A playlist: its chosen items one after another, each at its best. */
  async function downloadList(info: Extract<VideoInfoView, { kind: "playlist" }>, chosen: boolean[]) {
    const items = info.entries.filter((e, i) => chosen[i] && e.url);
    for (const [i, entry] of items.entries()) {
      url = entry.url!;
      const probed = await api.videoProbe(url).catch((e) => e as VideoErrorView);
      if ("message" in probed || probed.kind !== "video") continue;
      const list = options(probed.formats);
      const option = list[picked(list, probed.best)];
      if (!option) continue;
      const ok = await download(option.choice, fileName(probed.title, option), `${i + 1}/${items.length}: ${probed.title}`);
      if (!ok) return;
    }
  }

  async function chooseFolder() {
    const chosen = await openDialog({ directory: true, multiple: false }).catch(() => null);
    if (typeof chosen === "string") folder = chosen;
  }

  async function close() {
    if (step.kind === "installing") await api.videoToolsCancel().catch(() => {});
    if (step.kind === "probing") await api.videoProbeCancel().catch(() => {});
    if (step.kind === "downloading" && step.id !== null) await api.videoCancel(step.id).catch(() => {});
    await api.closeWindow();
  }

  const quality = (o: Option) =>
    o.choice.kind === "audio_only" ? locale.t("video-audio-only", { format: "m4a" }) : `${o.height}p${o.fps && o.fps > 30 ? ` ${Math.round(o.fps)}fps` : ""} · ${o.ext.toUpperCase()}`;
  const errorText = (e: VideoErrorView | string) =>
    typeof e === "string" ? e : e.kind === "drm" ? locale.t("video-drm") : e.kind === "tools_missing" ? locale.t("video-tools-missing") : e.message;
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && void close()} />

<main class="video" aria-labelledby="video-title">
  <h1 id="video-title" class="visually-hidden">{locale.t("video-title")}</h1>

  {#if step.kind === "link"}
    <form class="col" onsubmit={(e) => (e.preventDefault(), typed.trim() && ((url = typed.trim()), void begin()))}>
      <label class="label" for="video-url">{locale.t("video-page")}</label>
      <input id="video-url" class="box" dir="ltr" autocomplete="off" spellcheck="false" bind:value={typed} data-first />
      <div class="actions"><button class="btn primary" disabled={!typed.trim()}>{locale.t("addlink-ok")}</button><button type="button" class="btn" onclick={() => void close()}>{locale.t("add-cancel")}</button></div>
    </form>
  {:else if step.kind === "consent"}
    <section class="col" aria-labelledby="consent-title">
      <h2 id="consent-title">{locale.t("video-consent-title")}</h2>
      <p class="muted">{locale.t("video-consent-why")}</p>
      <table class="tools">
        <thead><tr><th scope="col">{locale.t("video-tool")}</th><th scope="col">{locale.t("video-version")}</th><th scope="col">{locale.t("video-source")}</th><th scope="col">{locale.t("video-license")}</th></tr></thead>
        <tbody>
          {#each step.tools.filter((t) => !t.installed) as t (t.tool)}
            <tr><td dir="ltr">{t.tool}</td><td dir="ltr">{t.version ?? "—"}</td><td dir="ltr">{t.host ?? "—"}</td><td dir="ltr">{t.license}</td></tr>
          {/each}
        </tbody>
      </table>
      <p class="muted">{locale.t("video-consent-note")}</p>
      <div class="actions">
        <button class="btn primary" onclick={() => void install()} data-first><Icon name="download" size={15} />{locale.t("video-consent-ok")}</button>
        <button class="btn" onclick={() => void close()}>{locale.t("add-cancel")}</button>
      </div>
    </section>
  {:else if step.kind === "installing"}
    <section class="col" aria-live="polite">
      <h2>{locale.t("video-installing")}</h2>
      <p class="muted" dir="ltr">{step.tool}</p>
      <div class="bar" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={step.total ? Math.round((step.done / step.total) * 100) : 0}>
        <b style:inline-size="{step.total ? (step.done / step.total) * 100 : 5}%"></b>
      </div>
      <div class="actions"><button class="btn" onclick={() => void close()}>{locale.t("add-cancel")}</button></div>
    </section>
  {:else if step.kind === "probing"}
    <p class="waiting" role="status">{locale.t("video-probing")}</p>
  {:else if step.kind === "video"}
    {@const info = step.info}
    <section class="col">
      <h2 class="title" dir="auto">{info.title}</h2>
      <fieldset class="qualities">
        <legend>{locale.t("video-quality")}</legend>
        {#each step.list as o, i (i)}
          <label class="q" class:on={pick === i}>
            <input type="radio" name="quality" checked={pick === i} onchange={() => ((pick = i), (name = fileName(info.title, o)))} data-first={i === 0 ? true : undefined} />
            <span>{quality(o)}</span>
            <span class="muted num">{o.size === null ? locale.t("add-size-unknown") : size(locale.lang, locale.digits, o.size)}</span>
          </label>
        {/each}
      </fieldset>
      <div class="grid">
        <label class="label" for="video-name">{locale.t("add-file-name")}</label>
        <input id="video-name" class="box" dir="auto" bind:value={name} />
        <span class="label">{locale.t("add-folder")}</span>
        <div class="with"><bdi class="box path" dir="ltr">{folder}</bdi><button class="btn" onclick={() => void chooseFolder()}>{locale.t("add-change")}</button></div>
      </div>
      <div class="actions">
        <button class="btn primary" disabled={!name.trim() || !folder} onclick={() => void download(step.kind === "video" ? step.list[pick]!.choice : { kind: "audio_only" }, name.trim(), name.trim())}>
          <Icon name="download" size={15} />{locale.t("add-start")}
        </button>
        <button class="btn" onclick={() => void close()}>{locale.t("add-cancel")}</button>
      </div>
    </section>
  {:else if step.kind === "playlist"}
    {@const list = step}
    <section class="col">
      <h2 class="title" dir="auto">{list.info.title}</h2>
      <p class="muted">{locale.t("video-playlist", { count: list.info.entries.length })}</p>
      <ul class="entries">
        {#each list.info.entries as e, i (i)}
          <li><label><input type="checkbox" bind:checked={list.chosen[i]} data-first={i === 0 ? true : undefined} /><bdi dir="auto">{e.title ?? e.url ?? "—"}</bdi></label></li>
        {/each}
      </ul>
      <div class="with"><bdi class="box path" dir="ltr">{folder}</bdi><button class="btn" onclick={() => void chooseFolder()}>{locale.t("add-change")}</button></div>
      <div class="actions">
        <button class="btn primary" disabled={!list.chosen.some(Boolean) || !folder} onclick={() => void downloadList(list.info, list.chosen)}><Icon name="download" size={15} />{locale.t("video-download-chosen")}</button>
        <button class="btn" onclick={() => void close()}>{locale.t("add-cancel")}</button>
      </div>
    </section>
  {:else if step.kind === "downloading"}
    <section class="col" aria-live="polite">
      <h2 class="title" dir="auto">{step.label}</h2>
      <div class="bar" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={step.total ? Math.round((step.done / step.total) * 100) : 0}>
        <b style:inline-size="{step.total ? (step.done / step.total) * 100 : 5}%"></b>
      </div>
      <p class="muted num">{size(locale.lang, locale.digits, step.done)}{step.total ? ` / ${size(locale.lang, locale.digits, step.total)}` : ""}</p>
      <div class="actions"><button class="btn" onclick={() => void close()}>{locale.t("add-cancel")}</button></div>
    </section>
  {:else if step.kind === "done"}
    <section class="col" aria-live="polite">
      <h2><Icon name="circle-check" size={18} />{locale.t("video-done")}</h2>
      <p class="muted" dir="ltr">{step.file}</p>
      <div class="actions"><button class="btn primary" onclick={() => void api.closeWindow()} data-first>{locale.t("progress-close")}</button></div>
    </section>
  {:else}
    <section class="col">
      <p class="problem" role="alert"><Icon name="circle-alert" size={16} />{errorText(step.error)}</p>
      <div class="actions"><button class="btn" onclick={() => void close()} data-first>{locale.t("progress-close")}</button></div>
    </section>
  {/if}
</main>

<style>
  .video {
    height: 100%;
    box-sizing: border-box;
    padding: 18px 22px;
    background: var(--surface);
    overflow: auto;
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  h2 {
    margin: 0;
    font-size: 15px;
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .label {
    font-size: 13px;
    color: var(--muted);
  }
  .box {
    border: 1px solid var(--line);
    background: var(--sunken);
    border-radius: 8px;
    padding: 6px 10px;
    min-height: 34px;
    box-sizing: border-box;
    font: inherit;
    color: var(--ink);
    min-width: 0;
  }
  .path {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .grid {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 10px 16px;
    align-items: center;
  }
  .with {
    display: flex;
    gap: 8px;
    min-width: 0;
  }
  .qualities {
    border: 1px solid var(--line);
    border-radius: 10px;
    padding: 6px 10px;
    margin: 0;
    max-height: 210px;
    overflow: auto;
  }
  legend {
    font-size: 12.5px;
    color: var(--muted);
    padding: 0 6px;
  }
  .q {
    display: grid;
    grid-template-columns: auto 1fr auto;
    gap: 10px;
    align-items: center;
    padding: 5px 6px;
    border-radius: 6px;
    font-size: 13.5px;
  }
  .q.on {
    background: var(--sel);
  }
  .muted {
    color: var(--muted);
    font-size: 12.5px;
    margin: 0;
  }
  .tools {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  .tools th {
    text-align: start;
    color: var(--muted);
    font-weight: 600;
    padding: 6px 8px;
    border-bottom: 1px solid var(--line);
  }
  .tools td {
    padding: 6px 8px;
    border-bottom: 1px solid var(--line);
  }
  .entries {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 240px;
    overflow: auto;
    border: 1px solid var(--line);
    border-radius: 8px;
  }
  .entries li label {
    display: flex;
    gap: 8px;
    padding: 5px 10px;
    font-size: 13px;
  }
  .bar {
    height: 10px;
    border-radius: 6px;
    background: var(--sunken);
    overflow: hidden;
  }
  .bar b {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .actions {
    display: flex;
    gap: 8px;
    margin-top: 4px;
  }
  .waiting {
    color: var(--muted);
  }
  .problem {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    color: var(--red);
    margin: 0;
  }
</style>
