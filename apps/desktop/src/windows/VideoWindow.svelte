<script lang="ts">
  import { onMount, tick } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import Icon from "../lib/components/Icon.svelte";
  import ColorIcon from "../lib/components/ColorIcon.svelte";
  import FileName from "../lib/components/FileName.svelte";
  import { nameDir } from "../lib/fname";
  import { licenseOf, toolName, versionOf } from "../lib/about";
  import { api } from "../lib/api";
  import { locale } from "../lib/i18n.svelte";
  import { size } from "../lib/format";
  import { startWindow } from "../lib/windowStart";
  import { fileName, options, picked, type Option } from "../lib/videoChoices";
  import { errorDetails, errorKey } from "../lib/videoErrors";
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
    // A format chosen in the browser: it downloads already, while the
    // user says where it goes (owner, 2026-10-29, as IDM).
    | { kind: "started"; id: number; label: string | null; size: number | null; status: string; error: string | null }
    | { kind: "failed"; error: VideoErrorView | string };

  // The UI test scenes («tools», «quality») start at a step, with demo
  // data; «tools» asks Rust for the tools' real pins only.
  let { scene = null }: { scene?: "tools" | "quality" | null } = $props();
  let step = $state<Step>({ kind: "probing" });
  let url = $state("");
  let typed = $state("");
  let folder = $state("");
  let name = $state("");
  // The name box is being edited: until then the name is drawn as
  // FileName draws it (as in «Download file info»).
  let editing = $state(false);
  let pick = $state(0);
  // The name and folder the user changed (else they follow the download's).
  let nameTouched = false;
  let folderTouched = false;
  let unlisten: (() => void)[] = [];
  // Nothing is drawn before the window knows what it shows: a download the
  // browser started, or a page to ask about (the owner's notes: no view
  // that is replaced by another as soon as it shows).
  let decided = $state(false);
  // The browser's download that waits for the tools: it starts again
  // once they are in, and goes if the user cancels instead.
  let afterTools: number | null = null;

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
      unlisten.push(
        await api.onVideoProgress((p) => {
          if (step.kind === "downloading" && p.id === step.id) step = { ...step, done: p.done, total: p.total, speed: p.speed };
        }),
      );
      unlisten.push(await api.onChange(() => step.kind === "started" && void showStarted()));
      // Not while the tools come: what is asked about then waits its turn.
      unlisten.push(await api.onVideoStarted(() => step.kind !== "installing" && void showStarted(true)));
      if (await showStarted(true)) return;
      decided = true;
      const pending = await api.videoPending().catch(() => null);
      if (pending) {
        url = pending;
        await begin();
      } else step = { kind: "link" };
    })();
    return () => unlisten.forEach((u) => u());
  });

  /** The download the browser's list started, if any: its name once the
   * page is read, its folder, and how far it is. `fresh`: a new one. */
  async function showStarted(fresh = false): Promise<boolean> {
    const s = await api.videoStarted().catch(() => null);
    if (!s) return false;
    // Without the tools it cannot go on (it failed, or it will): they are
    // asked for here, as for a page, not left to a message (owner,
    // 2026-10-29: 1.1.0 said only «open the window again»).
    if (fresh || s.status === "failed") {
      const tools = await api.videoToolsStatus().catch(() => []);
      if (tools.some((t) => !t.installed)) {
        afterTools = s.id;
        step = { kind: "consent", tools };
        decided = true;
        return true;
      }
    }
    if (fresh || step.kind !== "started" || step.id !== s.id) {
      nameTouched = false;
      folderTouched = false;
      step = { kind: "started", id: s.id, label: s.label, size: s.size, status: s.status, error: s.error };
      decided = true;
      await tick();
      document.querySelector<HTMLElement>("[data-first]")?.focus();
    } else step = { ...step, label: s.label ?? step.label, size: step.size ?? s.size, status: s.status, error: s.error };
    if (!nameTouched && !editing && s.name) name = s.name;
    if (!folderTouched) folder = s.folder;
    return true;
  }

  /** «Start download»: it goes on, where the user said; its progress
   * window takes over, already showing what came. «Download later»: it
   * waits paused in the list. */
  async function keep(id: number, now: boolean) {
    try {
      if (now) {
        await api.videoKeep(id, folder, name.trim());
        await api.openProgress(id, name.trim() || "Zajil").catch(() => {});
      } else await api.videoLater(id, folder, name.trim());
      await api.closeWindow();
    } catch (e) {
      step = { kind: "failed", error: String(e) };
    }
  }

  function showScene(which: "tools" | "quality") {
    folder = "~/Downloads";
    if (which === "tools") {
      // The real pins (the pictures may go on a release page): only the
      // tools' status is asked, and they are shown as not downloaded.
      step = { kind: "consent", tools: [] };
      void api.videoToolsStatus().then(
        (tools) => step.kind === "consent" && (step = { kind: "consent", tools: tools.map((t) => ({ ...t, installed: false })) }),
        () => {},
      );
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
    const list = options(info.formats, info.best);
    pick = picked(list, info.best);
    name = fileName(info.title);
    step = { kind: "video", info, list };
  }

  /** The tools (asking once), then the page: asked while they are checked,
   * so a page read ahead is shown without waiting for that check. */
  async function begin() {
    const asked = api.videoProbe(url);
    asked.catch(() => {});
    const tools = await api.videoToolsStatus().catch(() => []);
    if (tools.some((t) => !t.installed)) {
      step = { kind: "consent", tools };
      return;
    }
    await probe(asked);
  }

  async function install() {
    step = { kind: "installing", tool: "", done: 0, total: null };
    try {
      await api.videoToolsInstall();
      if (afterTools !== null) {
        const id = afterTools;
        afterTools = null;
        await api.resume([id]);
        await showStarted(true);
      } else await probe();
    } catch (e) {
      // Why, in the user's words (`not_ready`: yt-dlp did not start after
      // its download), and its own text under «Details».
      step = { kind: "failed", error: typeof e === "string" ? e : (e as VideoErrorView) };
    }
  }

  async function probe(asked?: Promise<VideoInfoView>) {
    step = { kind: "probing" };
    try {
      const info = await (asked ?? api.videoProbe(url));
      if (info.kind === "video") {
        const list = options(info.formats, info.best);
        pick = picked(list, info.best);
        name = fileName(info.title);
        step = { kind: "video", info, list };
      } else step = { kind: "playlist", info, chosen: info.entries.map((e) => !e.playlist && !!e.url) };
      await tick();
      document.querySelector<HTMLElement>("[data-first]")?.focus();
    } catch (e) {
      step = { kind: "failed", error: e as VideoErrorView };
    }
  }

  /** One video joins the list, and its progress window takes over, as a
   * download in IDM does. */
  async function downloadOne(choice: Option["choice"], file: string) {
    step = { kind: "downloading", label: file, done: 0, total: null, speed: null, id: null };
    try {
      const id = await api.videoDownload(url, choice, folder, file);
      await api.openProgress(id, file);
      await api.closeWindow();
    } catch (e) {
      step = { kind: "failed", error: String(e) };
    }
  }

  /** A playlist: its chosen videos join the list at once, each at its
   * best, and the window closes; Zajil asks each page and downloads it in
   * turn, and one that fails says why in the list. */
  async function downloadList(info: Extract<VideoInfoView, { kind: "playlist" }>, chosen: boolean[]) {
    const items = info.entries.filter((e, i) => chosen[i] && e.url && !e.playlist).map((e) => ({ url: e.url!, title: e.title }));
    step = { kind: "downloading", label: info.title, done: 0, total: null, speed: null, id: null };
    try {
      await api.videoAddMany(url, items, folder);
      await api.closeWindow();
    } catch (e) {
      step = { kind: "failed", error: String(e) };
    }
  }

  /** A playlist inside the playlist: its own items, in this window. */
  async function openPlaylist(link: string) {
    url = link;
    await probe();
  }

  async function chooseFolder() {
    const chosen = await openDialog({ directory: true, multiple: false }).catch(() => null);
    if (typeof chosen === "string") {
      folder = chosen;
      folderTouched = true;
    }
  }

  async function close() {
    if (step.kind === "installing") await api.videoToolsCancel().catch(() => {});
    if (step.kind === "probing") await api.videoProbeCancel().catch(() => {});
    if (step.kind === "downloading" && step.id !== null) await api.videoCancel(step.id).catch(() => {});
    if (step.kind === "started") await api.videoDiscard(step.id).catch(() => {});
    else if (afterTools !== null) await api.videoDiscard(afterTools).catch(() => {});
    await api.closeWindow();
  }

  const quality = (o: Option) =>
    o.choice.kind === "audio_only" ? locale.t("video-audio-only", { format: "m4a" }) : `${o.height}p${o.fps && o.fps > 30 ? ` ${Math.round(o.fps)}fps` : ""} · ${o.ext.toUpperCase()}`;
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && void close()} />

