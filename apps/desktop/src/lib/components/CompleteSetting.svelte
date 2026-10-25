<script lang="ts">
  import { loadShow, saveShow } from "../completeApi";
  import { locale } from "../i18n.svelte";

  // The setting that brings «Download complete» back after «Don't show
  // this window again».
  let on = $state(loadShow());
</script>

<label class="choice">
  <input type="checkbox" checked={on} onchange={(e) => ((on = e.currentTarget.checked), void saveShow(on).catch(() => {}))} />
  <span>{locale.t("complete-setting")}</span>
</label>

<style>
  /* As the settings' other choices. */
  .choice {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 5px 12px;
    margin-top: 8px;
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
</style>
