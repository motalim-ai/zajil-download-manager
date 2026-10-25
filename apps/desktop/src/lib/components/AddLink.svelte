<script lang="ts">
  import Icon from "./Icon.svelte";
  import { locale } from "../i18n.svelte";

  let {
    url: given,
    fromClipboard,
    onok,
    oncancel,
  }: {
    /** The link it starts with. */
    url: string;
    /** That link came from the clipboard: said under the box. */
    fromClipboard: boolean;
    onok: (url: string) => void;
    oncancel: () => void;
  } = $props();

  let url = $state("");
  let dialog: HTMLDivElement | undefined = $state();
  $effect(() => {
    url = given;
  });
  $effect(() => {
    dialog?.querySelector<HTMLInputElement>("input")?.select();
  });
  function submit(event: SubmitEvent) {
    event.preventDefault();
    const link = url.trim();
    if (link) onok(link);
  }
  // Escape closes it; Tab stays inside it.
  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      oncancel();
    } else if (event.key === "Tab" && dialog) {
      const inside = [...dialog.querySelectorAll<HTMLElement>("button:not(:disabled), input")];
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
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="addlink-title" tabindex="-1" bind:this={dialog}>
    <h2 id="addlink-title">{locale.t("addlink-title")}</h2>
    <form onsubmit={submit}>
      <label class="label" for="addlink-url">{locale.t("addlink-url")}</label>
      <div class="value">
        <Icon name="link-2" size={16} />
        <input id="addlink-url" type="url" dir="ltr" autocomplete="off" spellcheck="false" bind:value={url} aria-describedby={fromClipboard ? "addlink-clip" : undefined} />
      </div>
      {#if fromClipboard}
        <p id="addlink-clip" class="note"><Icon name="clipboard-paste" size={15} />{locale.t("addlink-from-clipboard")}</p>
      {/if}
      <div class="actions">
        <button type="submit" class="btn primary" disabled={!url.trim()}>{locale.t("addlink-ok")}</button>
        <button type="button" class="btn" onclick={oncancel}>{locale.t("addlink-cancel")}</button>
      </div>
    </form>
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
    width: min(520px, calc(100% - 32px));
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: 14px;
    box-shadow: var(--shadow);
    padding: 20px 22px 18px;
  }
  h2 {
    margin: 0 0 12px;
    font-size: 17px;
  }
  .label {
    display: block;
    font-size: 13px;
    color: var(--muted);
    margin-bottom: 4px;
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
    color: var(--faint);
  }
  .value:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in oklab, var(--accent) 22%, transparent);
    background: var(--surface);
  }
  input {
    border: 0;
    background: none;
    outline: none;
    width: 100%;
    min-width: 0;
    font: inherit;
    color: var(--ink);
    text-align: left;
  }
  .note {
    display: flex;
    gap: 8px;
    align-items: center;
    font-size: 12.5px;
    color: var(--muted);
    margin: 8px 0 0;
  }
  .actions {
    display: flex;
    gap: 8px;
    margin-top: 16px;
  }
</style>
