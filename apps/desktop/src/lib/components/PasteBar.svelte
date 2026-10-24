<script lang="ts">
  // Always above the list (from design B): paste a link and press Enter or
  // "Download"; the add dialog opens with what the server says.
  import Icon from "./Icon.svelte";
  import { locale } from "../i18n.svelte";
  let { value = $bindable(""), onsubmit }: { value?: string; onsubmit: (url: string) => void } = $props();
  function submit(event: SubmitEvent) {
    event.preventDefault();
    const url = value.trim();
    if (url) onsubmit(url);
  }
</script>

<form class="paste" onsubmit={submit}>
  <Icon name="link" />
  <label class="visually-hidden" for="paste-url">{locale.t("add-url")}</label>
  <input id="paste-url" type="url" dir="ltr" placeholder={locale.t("paste-placeholder")} bind:value autocomplete="off" spellcheck="false" />
  <button class="btn primary" type="submit"><Icon name="down" />{locale.t("paste-download")}</button>
</form>

<style>
  .paste {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 12px 16px 0;
    padding: 4px;
    padding-inline-start: 12px;
    border: 1px solid var(--line);
    background: var(--sunken);
    border-radius: 10px;
    color: var(--faint);
  }
  input {
    flex: 1;
    border: 0;
    background: none;
    outline: none;
    padding: 6px 0;
    font-size: 14.5px;
    color: var(--ink);
    text-align: start;
  }
  input::placeholder {
    color: var(--faint);
    direction: inherit;
  }
  :global([dir="rtl"]) input:placeholder-shown {
    direction: rtl;
  }
</style>
