<script lang="ts">
  import Icon from "./Icon.svelte";
  import FileName from "./FileName.svelte";
  import { locale } from "../i18n.svelte";
  import { size } from "../format";
  import type { ProbeView } from "../bindings/ProbeView";
  import type { AddRequest } from "../bindings/AddRequest";
  import type { QueueView } from "../bindings/QueueView";

  let {
    url,
    probe,
    error,
    queues,
    onadd,
    oncancel,
  }: {
    url: string;
    probe: ProbeView | null;
    error: string | null;
    queues: QueueView[];
    onadd: (request: AddRequest) => void;
    oncancel: () => void;
  } = $props();

  let queue = $state("main");
  let dialog: HTMLDivElement | undefined = $state();
  $effect(() => {
    dialog?.querySelector<HTMLElement>("[data-autofocus]")?.focus();
  });

  const extension = $derived(probe ? (/\.([^.]+)$/.exec(probe.name)?.[1] ?? "") : "");
  function add(start: boolean) {
    onadd({ url, queue, folder: null, start, limit: null });
  }
  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Escape") oncancel();
  }
</script>

<div class="scrim">
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="add-title" tabindex="-1" bind:this={dialog} {onkeydown}>
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
        {locale.t(probe.resumable ? "add-probe-ranges" : "add-probe-single", {
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
            {probe.category
              ? locale.t("add-category-auto", { category: locale.t(`category-${probe.category}`), extension })
              : locale.t("category-none")}
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
      <button class="btn" disabled={!probe} onclick={() => add(false)}>{locale.t("add-later")}</button>
      <button class="btn primary" disabled={!probe} onclick={() => add(true)} data-autofocus>
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
