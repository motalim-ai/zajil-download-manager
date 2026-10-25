<script lang="ts">
  import { onMount } from "svelte";
  import UpdateNotice from "./UpdateNotice.svelte";
  import { updateApi } from "../updateApi";

  // The main window's «new version» bar (#124): what the check at start
  // found, now or before this page opened; hidden until the next start.
  let version = $state<string | null>(null);
  let hidden = $state(false);

  onMount(() => {
    let unlisten: (() => void) | undefined;
    void (async () => {
      unlisten = await updateApi.onFound((found) => (version = found.version)).catch(() => undefined);
      const found = await updateApi.notice().catch(() => null);
      if (found) version = found.version;
    })();
    return () => unlisten?.();
  });
</script>

{#if version && !hidden}
  <UpdateNotice {version} onopen={() => void updateApi.openReleasePage().catch(() => {})} onclose={() => (hidden = true)} />
{/if}
