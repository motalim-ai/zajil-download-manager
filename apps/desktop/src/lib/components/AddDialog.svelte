<script lang="ts">
  import Icon from "./Icon.svelte";
  import FileName from "./FileName.svelte";
  import { locale } from "../i18n.svelte";
  import { size } from "../format";
  import type { ProbeView } from "../bindings/ProbeView";
  import type { AddRequest } from "../bindings/AddRequest";
  import type { QueueView } from "../bindings/QueueView";
  import type { CategoryView } from "../bindings/CategoryView";

  let {
    url,
    probe,
    error,
    busy,
    queues,
    categories,
    onadd,
    oncancel,
  }: {
    url: string;
    probe: ProbeView | null;
    error: string | null;
    /// An add is on its way: the buttons wait (review of #48: two clicks
    /// added the download twice).
    busy: boolean;
    queues: QueueView[];
    categories: CategoryView[];
    onadd: (request: AddRequest) => void;
    oncancel: () => void;
  } = $props();

  let queue = $state("main");
  let dialog: HTMLDivElement | undefined = $state();
  // Focus goes into the dialog at once, and to the main button once the
  // server has answered (review of #48: it stayed on the paste bar, so
  // Escape and Enter went there).
  $effect(() => {
    if (!dialog) return;
    const main = dialog.querySelector<HTMLButtonElement>("[data-autofocus]");
    if (probe && main && !main.disabled) main.focus();
    else if (!dialog.contains(document.activeElement)) dialog.focus();
  });

  // The user's own name, else the translated built-in one (as the sidebar).
  const categoryName = (id: string) => categories.find((c) => c.id === id)?.name ?? locale.t(`category-${id}`);
  const probeText = $derived(
    probe ? (probe.resumable ? "add-probe-ranges" : probe.ranges ? "add-probe-unsure" : "add-probe-single") : "",
  );
  function add(start: boolean) {
    if (!busy) onadd({ url, queue, folder: null, start, limit: null });
  }
  // Escape closes it wherever the focus is; Tab stays inside it.
  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      oncancel();
    } else if (event.key === "Tab" && dialog) {
      const inside = [...dialog.querySelectorAll<HTMLElement>("button:not(:disabled), select, [href], input")];
      if (inside.length === 0) {
        event.preventDefault();
        return;
      }
      const [first, last] = [inside[0], inside[inside.length - 1]];
      const at = document.activeElement;
      if (event.shiftKey && (at === first || at === dialog)) {
        event.preventDefault();
        last?.focus();
      } else if (!event.shiftKey && (at === last || !dialog.contains(at))) {
        event.preventDefault();
        first?.focus();
      }
    }
  }
</script>

<svelte:window {onkeydown} />

<div class="scrim">
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="add-title" tabindex="-1" bind:this={dialog}>
    <h2 id="add-title">{locale.t("add-title")}</h2>
    <div class="field">
      <span class="label">{locale.t("add-url")}</span>
      <div class="value ltr"><Icon name="link" /><bdi dir="ltr">{url}</bdi></div>
    </div>
    {#if error}
      <p class="problem" role="alert"><Icon name="alert" />{error}</p>
    {:else if probe}
      <p class="probe" role="status">
        <Icon name="check" />
        {locale.t(probeText, {
          size: probe.size === null ? "?" : size(locale.lang, locale.digits, probe.size),
        })}
      </p>
      <div class="field">
        <span class="label">{locale.t("add-file-name")}</span>
        <div class="value"><FileName name={probe.name} /></div>
      </div>
      <div class="row">
        <div class="field">
          <span class="label">{locale.t("add-category")}</span>
          <div class="value">
            <Icon name="tag" />
            {#if probe.category && probe.category_site}
              {locale.t("add-category-site", { category: categoryName(probe.category), site: probe.category_site })}
            {:else if probe.category}
              {locale.t("add-category-extension", { category: categoryName(probe.category), extension: probe.category_extension ?? "" })}
            {:else}
              {locale.t("category-none")}
            {/if}
          </div>
        </div>
        <div class="field">
          <label class="label" for="add-queue">{locale.t("add-queue")}</label>
          <div class="value">
            <Icon name="queue" />
            <select id="add-queue" bind:value={queue}>
              {#each queues as q (q.name)}<option value={q.name}>{q.name}</option>{/each}
            </select>
          </div>
        </div>
      </div>
      <div class="field">
        <span class="label">{locale.t("add-folder")}</span>
        <div class="value"><Icon name="folder" /><bdi dir="ltr">{probe.folder}</bdi></div>
      </div>
    {:else}
      <p class="probe waiting" role="status">…</p>
    {/if}
    <div class="actions">
      <button class="btn quiet" onclick={oncancel}>{locale.t("add-cancel")}</button>
      <button class="btn" disabled={!probe || busy} onclick={() => add(false)}>{locale.t("add-later")}</button>
      <button class="btn primary" disabled={!probe || busy} onclick={() => add(true)} data-autofocus>
        <Icon name="down" />{locale.t("add-start")}
      </button>
    </div>
  </div>
</div>

<style>
  .scrim {
    position: absolute;
    inset: 0;
    background: var(--scrim);
    display: grid;
    place-items: center;
    z-index: 10;
  }
  .dialog {
    width: min(560px, calc(100% - 32px));
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: 14px;
    box-shadow: var(--shadow);
    padding: 22px 24px 18px;
  }
  h2 {
    margin: 0 0 14px;
    font-size: 18px;
  }
  .field {
    display: grid;
    gap: 4px;
    margin-bottom: 12px;
    min-width: 0;
  }
  .label {
    font-size: 12.5px;
    font-weight: 600;
    color: var(--muted);
  }
  .value {
    display: flex;
    align-items: center;
    gap: 8px;
    border: 1px solid var(--line);
    background: var(--sunken);
    border-radius: 8px;
    padding: 7px 10px;
    min-width: 0;
    overflow: hidden;
  }
  .value bdi {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  select {
    border: 0;
    background: none;
    outline: none;
    width: 100%;
  }
  .probe,
  .problem {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    border-radius: 9px;
    padding: 10px 12px;
    margin: 0 0 14px;
    font-size: 13px;
  }
  .probe {
    background: color-mix(in oklab, var(--green-bg) 70%, transparent);
  }
  .problem {
    background: var(--red-bg);
    color: var(--red);
  }
  .row {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }
  .actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
    margin-top: 8px;
    padding-top: 14px;
    border-top: 1px solid var(--line);
  }
</style>
