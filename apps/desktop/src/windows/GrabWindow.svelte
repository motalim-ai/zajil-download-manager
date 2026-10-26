<script lang="ts">
  import { onMount, tick } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import Icon from "../lib/components/Icon.svelte";
  import { api } from "../lib/api";
  import { locale } from "../lib/i18n.svelte";
  import { counted, num } from "../lib/format";
  import { startWindow } from "../lib/windowStart";
  import { limitText, listOf, refusalText } from "../lib/grabText";
  import type { GrabScan } from "../lib/bindings/GrabScan";

  // «Site grabber» (M7, ADR-0010, as IDM's): a start page, the depth, the
  // kinds of files and the other sites allowed; «Check» reads the site
  // (pages only, one request a second) and lists the files; the user takes
  // some out; they go into the site's own queue, one download at a time.
  // robots.txt is always respected (owner's decision 2 of ADR-0010).
  // Rust keeps the list: this page only says which places it took out.

  // ADR-0010's defaults and caps (`zajil_grab::crawl`).
  const DEPTH = 2;
  const MAX_DEPTH = 10;
  const FILES = 500;
  const MAX_FILES = 10_000;
  const TOTAL_GB = 5;
  const GIB = 1024 * 1024 * 1024;
  const KINDS = ["archives", "documents", "audio", "video", "programs", "images"] as const;

  let start = $state("");
  let depth = $state(DEPTH);
  let kinds = $state<Record<string, boolean>>({ archives: true, documents: true, audio: true, video: true, programs: true, images: false });
  let extensions = $state("");
  let sitesOn = $state(false);
  let sites = $state("");
  let maxFiles = $state(FILES);
  let maxTotal = $state(TOTAL_GB);

  let checking = $state(false);
  let progress = $state({ pages: 0, files: 0 });
  let scan = $state<GrabScan | null>(null);
  let chosen = $state<boolean[]>([]);
  let problem = $state<string | null>(null);
  let folder = $state<string | null>(null);
  let busy = $state(false);

  const chosenCount = $derived(chosen.filter(Boolean).length);

  onMount(() => {
    let unlisten: (() => void) | undefined;
    void (async () => {
      await startWindow();
      void getCurrentWindow()
        .setTitle(locale.t("grab-title"))
        .catch(() => {});
      unlisten = await api.onGrabProgress((p) => (progress = { pages: p.pages, files: p.files }));
      const pending = await api.grabPending().catch(() => null);
      if (pending) start = pending;
      await tick();
      document.getElementById("grab-start")?.focus();
    })();
    return () => unlisten?.();
  });

  async function check() {
    if (checking) return;
    checking = true;
    problem = null;
    scan = null;
    progress = { pages: 0, files: 0 };
    try {
      const found = await api.grabCheck({
        start: start.trim(),
        depth: Number(depth),
        kinds: KINDS.filter((k) => kinds[k]),
        extensions: listOf(extensions),
        sites: sitesOn ? listOf(sites) : [],
        max_files: Number(maxFiles),
        max_total: Math.round(Number(maxTotal) * GIB),
      });
      scan = found;
      chosen = found.files.map(() => true);
    } catch (e) {
      problem = refusalText((k, a) => locale.t(k, a), String(e), { maxDepth: MAX_DEPTH, maxFiles: MAX_FILES });
    } finally {
      checking = false;
    }
  }

  async function chooseFolder() {
    const picked = await openDialog({ directory: true, multiple: false }).catch(() => null);
    if (typeof picked === "string") folder = picked;
  }

  async function add(startNow: boolean) {
    if (!scan || busy || chosenCount === 0) return;
    busy = true;
    problem = null;
    const removed = chosen.flatMap((on, i) => (on ? [] : [i]));
    try {
      await api.grabAdd(scan.id, removed, folder, startNow);
      await api.closeWindow();
    } catch (e) {
      problem = String(e);
      busy = false;
    }
  }

  async function close() {
    if (checking) await api.grabCancel().catch(() => {});
    await api.closeWindow();
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      void close();
    }
  }
