<script lang="ts">
  import { locale } from "../i18n.svelte";

  // «Check for a new version at start» (#124), bound to the settings'
  // `update_check`.
  let { checked, onchange }: { checked: boolean; onchange: (on: boolean) => void } = $props();
</script>

<label class="choice">
  <input type="checkbox" {checked} onchange={(e) => onchange(e.currentTarget.checked)} />
  <span>{locale.t("update-setting")}</span>
</label>
<p class="hint">{locale.t("update-setting-hint")}</p>

<style>
  /* As the settings' other choices. */
  .choice {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 5px 12px;
    cursor: pointer;
  }
  .choice:has(input:checked) {
    border-color: var(--accent);
    background: var(--sel);
  }
  .choice:has(input:focus-visible) {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .choice input {
    margin: 0;
    accent-color: var(--accent);
  }
  .hint {
    font-size: 12.5px;
    color: var(--muted);
    margin: 6px 0 0;
  }
</style>
