<script lang="ts">
  import Icon from "./Icon.svelte";
  import { locale } from "../i18n.svelte";

  // «A new version of Zajil is available» (#124): a bar at the top of the
  // main window that does not stop the work. The release page opens from
  // Rust (`onopen`), never from a link in the page; closing hides it until
  // the next start.
  let { version, onopen, onclose }: { version: string; onopen: () => void; onclose: () => void } = $props();
</script>

<div class="update" role="status">
  <Icon name="download" />
  <span class="text">{locale.t("update-available")} <bdi dir="ltr" class="version">{version}</bdi></span>
  <button class="btn" onclick={onopen}><Icon name="external-link" />{locale.t("update-release-page")}</button>
  <button class="btn icon quiet" aria-label={locale.t("update-dismiss")} title={locale.t("update-dismiss")} onclick={onclose}><Icon name="x" /></button>
</div>

<style>
  .update {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 12px;
    background: var(--sel);
    border-bottom: 1px solid var(--line);
    font-size: 13px;
  }
  .text {
    flex: 1;
    min-width: 0;
  }
  .version {
    font-weight: 600;
  }
</style>
