<script lang="ts">
  import { onMount, tick } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import Icon from "../lib/components/Icon.svelte";
  import { api } from "../lib/api";
  import { locale } from "../lib/i18n.svelte";
  import { counted, num } from "../lib/format";
  import { startWindow } from "../lib/windowStart";
  import { isProblem, problemText } from "../lib/batchText";
  import type { BatchPreview } from "../lib/bindings/BatchPreview";
  import type { BatchRequest } from "../lib/bindings/BatchRequest";

  // «Batch download» (M7, as IDM's «Add batch download»): a link with a
  // pattern, its links shown before anything is added, then added to a
  // queue, started now or later, as «Download file info» adds one. Rust
  // makes the links again when they are added: this list is only shown.

  let pattern = $state("");
  let kind = $state<"numbers" | "letters">("numbers");
  let from = $state("1");
  let to = $state("10");
  let width = $state<number | null>(null);
  let preview = $state<BatchPreview | null>(null);
  let problem = $state<string[] | null>(null);
  let failed = $state<string | null>(null);
  let note = $state<string | null>(null);
  let queues = $state<string[]>(["main"]);
  let queue = $state("main");
  let folder = $state<string | null>(null);
  let busy = $state(false);

  const hasStar = $derived(pattern.includes("*"));
  const request = $derived<BatchRequest>({
    pattern,
    from: hasStar ? from : null,
    to: hasStar ? to : null,
    width: hasStar && kind === "numbers" && width !== null && width > 0 ? width : null,
  });

  onMount(() => {
    void (async () => {
      await startWindow();
      void getCurrentWindow()
        .setTitle(locale.t("batch-title"))
        .catch(() => {});
      queues = await api
        .snapshot()
        .then((s) => s.queues.map((q) => q.name))
        .catch(() => ["main"]);
      const pending = await api.batchPending().catch(() => null);
      if (pending === "") note = locale.t("batch-clipboard-empty");
      else if (pending) pattern = pending;
      await tick();
      document.getElementById("batch-pattern")?.focus();
    })();
  });

  // The preview follows what is typed, a moment after the last key.
  let timer: ReturnType<typeof setTimeout> | undefined;
  let asked = 0;
  $effect(() => {
    const now = { ...request };
    clearTimeout(timer);
    if (!now.pattern.trim()) {
      preview = null;
      problem = null;
      return;
    }
    timer = setTimeout(() => {
      const mine = ++asked;
      api.batchPreview(now).then(
        (p) => mine === asked && ((preview = p), (problem = null)),
        (e) => {
          if (mine !== asked) return;
          preview = null;
          problem = isProblem(e) ? problemText((k, a) => locale.t(k, a), e) : [String(e)];
        },
      );
    }, 250);
  });

  function setKind(next: "numbers" | "letters") {
    kind = next;
    if (next === "letters") {
      from = "a";
      to = "z";
    } else {
      from = "1";
      to = "10";
    }
  }

  async function chooseFolder() {
    const chosen = await openDialog({ directory: true, multiple: false }).catch(() => null);
    if (typeof chosen === "string") folder = chosen;
  }

  async function add(start: boolean) {
    if (busy || !preview) return;
    busy = true;
    failed = null;
    try {
      await api.batchAdd(request, queue, folder, start);
      await api.closeWindow();
    } catch (e) {
      failed = String(e);
      busy = false;
    }
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      void api.closeWindow();
    }
  }
</script>

<svelte:window {onkeydown} />

