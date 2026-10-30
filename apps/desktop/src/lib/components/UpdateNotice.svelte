<script lang="ts">
  import Icon from "./Icon.svelte";
  import { locale } from "../i18n.svelte";

  // «A new version of Zajil is available» (#124): a bar at the top of the
  // main window that does not stop the work. With a signed update
  // (ADR-0011): «Update now» installs it (the downloads save their progress
  // and Zajil opens again); else the release page opens from Rust
  // (`onopen`), never from a link in the page. «Skip this version» hides
  // it for good, closing until the next start.
  let {
    version,
    canInstall,
    progress,
    failed,
    oninstall,
    onopen,
    onskip,
    onclose,
  }: {
    version: string;
    canInstall: boolean;
    /** Undefined: not downloading; null: downloading, size unknown. */
    progress: number | null | undefined;
    failed: boolean;
    oninstall: () => void;
    onopen: () => void;
    onskip: () => void;
    onclose: () => void;
  } = $props();
  const busy = $derived(progress !== undefined);
</script>

<div class="update" role="status">
  <Icon name="download" />
  {#if busy}
    <span class="text">
      {locale.t("update-downloading")}
      {#if progress !== null && progress !== undefined}<bdi dir="ltr" class="version">{Math.round(progress * 100)}%</bdi>{/if}
    </span>
  {:else}
    <span class="text">
      {#if failed}{locale.t("update-failed")}{:else}{locale.t("update-available")} <bdi dir="ltr" class="version">{version}</bdi>{/if}
    </span>
    {#if canInstall}
      <button class="btn primary" title={locale.t("update-install-hint")} onclick={oninstall}><Icon name="download" />{locale.t("update-install")}</button>
    {:else}
      <button class="btn" onclick={onopen}><Icon name="external-link" />{locale.t("update-release-page")}</button>
    {/if}
    <button class="btn quiet" onclick={onskip}>{locale.t("update-skip")}</button>
    <button class="btn icon quiet" aria-label={locale.t("update-dismiss")} title={locale.t("update-dismiss")} onclick={onclose}><Icon name="x" /></button>
  {/if}
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