<main class="video" aria-labelledby="video-title">
  <h1 id="video-title" class="visually-hidden">{locale.t("video-title")}</h1>

  {#if !decided && !scene}
    <!-- Not known yet: nothing, for a few milliseconds. -->
  {:else if step.kind === "link"}
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
            <tr><td>{toolName(t.tool)}</td><td>{versionOf(t)}</td><td dir="ltr">{t.host ?? "—"}</td><td>{licenseOf((key, args) => locale.t(key, args), t.tool)}</td></tr>
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
      <p class="muted">{toolName(step.tool)}</p>
      <div class="bar" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={step.total ? Math.round((step.done / step.total) * 100) : 0}>
        <b style:inline-size="{step.total ? (step.done / step.total) * 100 : 5}%"></b>
      </div>
      <div class="actions"><button class="btn" onclick={() => void close()}>{locale.t("add-cancel")}</button></div>
    </section>
  {:else if step.kind === "probing"}
    <!-- It can take a while: «Cancel» stops it (the owner's notes on 1.0). -->
    <section class="col">
      <p class="waiting" role="status">{locale.t("video-probing")}</p>
      <div class="actions"><button class="btn" onclick={() => void close()} data-first>{locale.t("add-cancel")}</button></div>
    </section>
  {:else if step.kind === "video"}
    {@const info = step.info}
    <section class="col">
      <h2 class="title" dir="auto">{info.title}</h2>
      <fieldset class="qualities">
        <legend>{locale.t("video-quality")}</legend>
        {#each step.list as o, i (i)}
          <label class="q" class:on={pick === i}>
            <input type="radio" name="quality" checked={pick === i} onchange={() => (pick = i)} data-first={i === 0 ? true : undefined} />
            <span>{quality(o)}</span>
            <span class="muted num">{o.size === null ? locale.t("add-size-unknown") : size(locale.lang, locale.digits, o.size)}</span>
          </label>
        {/each}
      </fieldset>
      <div class="grid">
        <label class="label" for="video-name">{locale.t("add-file-name")}</label>
        <div class="namebox">
          <input
            id="video-name"
            class="box"
            class:quiet={!editing && name !== ""}
            dir="auto"
            bind:value={name}
            onfocus={() => (editing = true)}
            onblur={() => (editing = false)}
          />
          {#if !editing && name !== ""}<span class="box shown" dir={nameDir(name)} aria-hidden="true"><FileName {name} /></span>{/if}
        </div>
        <span class="label">{locale.t("add-folder")}</span>
        <div class="with"><bdi class="box path" dir="ltr">{folder}</bdi><button class="btn" onclick={() => void chooseFolder()}>{locale.t("add-change")}</button></div>
      </div>
      <div class="actions">
        <button class="btn primary" disabled={!name.trim() || !folder} onclick={() => void downloadOne(step.kind === "video" ? step.list[pick]!.choice : { kind: "audio_only" }, name.trim())}>
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
          {#if e.playlist}
            <li class="inner">
              <Icon name="queue" size={15} /><bdi dir="auto">{e.title ?? e.url ?? "—"}</bdi>
              {#if e.url}<button class="btn" onclick={() => void openPlaylist(e.url!)} data-first={i === 0 ? true : undefined}>{locale.t("video-open-playlist")}</button>{/if}
            </li>
          {:else}
            <li><label><input type="checkbox" bind:checked={list.chosen[i]} disabled={!e.url} data-first={i === 0 ? true : undefined} /><bdi dir="auto">{e.title ?? e.url ?? "—"}</bdi></label></li>
          {/if}
        {/each}
      </ul>
      {#if list.info.entries.length > 0 && list.info.entries.every((e) => e.playlist)}
        <p class="muted">{locale.t("video-playlists-only")}</p>
      {/if}
      <div class="with"><bdi class="box path" dir="ltr">{folder}</bdi><button class="btn" onclick={() => void chooseFolder()}>{locale.t("add-change")}</button></div>
      <div class="actions">
        <button class="btn primary" disabled={!list.chosen.some(Boolean) || !folder} onclick={() => void downloadList(list.info, list.chosen)}><Icon name="download" size={15} />{locale.t("video-download-chosen")}</button>
        <button class="btn" onclick={() => void close()}>{locale.t("add-cancel")}</button>
      </div>
    </section>
  {:else if step.kind === "started"}
    <!-- As IDM's «Download file info»: it downloads unseen meanwhile, so
         nothing here says it runs (owner, 2026-10-29). -->
    {@const s = step}
    <section class="col">
      <h2 class="title" dir="auto">{name || locale.t("video-started-reading")}</h2>
      <div class="grid">
        <!-- Every row from the first frame: what comes later fills them
             in, it adds none (the owner's notes). -->
        <span class="label">{locale.t("video-quality")}</span>
        <bdi class="box" dir="ltr">{s.label ?? "…"}</bdi>
        <span class="label">{locale.t("video-size")}</span>
        <span class="box num">{s.size === null ? locale.t("add-size-unknown") : size(locale.lang, locale.digits, s.size)}</span>
        <label class="label" for="started-name">{locale.t("add-file-name")}</label>
        <div class="namebox">
          <input
            id="started-name"
            class="box"
            class:quiet={!editing && name !== ""}
            dir="auto"
            bind:value={name}
            oninput={() => (nameTouched = true)}
            onfocus={() => (editing = true)}
            onblur={() => (editing = false)}
          />
          {#if !editing && name !== ""}<span class="box shown" dir={nameDir(name)} aria-hidden="true"><FileName {name} /></span>{/if}
        </div>
        <span class="label">{locale.t("add-folder")}</span>
        <div class="with"><bdi class="box path" dir="ltr">{folder}</bdi><button class="btn" onclick={() => void chooseFolder()}>{locale.t("add-change")}</button></div>
      </div>
      <!-- Its room is kept: the buttons do not move when it comes. -->
      <div class="slot">
        {#if s.status === "failed"}
          <p class="problem" role="alert"><ColorIcon name="s-failed" size={16} />{s.error ?? ""}</p>
        {/if}
      </div>
      <div class="actions">
        <button class="btn primary" disabled={!folder || s.status === "failed"} onclick={() => void keep(s.id, true)} data-first>
          <Icon name="download" size={15} />{locale.t("add-start")}
        </button>
        <button class="btn" disabled={!folder || s.status === "failed"} onclick={() => void keep(s.id, false)}>{locale.t("add-later")}</button>
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
  {:else}
    <section class="col">
      <p class="problem" role="alert"><ColorIcon name="s-failed" size={16} />{locale.t(errorKey(step.error))}</p>
      {#if errorDetails(step.error)}
        <details class="why"><summary>{locale.t("video-error-details")}</summary><p dir="ltr">{errorDetails(step.error)}</p></details>
      {/if}
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
  .namebox {
    display: grid;
    min-width: 0;
  }
  .namebox > * {
    grid-area: 1 / 1;
  }
  .box.quiet {
    color: transparent;
  }
  .box.shown {
    display: flex;
    align-items: center;
    pointer-events: none;
    background: transparent;
    border-color: transparent;
    overflow: hidden;
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
  /* A playlist inside it: opened, not chosen. */
  .entries li.inner {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 3px 10px;
    font-size: 13px;
  }
  .entries li.inner bdi {
    flex: 1;
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
  .slot {
    min-height: 22px;
  }
  .actions {
    display: flex;
    gap: 8px;
    margin-top: 4px;
  }
  .waiting {
    color: var(--muted);
  }
  .why {
    color: var(--muted);
    font-size: 12px;
  }
  .why p {
    margin: 6px 0 0;
    overflow-wrap: anywhere;
    user-select: text;
  }
  .problem {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    color: var(--red);
    margin: 0;
  }
</style>