</script>

<svelte:window {onkeydown} />

<main class="grab" aria-labelledby="grab-title">
  <h1 id="grab-title" class="visually-hidden">{locale.t("grab-title")}</h1>
  <section class="col">
    <label class="label" for="grab-start">{locale.t("grab-start")}</label>
    <input id="grab-start" class="box" dir="ltr" autocomplete="off" spellcheck="false" placeholder="https://example.com/files/" bind:value={start} />

    <div class="row">
      <label class="small" for="grab-depth">{locale.t("grab-depth")}</label>
      <input id="grab-depth" class="box short" type="number" min="1" max={MAX_DEPTH} dir="ltr" bind:value={depth} aria-describedby="grab-depth-hint" />
      <span class="muted" id="grab-depth-hint">{locale.t("grab-depth-hint")}</span>
    </div>

    <fieldset class="group">
      <legend>{locale.t("grab-kinds")}</legend>
      <div class="kinds">
        {#each KINDS as kind (kind)}
          <label class="choice"><input type="checkbox" bind:checked={kinds[kind]} />{locale.t(`grab-kind-${kind}`)}</label>
        {/each}
      </div>
      <label class="small" for="grab-ext">{locale.t("grab-extensions")}</label>
      <input id="grab-ext" class="box" dir="ltr" spellcheck="false" bind:value={extensions} aria-describedby="grab-ext-hint" />
      <p class="muted" id="grab-ext-hint">{locale.t("grab-extensions-hint", { example: "psd, ai, svg" })}</p>
    </fieldset>

    <label class="choice"><input type="checkbox" bind:checked={sitesOn} />{locale.t("grab-sites-on")}</label>
    {#if sitesOn}
      <input class="box" dir="ltr" spellcheck="false" aria-label={locale.t("grab-sites-on")} bind:value={sites} aria-describedby="grab-sites-hint" />
      <p class="muted" id="grab-sites-hint">{locale.t("grab-sites-hint", { example: "cdn.example.com" })}</p>
    {/if}
    <p class="muted robots"><Icon name="info" size={14} />{locale.t("grab-robots")}</p>

    <fieldset class="group limits">
      <legend>{locale.t("grab-limits")}</legend>
      <label class="small">{locale.t("grab-max-files")}<input class="box short" type="number" min="1" max={MAX_FILES} dir="ltr" bind:value={maxFiles} /></label>
      <span class="muted">{locale.t("grab-files-hint", { default: num(locale.lang, locale.digits, FILES, 0), max: num(locale.lang, locale.digits, MAX_FILES, 0) })}</span>
      <label class="small">{locale.t("grab-max-total")}<input class="box short" type="number" min="1" step="1" dir="ltr" bind:value={maxTotal} /></label>
      <span class="muted">{locale.t("grab-total-hint", { default: num(locale.lang, locale.digits, TOTAL_GB, 0) })}</span>
    </fieldset>

    <div class="actions">
      {#if checking}
        <button class="btn" onclick={() => void api.grabCancel()}>{locale.t("grab-stop")}</button>
        <span class="muted" role="status">{locale.t("grab-checking")} {locale.t("grab-progress", { pages: num(locale.lang, locale.digits, progress.pages, 0), files: num(locale.lang, locale.digits, progress.files, 0) })}</span>
      {:else}
        <button class="btn primary" disabled={!start.trim()} onclick={() => void check()}><Icon name="search" size={15} />{locale.t("grab-check")}</button>
      {/if}
    </div>

    {#if problem}<p class="problem" role="alert"><Icon name="alert" size={16} />{problem}</p>{/if}

    {#if scan}
      <div class="head">
        <strong>{counted(locale.lang, locale.digits, "files-count", scan.files.length)}</strong>
        {#if scan.files.length > 0}
          <span class="muted">{locale.t("grab-chosen", { chosen: num(locale.lang, locale.digits, chosenCount, 0), all: num(locale.lang, locale.digits, scan.files.length, 0) })}</span>
          <span class="end"></span>
          <button class="btn quiet" onclick={() => (chosen = chosen.map(() => true))}>{locale.t("grab-select-all")}</button>
          <button class="btn quiet" onclick={() => (chosen = chosen.map(() => false))}>{locale.t("grab-select-none")}</button>
        {/if}
      </div>
      {#if scan.robots_closed}<p class="problem" role="status">{locale.t("grab-robots-closed")}</p>
      {:else if scan.files.length === 0}<p class="muted" role="status">{locale.t("grab-none")}</p>{/if}
      {#if scan.files.length > 0}
        <ul class="files" dir="ltr" aria-label={locale.t("grab-kinds")}>
          {#each scan.files as file, i (i)}
            <li><label><input type="checkbox" bind:checked={chosen[i]} /><span class="link">{file}</span></label></li>
          {/each}
        </ul>
      {/if}
      <ul class="notes">
        {#if scan.stopped}<li>{locale.t("grab-stopped")}</li>{/if}
        {#each scan.limits as limit (limit)}<li>{limitText((k, a) => locale.t(k, a), limit)}</li>{/each}
        {#if scan.skipped > 0}<li>{locale.t("grab-skipped", { skipped: num(locale.lang, locale.digits, scan.skipped, 0) })}</li>{/if}
      </ul>

      {#if scan.files.length > 0}
        <div class="grid">
          <span class="label">{locale.t("add-folder")}</span>
          <div class="with">
            <bdi class="box path" dir={folder ? "ltr" : "auto"}>{folder ?? locale.t("batch-folder-category")}</bdi>
            <button class="btn" onclick={() => void chooseFolder()}>{locale.t("add-change")}</button>
          </div>
        </div>
        <p class="muted">{locale.t("grab-queue-note")}</p>
        <div class="actions">
          <button class="btn primary" disabled={chosenCount === 0 || busy} onclick={() => void add(true)}><Icon name="download" size={15} />{locale.t("add-start")}</button>
          <button class="btn" disabled={chosenCount === 0 || busy} onclick={() => void add(false)}>{locale.t("add-later")}</button>
          <span class="end"></span>
          <button class="btn" onclick={() => void close()}>{locale.t("add-cancel")}</button>
        </div>
      {/if}
    {/if}
  </section>
</main>

<style>
  .grab {
    height: 100%;
    box-sizing: border-box;
    padding: 18px 22px;
    background: var(--surface);
    overflow: auto;
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: 10px;
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
  .short {
    width: 6.5em;
    margin-inline-start: 6px;
  }
  .row,
  .head,
  .kinds,
  .limits {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 14px;
    align-items: center;
  }
  .group {
    display: flex;
    flex-direction: column;
    gap: 8px;
    border: 1px solid var(--line);
    border-radius: 10px;
    padding: 8px 12px;
    margin: 0;
  }
  .limits {
    flex-direction: row;
  }
  legend {
    font-size: 12.5px;
    color: var(--muted);
    padding: 0 6px;
  }
  .choice,
  .small {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13.5px;
  }
  .muted {
    color: var(--muted);
    font-size: 12.5px;
    margin: 0;
  }
  .robots {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .end {
    margin-inline-start: auto;
  }
  .files {
    list-style: none;
    margin: 0;
    padding: 4px 8px;
    max-height: 220px;
    overflow: auto;
    border: 1px solid var(--line);
    border-radius: 8px;
    background: var(--sunken);
    font-size: 12.5px;
  }
  .files label {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 2px 0;
  }
  .link {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .notes {
    margin: 0;
    padding-inline-start: 1.2em;
    color: var(--muted);
    font-size: 12.5px;
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
  .path {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .actions {
    display: flex;
    gap: 8px;
    align-items: center;
    margin-top: 4px;
  }
  .problem {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    color: var(--red);
    margin: 0;
    padding: 10px 12px;
    border-radius: 8px;
    background: var(--red-bg);
    font-size: 13px;
  }
</style>
