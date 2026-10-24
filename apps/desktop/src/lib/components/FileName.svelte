<script lang="ts">
  // A file name that mixes Arabic, Latin and numbers (rule in ../fname.ts).
  import { nameParts } from "../fname";
  let { name }: { name: string } = $props();
  const parts = $derived(nameParts(name));
</script>

<bdi dir="auto" class="fn">{parts.head}{#if parts.tail}<bdi dir="ltr">{parts.tail}</bdi>{/if}</bdi>

<style>
  .fn {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