<main class="batch" aria-labelledby="batch-title">
  <h1 id="batch-title" class="visually-hidden">{locale.t("batch-title")}</h1>
  <section class="col">
    <label class="label" for="batch-pattern">{locale.t("batch-pattern")}</label>
    <input id="batch-pattern" class="box" dir="ltr" autocomplete="off" spellcheck="false" bind:value={pattern} aria-describedby="batch-pattern-hint" />
    <p class="muted" id="batch-pattern-hint">{locale.t("batch-pattern-hint", { example: "https://site.com/img*.jpg" })}</p>
    {#if note}<p class="muted" role="status">{note}</p>{/if}

    <fieldset class="range" disabled={!hasStar}>
      <legend>{locale.t("batch-kind")}</legend>
      <label class="choice"><input type="radio" name="batch-kind" checked={kind === "numbers"} onchange={() => setKind("numbers")} />{locale.t("batch-numbers")}</label>
      <label class="choice"><input type="radio" name="batch-kind" checked={kind === "letters"} onchange={() => setKind("letters")} />{locale.t("batch-letters")}</label>
      <span class="gap"></span>
      <label class="small">{locale.t("batch-from")}<input class="box short" dir="ltr" bind:value={from} /></label>
      <label class="small">{locale.t("batch-to")}<input class="box short" dir="ltr" bind:value={to} /></label>
      <label class="small"
        >{locale.t("batch-width")}<input class="box short" type="number" min="1" max="10" dir="ltr" disabled={kind === "letters"} bind:value={width} aria-describedby="batch-width-hint" /></label
      >
    </fieldset>
    {#if hasStar && kind === "numbers"}
      <p class="muted" id="batch-width-hint">{locale.t("batch-width-hint", { example: "001" })}</p>
    {/if}

    <div class="head">
      <span class="label">{locale.t("batch-links")}</span>
      {#if preview}
        <strong>{counted(locale.lang, locale.digits, "batch-links-count", preview.count)}</strong>
      {/if}
      <span class="muted end">{locale.t("batch-max", { max: num(locale.lang, locale.digits, preview?.max ?? 1000, 0) })}</span>
    </div>
    {#if problem}
      <div class="problem" role="alert"><Icon name="alert" size={16} /><span>{#each problem as line (line)}<span class="line">{line}</span>{/each}</span></div>
    {:else if preview}
      <ol class="links" dir="ltr" aria-label={locale.t("batch-links")}>
        {#each preview.shown as link (link)}<li>{link}</li>{/each}
      </ol>
      {#if preview.count > preview.shown.length}
        <p class="muted">{locale.t("batch-shown", { shown: num(locale.lang, locale.digits, preview.shown.length, 0) })}</p>
      {/if}
    {:else}
      <p class="muted links empty">{locale.t("batch-empty")}</p>
    {/if}

    <div class="grid">
      <label class="label" for="batch-queue">{locale.t("add-queue")}</label>
      <select id="batch-queue" class="box" bind:value={queue}>
        {#each queues as q (q)}<option value={q}>{q === "main" ? locale.t("queue-main") : q}</option>{/each}
      </select>
      <span class="label">{locale.t("add-folder")}</span>
      <div class="with">
        <bdi class="box path" dir={folder ? "ltr" : "auto"}>{folder ?? locale.t("batch-folder-category")}</bdi>
        <button class="btn" onclick={() => void chooseFolder()}>{locale.t("add-change")}</button>
      </div>
    </div>
    {#if failed}<p class="problem" role="alert"><Icon name="alert" size={16} />{failed}</p>{/if}
    <div class="actions">
      <button class="btn primary" disabled={!preview || busy} onclick={() => void add(true)}><Icon name="download" size={15} />{locale.t("add-start")}</button>
      <button class="btn" disabled={!preview || busy} onclick={() => void add(false)}>{locale.t("add-later")}</button>
      <span class="gap"></span>
      <button class="btn" onclick={() => void api.closeWindow()}>{locale.t("add-cancel")}</button>
    </div>
  </section>
</main>

<style>
  .batch {
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
  .muted {
    color: var(--muted);
    font-size: 12.5px;
    margin: 0;
  }
  .range {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 14px;
    align-items: center;
    border: 1px solid var(--line);
    border-radius: 10px;
    padding: 8px 12px;
    margin: 0;
  }
  .range:disabled {
    opacity: 0.55;
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
  .gap {
    flex: 1;
  }
  .head {
    display: flex;
    gap: 10px;
    align-items: baseline;
  }
  .end {
    margin-inline-start: auto;
  }
  .links {
    margin: 0;
    padding: 6px 10px 6px 0;
    padding-inline-start: 2.6em;
    height: 170px;
    overflow: auto;
    border: 1px solid var(--line);
    border-radius: 8px;
    font-size: 12.5px;
    background: var(--sunken);
  }
  .links li {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .links.empty {
    display: grid;
    place-items: center;
    padding: 0;
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
  .line {
    display: block;
  }
</style>
