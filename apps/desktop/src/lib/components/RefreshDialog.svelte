<script lang="ts">
  import Icon from "./Icon.svelte";
  import FileName from "./FileName.svelte";
  import { locale } from "../i18n.svelte";

  let {
    name,
    problem,
    detail,
    busy,
    onrefresh,
    oncancel,
  }: {
    /// The download's file name.
    name: string;
    /// Why the last try was refused, in the user's language.
    problem: string | null;
    /// The program's own words about it (English), if any.
    detail: string | null;
    /// A refresh is on its way: the button waits.
    busy: boolean;
    onrefresh: (url: string) => void;
    oncancel: () => void;
  } = $props();

  let url = $state("");
  let dialog: HTMLDivElement | undefined = $state();
  // The focus goes to the link field at once, as in the add dialog.
  $effect(() => {
    dialog?.querySelector<HTMLInputElement>("input")?.focus();
  });
  function submit(event: SubmitEvent) {
    event.preventDefault();
    const link = url.trim();
    if (link && !busy) onrefresh(link);
  }
  // Escape closes it wherever the focus is; Tab stays inside it.
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
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="refresh-title" aria-describedby="refresh-hint" tabindex="-1" bind:this={dialog}>
    <h2 id="refresh-title">{locale.t("refresh-title")}</h2>
    <p class="name"><FileName {name} /></p>
    <p id="refresh-hint" class="hint">{locale.t("refresh-hint")}</p>
    <form onsubmit={submit}>
      <label class="label" for="refresh-url">{locale.t("refresh-url")}</label>
      <div class="value">
        <Icon name="link" />
        <input id="refresh-url" type="url" dir="ltr" autocomplete="off" spellcheck="false" bind:value={url} />
      </div>
      {#if problem}
        <div class="problem" role="alert">
          <Icon name="alert" />
          <div>
            <p>{problem}</p>
            {#if detail}<p class="detail"><bdi dir="ltr">{detail}</bdi></p>{/if}
          </div>
        </div>
      {/if}
      <div class="actions">
        <button type="button" class="btn quiet" onclick={oncancel}>{locale.t("refresh-cancel")}</button>
        <button type="submit" class="btn primary" disabled={!url.trim() || busy}>
          <Icon name="link" />{locale.t("refresh-go")}
        </button>
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
    width: min(560px, calc(100% - 32px));
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: 14px;
    box-shadow: var(--shadow);
    padding: 22px 24px 18px;
  }
  h2 {
    margin: 0 0 4px;
    font-size: 18px;
  }
  .name {
    margin: 0 0 10px;
    display: flex;
    min-width: 0;
    font-weight: 600;
  }
  .hint {
    margin: 0 0 14px;
    color: var(--muted);
    font-size: 13px;
    line-height: 1.6;
  }
  .label {
    display: block;
    font-size: 12.5px;
    font-weight: 600;
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
    padding: 7px 10px;
    margin-bottom: 12px;
  }
  .value:focus-within {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  input {
    border: 0;
    background: none;
    outline: none;
    width: 100%;
    min-width: 0;
    font: inherit;
    color: inherit;
  }
  .problem {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    border-radius: 9px;
    padding: 10px 12px;
    margin: 0 0 12px;
    font-size: 13px;
    background: var(--red-bg);
    color: var(--red);
  }
  .problem p {
    margin: 0;
  }
  .detail {
    margin-top: 4px !important;
    opacity: 0.85;
    font-size: 12px;
    overflow-wrap: anywhere;
  }
  .actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
    padding-top: 14px;
    border-top: 1px solid var(--line);
  }
</style>
