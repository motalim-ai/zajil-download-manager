<script lang="ts">
  import { onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import AddDialog from "../lib/components/AddDialog.svelte";
  import { api } from "../lib/api";
  import { startWindow } from "../lib/windowStart";
  import type { ProbeView } from "../lib/bindings/ProbeView";
  import type { AddRequest } from "../lib/bindings/AddRequest";
  import type { QueueView } from "../lib/bindings/QueueView";
  import type { CategoryView } from "../lib/bindings/CategoryView";

  // The «Download file info» window: asks the server about the link, then
  // adds the download as the user says, and closes. Closing it gives up a
  // check still running (Rust).
  let url = $state("");
  let probe = $state<ProbeView | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let queues = $state<QueueView[]>([]);
  let categories = $state<CategoryView[]>([]);

  onMount(() => {
    void (async () => {
      await startWindow();
      url = (await api.pendingInfo().catch(() => null)) ?? "";
      try {
        const snapshot = await api.snapshot();
        queues = snapshot.queues;
        categories = snapshot.categories;
      } catch {
        queues = [{ name: "main", max_active: 1, active: 0, window: null }];
      }
      try {
        probe = await api.probe(url);
      } catch (e) {
        error = String(e);
      }
    })();
  });

  async function add(request: AddRequest, remember: boolean) {
    if (busy) return;
    busy = true;
    error = null;
    try {
      await api.add(request);
      // The folder becomes the category's: the next file of that kind goes
      // there.
      if (remember && probe?.category && request.folder !== null) {
        const id = probe.category;
        const folder = request.folder;
        await api.saveCategories(categories.map((c) => (c.id === id ? { ...c, folder } : c)));
      }
      await api.closeWindow();
    } catch (e) {
      busy = false;
      error = String(e);
    }
  }

  async function chooseFolder(): Promise<string | null> {
    const chosen = await openDialog({ directory: true, multiple: false }).catch(() => null);
    return typeof chosen === "string" ? chosen : null;
  }
</script>

<AddDialog
  {url}
  {probe}
  {error}
  {busy}
  {queues}
  {categories}
  onadd={(r, remember) => void add(r, remember)}
  oncancel={() => void api.closeWindow()}
  onchoosefolder={chooseFolder}
  onnewcategory={() => void api.showMainView("categories")}
/>
