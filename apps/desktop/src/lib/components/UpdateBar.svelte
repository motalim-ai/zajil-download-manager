<script lang="ts">
  import { onMount } from "svelte";
  import UpdateNotice from "./UpdateNotice.svelte";
  import { updateApi } from "../updateApi";
  import type { UpdateView } from "../bindings/UpdateView";

  // The main window's «new version» bar (#124): what the check at start
  // found, now or before this page opened; hidden until the next start.
  // A signed update (ADR-0011) installs from here.
  let found = $state<UpdateView | null>(null);
  let hidden = $state(false);
  // While it downloads: the share done (0 to 1), or null if its size is unknown.
  let progress = $state<number | null | undefined>(undefined);
  let failed = $state(false);

  onMount(() => {
    const unlisten: (() => void)[] = [];
    void (async () => {
      unlisten.push(await updateApi.onFound((view) => (found = view)).catch(() => () => {}));
      unlisten.push(
        await updateApi
          .onProgress((p) => (progress = p.total ? Math.min(1, p.done / p.total) : null))
          .catch(() => () => {}),
      );
      const view = await updateApi.notice().catch(() => null);
      if (view) found = view;
    })();
    return () => unlisten.forEach((u) => u());
  });

  async function install() {
    failed = false;
    progress = null;
    try {
      // Zajil opens again once it is installed.
      await updateApi.install();
    } catch {
      progress = undefined;
      failed = true;
    }
  }

  async function skip() {
    await updateApi.skip().catch(() => {});
    hidden = true;
  }
</script>

{#if found && !hidden}
  <UpdateNotice
    version={found.version}
    canInstall={found.can_install && !failed}
    {progress}
    {failed}
    oninstall={() => void install()}
    onopen={() => void updateApi.openReleasePage().catch(() => {})}
    onskip={() => void skip()}
    onclose={() => (hidden = true)}
  />
{/if}
