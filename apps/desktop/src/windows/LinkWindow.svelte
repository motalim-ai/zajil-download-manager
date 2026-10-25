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
      // A link copied while it is open takes the box (listening first, so
      // one copied meanwhile is not lost).
      unlisten = await api.onPendingLink((link) => {
        pending = link;
        problem = null;
      });
      pending ??= (await api.pendingLink().catch(() => null)) ?? { url: "", from_clipboard: false };
    })();
    return () => unlisten?.();
  });

  async function ok(url: string) {
    problem = null;
    try {
      await api.openAddInfo(url, locale.t("info-title"));
    } catch (e) {
      problem = String(e) === "info-open" ? locale.t("addlink-info-open") : String(e);
    }
  }
</script>

{#if pending}
  <AddLink url={pending.url} fromClipboard={pending.from_clipboard} {problem} onok={(url) => void ok(url)} oncancel={() => void api.closeWindow()} />
{/if}
