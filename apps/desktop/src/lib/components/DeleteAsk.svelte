<script lang="ts">
  import { tick } from "svelte";
  import Icon from "./Icon.svelte";
  import { locale } from "../i18n.svelte";

  // Deleting downloads that have not finished (an IDM touch): asks first,
  // and whether their unfinished files go too: on at first (owner's
  // decision, 2026-10-29: a download deleted before it finished leaves
  // nothing on the disk unless the user keeps it).
  let {
    unfinished,
    total,
    onanswer,
  }: {
    unfinished: number;
    total: number;
    /** `null`: not deleted; else deleted, with the unfinished files or not. */
    onanswer: (withFiles: boolean | null) => void;
  } = $props();

  let withFiles = $state(true);
  let dialog: HTMLDivElement | undefined = $state();
  $effect(() => {
    void tick().then(() => dialog?.querySelector<HTMLButtonElement>("[data-cancel]")?.focus());
  });
  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      onanswer(null);
    }
  }
</script>

<svelte:window {onkeydown} />

<div class="scrim">
  <div class="dialog" role="alertdialog" aria-modal="true" aria-labelledby="del-q" tabindex="-1" bind:this={dialog}>
    <p id="del-q" class="q"><Icon name="circle-alert" size={18} />{locale.t("delete-unfinished", { count: unfinished })}</p>
    {#if total > unfinished}
      <p class="also">{locale.t("delete-also-finished", { count: total - unfinished })}</p>
    {/if}
    <label class="check"><input type="checkbox" bind:checked={withFiles} />{locale.t("delete-unfinished-files")}</label>
    <div class="actions">
      <button class="btn danger" onclick={() => onanswer(withFiles)}><Icon name="trash-2" size={15} />{locale.t("confirm-remove")}</button>
      <button class="btn" data-cancel onclick={() => onanswer(null)}>{locale.t("confirm-cancel")}</button>
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
    z-index: 20;
  }
  .dialog {
    width: min(460px, calc(100% - 32px));
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: 12px;
    box-shadow: var(--shadow);
    padding: 18px 20px 16px;
  }
  .q {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    margin: 0 0 8px;
    font-size: 14px;
  }
  .q :global(.icon) {
    color: var(--amber);
    flex: none;
  }
  .also {
    margin: 0 0 8px;
    font-size: 12.5px;
    color: var(--muted);
  }
  .check {
    display: flex;
    gap: 8px;
    align-items: center;
    font-size: 13px;
  }
  .actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
    margin-top: 16px;
  }
  .danger {
    color: var(--red);
  }
</style>
