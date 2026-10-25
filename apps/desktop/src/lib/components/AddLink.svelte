<script lang="ts">
  import Icon from "./Icon.svelte";
  import { locale } from "../i18n.svelte";

  // «Add a link» (M5.5, the design of #83, window 2): a window of its own,
  // its box filled from the clipboard when a link was copied, and saying
  // so.
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
  let box: HTMLInputElement | undefined = $state();
  $effect(() => {
    url = given;
    // A new link (from the clipboard while open) is selected, ready to
    // replace.
    void given;
    queueMicrotask(() => box?.select());
  });
  function submit(event: SubmitEvent) {
    event.preventDefault();
    const link = url.trim();
    if (link) onok(link);
  }
  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      oncancel();
    }
  }
</script>

<svelte:window {onkeydown} />

<main class="link" aria-labelledby="addlink-title">
  <h1 id="addlink-title" class="visually-hidden">{locale.t("addlink-title")}</h1>
  <form onsubmit={submit}>
    <div class="row">
      <label class="label" for="addlink-url">{locale.t("addlink-url")}</label>
      <div class="value">
        <input
          id="addlink-url"
          type="url"
          dir="ltr"
          autocomplete="off"
          spellcheck="false"
          bind:this={box}
          bind:value={url}
          aria-describedby={fromClipboard ? "addlink-clip" : undefined}
        />
        <Icon name="link-2" size={16} />
      </div>
      {#if fromClipboard}
        <span></span>
        <p id="addlink-clip" class="note"><Icon name="clipboard-paste" size={15} />{locale.t("addlink-from-clipboard")}</p>
      {/if}
    </div>
    <div class="actions">
      <button type="submit" class="btn primary" disabled={!url.trim()}>{locale.t("addlink-ok")}</button>
      <button type="button" class="btn" onclick={oncancel}>{locale.t("addlink-cancel")}</button>
    </div>
  </form>
</main>

<style>
  .link {
    height: 100%;
    padding: 22px 24px 18px;
    background: var(--surface);
    box-sizing: border-box;
  }
  form {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .row {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 10px 18px;
    align-items: center;
  }
  .label {
    font-size: 13px;
    color: var(--muted);
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
    margin: 0;
  }
  .actions {
    display: flex;
    gap: 8px;
    margin-top: auto;
    padding-top: 16px;
  }
</style>
