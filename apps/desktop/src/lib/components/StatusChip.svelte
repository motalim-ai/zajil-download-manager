<script lang="ts">
  import Icon from "./Icon.svelte";
  import { locale } from "../i18n.svelte";
  import type { StatusView } from "../bindings/StatusView";
  let { status }: { status: StatusView } = $props();
  const icons: Record<StatusView, string> = {
    active: "down",
    waiting: "clock",
    paused: "pause",
    done: "check",
    failed: "alert",
  };
</script>

<span class="chip {status}"><Icon name={icons[status]} size={14} />{locale.t(`status-${status}`)}</span>

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
    color: var(--accent);
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
