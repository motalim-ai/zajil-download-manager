<script lang="ts">
  // The file's bytes as squares (from design C, in the details only): each
  // part fills from where it starts; the square being written is lighter.
  import type { SegmentView } from "../bindings/SegmentView";
  let { segments, total, label }: { segments: SegmentView[]; total: number; label: string } = $props();
  const CELLS = 512;
  const cells = $derived.by(() => {
    const out: ("on" | "head" | "")[] = [];
    if (total <= 0) return out;
    const per = total / CELLS;
    for (let c = 0; c < CELLS; c++) {
      const from = c * per;
      const to = from + per;
      const seg = segments.find((s) => from >= s.start && from <= (s.end ?? total - 1));
      if (!seg) {
        out.push("");
        continue;
      }
      const written = seg.start + seg.done;
      out.push(written >= to ? "on" : written > from ? "head" : "");
    }
    return out;
  });
</script>

<div class="map" role="img" aria-label={label}>
  {#each cells as kind, i (i)}<i class={kind}></i>{/each}
</div>

<style>
  .map {
    display: grid;
    grid-template-columns: repeat(64, 1fr);
    gap: 2px;
  }
  i {
    aspect-ratio: 1;
    border-radius: 2px;
    background: var(--feather-bg);
  }
  .on {
    background: var(--feather);
  }
  .head {
    background: var(--feather-2);
  }
</style>
