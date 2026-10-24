<script lang="ts">
  // The logo inside the window: assets/icon/zajil-mark.svg itself, in the
  // colour of the text around it (currentColor). Never redrawn.
  import mark from "../../../../../assets/icon/zajil-mark.svg?raw";
  let { size = 26 }: { size?: number } = $props();
</script>

<span class="mark" style:width="{size}px" style:height="{size}px" aria-hidden="true">{@html mark}</span>

<style>
  .mark {
    display: inline-flex;
    color: var(--accent);
    flex: none;
  }
  .mark :global(svg) {
    width: 100%;
    height: 100%;
  }
</style>
