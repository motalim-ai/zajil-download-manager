<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "./Icon.svelte";
  import { api } from "../api";
  import { locale } from "../i18n.svelte";

  // The main window's «a video waits for its quality» bar (the owner's
  // notes on 1.0): while «Video quality» is open, it says so and brings
  // that window to the front.
  let open = $state(false);

  onMount(() => {
    let unlisten: (() => void) | undefined;
    void (async () => {
      unlisten = await api.onVideoWindow((now) => (open = now)).catch(() => undefined);
      open = await api.videoWindowOpen().catch(() => false);
    })();
    return () => unlisten?.();
  });
</script>

{#if open}
  <div class="waits" role="status">
    <Icon name="film" />
    <span class="text">{locale.t("video-waits")}</span>
    <button class="btn" onclick={() => void api.showVideoWindow().catch(() => {})}>{locale.t("video-waits-show")}</button>
  </div>
{/if}

<style>
  .waits {
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
</style>
