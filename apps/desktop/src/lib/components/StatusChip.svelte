<script lang="ts">
  import Icon from "./Icon.svelte";
  import { locale } from "../i18n.svelte";
  import type { StatusView } from "../bindings/StatusView";
  /// `browser`: it waits for its browser's cookies after a restart (M5).
  /// `held`: a file that can run code, waiting for the user (review #84).
  let { status, browser = false, held = false }: { status: StatusView; browser?: boolean; held?: boolean } = $props();
  const icons: Record<StatusView, string> = {
    active: "down",
    waiting: "clock",
    paused: "pause",
    done: "check",
    failed: "alert",
  };
</script>

{#if held}
  <span class="chip paused"><Icon name="alert" size={14} />{locale.t("status-needs-ok")}</span>
{:else if browser}
  <span class="chip paused"><Icon name="link" size={14} />{locale.t("status-open-browser")}</span>
{:else}
  <span class="chip {status}"><Icon name={icons[status]} size={14} />{locale.t(`status-${status}`)}</span>
{/if}

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12.5px;
    padding: 1px 9px 1px 8px;
    border-radius: 99px;
    white-space: nowrap;
  }
  .active {
    /* Toward the ink: 4.5:1 or more on a selected row too (axe, review
       of #57); in the dark themes the ink is light, so it stays light. */
    color: color-mix(in oklab, var(--accent) 72%, var(--ink));
    background: color-mix(in oklab, var(--feather) 14%, transparent);
  }
  .done {
    color: var(--green);
    background: var(--green-bg);
  }
  .paused {
    color: var(--amber);
    background: var(--amber-bg);
  }
  .failed {
    color: var(--red);
    background: var(--red-bg);
  }
  .waiting {
    color: var(--muted);
    background: var(--sunken);
  }
</style>
