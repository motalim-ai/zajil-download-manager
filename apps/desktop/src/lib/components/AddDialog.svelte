<script lang="ts">
  import { tick } from "svelte";
  import Icon from "./Icon.svelte";
  import FileName from "./FileName.svelte";
  import { categoryIcon } from "./Tree.svelte";
  import { locale } from "../i18n.svelte";
  import { size } from "../format";
  import type { ProbeView } from "../bindings/ProbeView";
  import type { AddRequest } from "../bindings/AddRequest";
  import type { QueueView } from "../bindings/QueueView";
  import type { CategoryView } from "../bindings/CategoryView";

  // «Download file info» (M5.5, the design of #83, window 3): what the
  // server said of the file, where it goes, and when. A window of its own;
  // the UI test scene draws it in the main one. The file name and the
  // category can be changed once zajil-core keeps them (#103), the
  // description once it has one (#95).
  let {
    url,
    probe,
    error,
    busy,
    queues,
    categories,
    onadd,
    oncancel,
    onchoosefolder,
    onnewcategory,
  }: {
    url: string;
    probe: ProbeView | null;
    error: string | null;
    /// An add is on its way: the buttons wait (review of #48: two clicks
    /// added the download twice).
    busy: boolean;
    queues: QueueView[];
    categories: CategoryView[];
    /// `remember`: the chosen folder becomes the category's.
    onadd: (request: AddRequest, remember: boolean) => void;
    oncancel: () => void;
    onchoosefolder: () => Promise<string | null>;
    onnewcategory: () => void;
  } = $props();

  // A folder the user chose; `null`: the category's (or the downloads
  // folder).
  let folder = $state<string | null>(null);
  let remember = $state(false);
  let later = $state(false);
  let main: HTMLElement | undefined = $state();

  // The focus goes to «Start download» once the server has answered.
  $effect(() => {
    if (probe && !busy) main?.querySelector<HTMLButtonElement>("[data-autofocus]")?.focus();
  });

  // The user's own name, else the translated built-in one (as the sidebar).
  const categoryName = (id: string) => categories.find((c) => c.id === id)?.name ?? locale.t(`category-${id}`);
  const probeText = $derived(
    probe ? (probe.resumable ? "add-probe-ranges" : probe.ranges ? "add-probe-unsure" : "add-probe-single") : "",
  );
  const extension = $derived((/\.([^.]+)$/.exec(probe?.name ?? "")?.[1] ?? "").toUpperCase());

  function add(start: boolean, queue = "main") {
    later = false;
    if (!busy) onadd({ url, queue, folder, start, limit: null }, remember && folder !== null && Boolean(probe?.category));
  }
  async function change() {
    const chosen = await onchoosefolder();
    if (chosen !== null) folder = chosen;
  }
  async function openLater() {
    later = !later;
    if (later) {
      await tick();
      main?.querySelector<HTMLButtonElement>(".queues [role='menuitem']")?.focus();
    }
  }
  function onQueuesKey(event: KeyboardEvent) {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    const items = [...(main?.querySelectorAll<HTMLButtonElement>(".queues [role='menuitem']") ?? [])];
    const at = items.indexOf(document.activeElement as HTMLButtonElement);
    items[(at + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length]?.focus();
  }
  // Escape closes the queues, then the window.
  function onkeydown(event: KeyboardEvent) {
    if (event.key !== "Escape") return;
    event.preventDefault();
    if (later) {
      later = false;
      document.getElementById("add-later-more")?.focus();
    } else oncancel();
  }
</script>

<svelte:window {onkeydown} />
<svelte:document onpointerdown={(e) => later && !(e.target as Element).closest?.(".split") && (later = false)} />

<main class="info" bind:this={main} aria-labelledby="add-title">
  <h1 id="add-title" class="visually-hidden">{locale.t("info-title")}</h1>
  <div class="body">
    <div class="fields">
      <span class="label">{locale.t("add-url")}</span>
      <div class="value ltr"><bdi dir="ltr">{url}</bdi><Icon name="link" /></div>

      {#if probe}
        <span class="label">{locale.t("add-category")}</span>
        <div class="with">
          <div class="value">
            <Icon name={categoryIcon(probe.category)} />
            <span class="text">
              {#if probe.category && probe.category_site}
                {locale.t("add-category-site", { category: categoryName(probe.category), site: probe.category_site })}
              {:else if probe.category}
                {locale.t("add-category-extension", { category: categoryName(probe.category), extension: probe.category_extension ?? "" })}
              {:else}
                {locale.t("category-none")}
              {/if}
            </span>
          </div>
          <button class="btn square" title={locale.t("add-new-category")} aria-label={locale.t("add-new-category")} onclick={onnewcategory}>
            <Icon name="circle-plus" />
          </button>
        </div>

        <span class="label">{locale.t("add-file-name")}</span>
        <div class="value"><span class="text"><FileName name={probe.name} /></span></div>

        <span class="label">{locale.t("add-folder")}</span>
        <div class="with">
          <div class="value"><Icon name="folder" /><bdi dir="ltr" class="text">{folder ?? probe.folder}</bdi></div>
          <button class="btn" onclick={() => void change()}>{locale.t("add-change")}</button>
        </div>

        {#if probe.category}
          <span></span>
          <label class="check" class:off={folder === null}>
            <input type="checkbox" bind:checked={remember} disabled={folder === null} />
            {locale.t("add-remember", { category: categoryName(probe.category) })}
          </label>
        {/if}
      {/if}
    </div>

    <aside class="card" aria-live="polite" aria-label={locale.t("add-about-file")}>
      {#if error}
        <p class="problem" role="alert"><Icon name="alert" />{error}</p>
      {:else if probe}
        <span class="type" aria-hidden="true"><Icon name={categoryIcon(probe.category)} size={30} /></span>
        <strong class="size">{probe.size === null ? "?" : size(locale.lang, locale.digits, probe.size)}</strong>
        <span class="kind">{probe.category ? categoryName(probe.category) : locale.t("category-none")} {extension}</span>
        <span class="resume" class:no={!probe.resumable}>
          <Icon name={probe.resumable ? "check" : "circle-alert"} size={15} />{locale.t(probe.resumable ? "add-resumable" : "add-not-resumable")}
        </span>
        <p class="why">
          {locale.t(probeText, { size: probe.size === null ? "?" : size(locale.lang, locale.digits, probe.size) })}
        </p>
      {:else}
        <p class="waiting" role="status">{locale.t("add-checking")}</p>
      {/if}
    </aside>
  </div>

  <div class="actions">
    <button class="btn primary" disabled={!probe || busy} onclick={() => add(true)} data-autofocus>
      <Icon name="down" />{locale.t("add-start")}
    </button>
    <div class="split">
      <button class="btn" disabled={!probe || busy} onclick={() => add(false)}>
        <Icon name="list-ordered" />{locale.t("add-later")}
      </button>
      <button
        id="add-later-more"
        class="btn more"
        disabled={!probe || busy}
        aria-label={locale.t("add-later-queue")}
        aria-haspopup="menu"
        aria-expanded={later}
        onclick={() => void openLater()}
      >
        <Icon name="chevron-down" size={15} />
      </button>
      {#if later}
        <div class="queues" role="menu" tabindex="-1" aria-label={locale.t("add-later-queue")} onkeydown={onQueuesKey}>
          {#each queues as q (q.name)}
            <button class="q" role="menuitem" onclick={() => add(false, q.name)}>
              <bdi>{q.name === "main" ? locale.t("queue-main") : q.name}</bdi>
            </button>
          {/each}
        </div>
      {/if}
    </div>
    <button class="btn quiet end" onclick={oncancel}>{locale.t("add-cancel")}</button>
  </div>
</main>

<style>
  .info {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: 20px 24px 18px;
    background: var(--surface);
    box-sizing: border-box;
  }
  .body {
    flex: 1;
    display: grid;
    grid-template-columns: 1fr 190px;
    gap: 24px;
    min-height: 0;
  }
  .fields {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 10px 18px;
    align-content: start;
    align-items: center;
    min-width: 0;
  }
  .label {
    font-size: 13px;
    color: var(--muted);
  }
  .with {
    display: flex;
    gap: 8px;
    min-width: 0;
  }
  .with .value {
    flex: 1;
  }
  .value {
    display: flex;
    align-items: center;
    gap: 8px;
    border: 1px solid var(--line);
    background: var(--sunken);
    border-radius: 8px;
    padding: 6px 10px;
    min-height: 34px;
    min-width: 0;
    box-sizing: border-box;
    overflow: hidden;
  }
  .value.ltr {
    justify-content: space-between;
  }
  .value bdi,
  .text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .square {
    padding-inline: 8px;
  }
  .check {
    display: flex;
    gap: 8px;
    align-items: center;
    font-size: 13px;
  }
  .check.off {
    color: var(--faint);
  }
  .card {
    border: 1px solid var(--line);
    border-radius: 12px;
    background: var(--sunken);
    padding: 16px 12px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    text-align: center;
    align-self: start;
  }
  .type {
    display: grid;
    place-items: center;
    width: 56px;
    height: 56px;
    border-radius: 14px;
    background: var(--sel);
    color: var(--accent);
  }
  .size {
    font-size: 18px;
  }
  .kind {
    color: var(--muted);
    font-size: 13px;
  }
  .resume {
    display: flex;
    gap: 6px;
    align-items: center;
    color: var(--green);
    font-size: 13px;
  }
  .resume.no {
    color: var(--amber);
  }
  .why {
    margin: 4px 0 0;
    font-size: 11.5px;
    color: var(--muted);
  }
  .waiting {
    margin: 0;
    color: var(--muted);
  }
  .problem {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    margin: 0;
    color: var(--red);
    font-size: 13px;
    text-align: start;
  }
  .actions {
    display: flex;
    gap: 8px;
    align-items: center;
    padding-top: 16px;
  }
  .end {
    margin-inline-start: auto;
  }
  .split {
    position: relative;
    display: flex;
  }
  .split > .btn:first-child {
    border-start-end-radius: 0;
    border-end-end-radius: 0;
  }
  .more {
    border-start-start-radius: 0;
    border-end-start-radius: 0;
    border-inline-start: 0;
    padding-inline: 8px;
  }
  .queues {
    position: absolute;
    bottom: calc(100% + 4px);
    inset-inline-start: 0;
    min-width: 100%;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: 8px;
    box-shadow: var(--shadow);
    padding: 4px;
    display: flex;
    flex-direction: column;
    z-index: 5;
  }
  .q {
    border: 0;
    background: none;
    text-align: start;
    padding: 6px 10px;
    border-radius: 6px;
    font: inherit;
    color: var(--ink);
  }
  .q:hover,
  .q:focus-visible {
    background: var(--sel);
    outline: none;
  }
</style>
