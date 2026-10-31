<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import Icon from "./Icon.svelte";
  import { locale } from "../i18n.svelte";

  // The browser extension is older than this Zajil needs (owner's request,
  // 2026-10-31): it does not update itself. A bar at the top of the main
  // window, as the update's; «How to update it» opens the newest release's
  // page from Rust. Closed, it stays hidden until the next start.
  let version = $state<string | null>(null);
  let hidden = $state(false);

  onMount(() => {
    let unlisten: (() => void) | undefined;
    void (async () => {
      unlisten = await listen<string>("extension-old", (e) => (version = e.payload)).catch(() => undefined);
      const said = await invoke<string | null>("extension_old_notice").catch(() => null);
      if (said) version = said;
    })();
    return () => unlisten?.();
  });
</script>

{#if version && !hidden}
  <div class="bar" role="status">
    <Icon name="globe" />
    <span class="text">{locale.t("extension-old")} <bdi dir="ltr" class="version">{version}</bdi></span>
    <button class="btn" onclick={() => void invoke("open_extension_help").catch(() => {})}><Icon name="external-link" />{locale.t("extension-old-help")}</button>
    <button class="btn icon quiet" aria-label={locale.t("update-dismiss")} title={locale.t("update-dismiss")} onclick={() => (hidden = true)}><Icon name="x" /></button>
  </div>
{/if}

<style>
  .bar {
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
