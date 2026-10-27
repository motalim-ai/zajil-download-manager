<script lang="ts">
  import { onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import AddDialog from "../lib/components/AddDialog.svelte";
  import { api } from "../lib/api";
  import { startWindow } from "../lib/windowStart";
  import { locale } from "../lib/i18n.svelte";
  import type { ProbeView } from "../lib/bindings/ProbeView";
  import type { AddRequest } from "../lib/bindings/AddRequest";
  import type { QueueView } from "../lib/bindings/QueueView";
  import type { CategoryView } from "../lib/bindings/CategoryView";

  // The «Download file info» window: asks the server about the link, then
  // adds the download as the user says, and closes. Closing it gives up a
  // check still running (Rust). A download from the browser has a window
  // of its own (`offers.rs`): Zajil asks the server with the browser's
  // cookies, and the window waits for that answer.
  let url = $state("");
  let offer: number | null = null;
  let probe = $state<ProbeView | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  // Added: the buttons stay off, whatever comes next (review of #104: a
  // failed «remember» let a second click add it again).
  let added = false;
  // Started in the background (the setting «Start downloading while the
  // window is open»): its id, so the buttons act on it.
  let started = $state<number | null>(null);
  let queues = $state<QueueView[]>([]);
  let categories = $state<CategoryView[]>([]);

  onMount(() => {
    void (async () => {
      await startWindow();
      const start = await api.pendingInfo().catch(() => null);
      url = start?.url ?? "";
      offer = start?.offer ?? null;
      try {
        const snapshot = await api.snapshot();
        queues = snapshot.queues;
        categories = snapshot.categories;
      } catch {
        queues = [{ name: "main", max_active: 1, active: 0, window: null }];
      }
      // A file that can run code never starts before «Start download».
      let runsCode = false;
      try {
        if (offer !== null) {
          const answer = await api.offerAnswer(offer);
          probe = answer.probe;
          runsCode = answer.runs_code;
        } else probe = await api.probe(url);
      } catch (e) {
        error = String(e);
        return;
      }
      const prefs = await api.addPrefs().catch(() => null);
      if (prefs?.start_while_open && !probe.page && !runsCode) {
        const request = { url, name: null, category: null, description: null, queue: "main", folder: null, start: true, limit: null };
        try {
          started = offer !== null ? await api.addOffer(offer, request, true) : await api.add(request);
          void api.openStartedProgress(started, probe.name).catch(() => {});
        } catch {
          started = null;
        }
      }
    })();
  });

  async function add(request: AddRequest, remember: string | null) {
    if (busy || added) return;
    busy = true;
    error = null;
    try {
      if (started !== null) {
        // Running already: «Start download» keeps it as the user leaves
        // the category and description; «Download later» pauses it and
        // moves it to the chosen queue.
        await api.setItemDetails(started, request.category, request.description);
        if (!request.start) await api.pause([started]);
        if (request.queue && request.queue !== "main") await api.moveToQueue([started], request.queue);
        added = true;
      } else {
        // From the browser, «Start download» is the user's OK for a file
        // that can run code: it is not held again.
        const id = offer !== null ? await api.addOffer(offer, request, false) : await api.add(request);
        added = true;
        // A download that starts shows its progress, as in IDM (as the
        // settings say: shown, minimized, or not).
        if (request.start) void api.openStartedProgress(id, probe?.name ?? "").catch(() => {});
      }
    } catch (e) {
      busy = false;
      error = String(e);
      return;
    }
    // The folder becomes the category's: the next file of that kind goes
    // there. The categories as they are now, not as they were when the
    // window opened (review of #104: one added meanwhile was lost).
    if (remember !== null && request.folder !== null) {
      const id = remember;
      const folder = request.folder;
      try {
        const now = (await api.snapshot()).categories;
        await api.saveCategories(now.map((c) => (c.id === id ? { ...c, folder } : c)));
      } catch (e) {
        error = locale.t("add-remember-failed", { error: String(e) });
        return;
      }
    }
    await api.closeWindow();
  }

  // «Cancel»: a download started in the background is taken off the list
  // and its unfinished files are deleted.
  async function cancel() {
    if (started !== null) await api.removeWithFiles([started]).catch(() => {});
    await api.closeWindow();
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
  oncancel={() => void cancel()}
  onchoosefolder={chooseFolder}
  onnewcategory={() => void api.showMainView("categories")}
  started={started !== null}
/>
