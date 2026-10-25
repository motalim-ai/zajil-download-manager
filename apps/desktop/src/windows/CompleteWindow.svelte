<script lang="ts">
  import { onMount } from "svelte";
  import { ask } from "@tauri-apps/plugin-dialog";
  import Icon from "../lib/components/Icon.svelte";
  import FileName from "../lib/components/FileName.svelte";
  import { api } from "../lib/api";
  import { completeApi, saveShow } from "../lib/completeApi";
  import { startWindow } from "../lib/windowStart";
  import { locale } from "../lib/i18n.svelte";
  import { size } from "../lib/format";
  import type { CompleteView } from "../lib/bindings/CompleteView";

  // «Download complete» (the design of #83, window 5): what finished and
  // where, «Open» (a program only once the user agrees, #84), «Open
  // folder», «Close», and «Don't show this window again».
  const id = Number(new URLSearchParams(location.search).get("id"));
  let done = $state<CompleteView | null>(null);
  let problem = $state<string | null>(null);
  let never = $state(false);

  onMount(() => {
    void (async () => {
      await startWindow();
      try {
        done = await completeApi.info(id);
      } catch (e) {
        problem = String(e);
      }
    })();
  });

  async function close() {
    if (never) await saveShow(false).catch(() => {});
    await api.closeWindow();
  }

  async function open(folder: boolean) {
    problem = null;
    try {
      if (!(await api.openDownload(id, folder))) {
        const yes = await ask(locale.t("open-code-question", { name: done?.name ?? "" }), {
          title: locale.t("app-name"),
          kind: "warning",
          okLabel: locale.t("open-code-ok"),
          cancelLabel: locale.t("confirm-cancel"),
        }).catch(() => false);
        if (!yes) return;
        await api.openDownload(id, folder, true);
      }
      await close();
    } catch (e) {
      problem = String(e);
    }
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") void close();
  }
</script>

<svelte:window {onkeydown} />

<main class="complete" aria-labelledby="complete-heading">
  <h1 id="complete-heading"><span class="badge" aria-hidden="true"><Icon name="check" /></span>{locale.t("complete-title")}</h1>

  {#if done}
    <div class="fields">
      <span class="label">{locale.t("complete-name")}</span>
      <span class="value"><FileName name={done.name} /></span>
      <span class="label">{locale.t("complete-size")}</span>
      <span class="value">{done.size === null ? "?" : size(locale.lang, locale.digits, done.size)}</span>
      <span class="label">{locale.t("complete-folder")}</span>
      <span class="value" title={done.folder}><bdi dir="ltr">{done.folder}</bdi></span>
      <span class="label">{locale.t("complete-url")}</span>
      <span class="value" title={done.url}><bdi dir="ltr">{done.url}</bdi></span>
    </div>
  {/if}
  {#if problem}<p class="problem" role="alert"><Icon name="alert" />{problem}</p>{/if}

  <label class="check"><input type="checkbox" bind:checked={never} />{locale.t("complete-dont-show")}</label>

  <div class="actions">
    <!-- svelte-ignore a11y_autofocus -->
    <button class="btn primary" disabled={!done} onclick={() => void open(false)} autofocus><Icon name="file" />{locale.t("complete-open")}</button>
    <button class="btn" disabled={!done} onclick={() => void open(true)}><Icon name="folder-open" />{locale.t("complete-open-folder")}</button>
    <button class="btn end" onclick={() => void close()}>{locale.t("complete-close")}</button>
  </div>
</main>

<style>
  .complete {
    display: flex;
    flex-direction: column;
    gap: 14px;
    height: 100%;
    padding: 18px 24px 18px;
    background: var(--surface);
    box-sizing: border-box;
  }
  h1 {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 18px;
    margin: 0;
  }
  .badge {
    display: inline-grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: 99px;
    background: var(--green-bg);
    color: var(--green);
  }
  .fields {
    flex: 1;
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 8px 18px;
    align-content: start;
    min-height: 0;
  }
  .label {
    font-size: 13px;
    color: var(--muted);
  }
  .value {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .check {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
  }
  .problem {
    display: flex;
    gap: 6px;
    align-items: center;
    color: var(--red);
    margin: 0;
  }
  .actions {
    display: flex;
    gap: 8px;
  }
  .actions .end {
    margin-inline-start: auto;
  }
</style>
