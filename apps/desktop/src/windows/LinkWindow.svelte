<script lang="ts">
  import { onMount } from "svelte";
  import AddLink from "../lib/components/AddLink.svelte";
  import { api } from "../lib/api";
  import { locale } from "../lib/i18n.svelte";
  import { startWindow } from "../lib/windowStart";
  import type { PendingLink } from "../lib/bindings/PendingLink";

  // The «Add a link» window: OK opens «Download file info» for the link,
  // and this one closes.
  let pending = $state<PendingLink | null>(null);
  let problem = $state<string | null>(null);

  onMount(() => {
    let unlisten: (() => void) | undefined;
    void (async () => {
      await startWindow();
      pending = (await api.pendingLink().catch(() => null)) ?? { url: "", from_clipboard: false };
      // A link copied while it is open takes the box.
      unlisten = await api.onPendingLink((link) => (pending = link));
    })();
    return () => unlisten?.();
  });

  async function ok(url: string) {
    try {
      await api.openAddInfo(url, locale.t("info-title"));
    } catch (e) {
      problem = String(e);
    }
  }
</script>

{#if pending}
  <AddLink url={pending.url} fromClipboard={pending.from_clipboard} onok={(url) => void ok(url)} oncancel={() => void api.closeWindow()} />
  {#if problem}<p class="visually-hidden" role="alert">{problem}</p>{/if}
{/if}
