<script lang="ts">
  // The progress of a download: one feather for each part (connection), as
  // in the wings of the icon. The label says it in words.
  import type { StatusView } from "../bindings/StatusView";
  let { parts, status, label, width = 150 }: { parts: number[]; status: StatusView; label: string; width?: number } =
    $props();
  const shown = $derived(parts.length > 8 ? parts.slice(0, 8) : parts.length ? parts : [0]);
</script>

<span class="feathers {status}" style:inline-size="{width}px" role="img" aria-label={label}>
  {#each shown as part, i (i)}
    <i><b style:inline-size="{Math.round(Math.min(Math.max(part, 0), 1) * 100)}%"></b></i>
  {/each}
</span>

<style>
  .feathers {
    display: inline-flex;
    gap: 3px;
    align-items: center;
  }
  i {
    flex: 1;
    height: 7px;
    border-radius: 99px;
    background: var(--feather-bg);
    position: relative;
    overflow: hidden;
  }
  b {
    position: absolute;
    inset-block: 0;
    inset-inline-start: 0;
    background: linear-gradient(to left, var(--feather), var(--feather-2));
    border-radius: inherit;
  }
  :global([dir="ltr"]) b {
    background: linear-gradient(to right, var(--feather), var(--feather-2));
  }
  .done b {
    background: var(--green-fill);
  }
  .paused b {
    background: var(--amber-fill);
  }
  .failed b {
    background: var(--red);
  }
</style>
