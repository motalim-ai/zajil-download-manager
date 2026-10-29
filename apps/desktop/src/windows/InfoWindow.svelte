<script lang="ts">
  import { onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import AddDialog from "../lib/components/AddDialog.svelte";
  import { api } from "../lib/api";
  import { startWindow } from "../lib/windowStart";
  import { showFitted } from "../lib/fitWindow";
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
  let connections = $state(8);
  let maxConnections = $state(32);
  /** How long the window waits for the server's answer before it shows
   * with what the link's address says. */
  const QUICK_ANSWER_MS = 150;

  // What the link's address says before the server answers: every row is
  // drawn with it from the first frame, and the answer fills them in (the
  // owner's notes: the window opened small and grew once the server
  // answered).
  let guess = $state<ProbeView | null>(null);
  // The server's answer on its way, and the download started in the
  // background on its way: «Start download» before them waits for them.
  let answered: Promise<unknown> = Promise.resolve();
  let starting: Promise<unknown> = Promise.resolve();

  onMount(() => {
    void (async () => {
      await startWindow();
      const start = await api.pendingInfo().catch(() => null);
      url = start?.url ?? "";
      offer = start?.offer ?? null;
      void api.settings().then(
        (s) => ((connections = s.connections), (maxConnections = s.max_connections)),
        () => {},
      );
      // A file that can run code never starts before «Start download».
      let runsCode = false;
      const asked = (async () => {
        if (offer !== null) {
          const answer = await api.offerAnswer(offer);
          runsCode = answer.runs_code;
          return answer.probe;
        }
        return api.probe(url);
      })();
      answered = asked.catch(() => {});
      const [guessed, snapshot] = await Promise.all([
        api.guessInfo(url).catch(() => null),
        api.snapshot().catch(() => null),
      ]);
      guess = guessed;
      queues = snapshot?.queues ?? [{ name: "main", max_active: 1, active: 0, window: null }];
      categories = snapshot?.categories ?? [];
      // A server that answers at once is shown with its answer; one that
      // takes longer is not waited for.
      const quick = await Promise.race([asked.then((p) => p, () => null), new Promise<null>((r) => setTimeout(() => r(null), QUICK_ANSWER_MS))]);
      if (quick) probe = quick;
      await showFitted();
      try {
        probe = await asked;
      } catch (e) {
        error = String(e);
        return;
      }
      // The user added it before the server answered: it is not started
      // a second time.
      if (busy || added) return;
      const prefs = await api.addPrefs().catch(() => null);
      if (busy || added) return;
      if ((prefs?.start_at_once ?? true) && !probe.page && !runsCode) {
        const request = { url, name: null, category: null, description: null, queue: "main", folder: null, start: true, limit: null, connections: null };
        starting = (async () => {
          try {
            const id = offer !== null ? await api.addOffer(offer, request, true) : await api.add(request);
            // Held by this window until the user says where it goes: closing
            // it removes the download with its files (Rust).
            await api.holdStarted(id);
            started = id;
          } catch {
            started = null;
          }
        })();
      }
    })();
  });

  async function add(request: AddRequest, remember: string | null) {
    if (busy || added) return;
    busy = true;
    error = null;
    // A browser's download is added once Zajil has its answer; one started
    // in the background meanwhile is placed, not added again.
    if (offer !== null) await answered;
    await starting;
    try {
      if (started !== null) {
        // Running already: it moves to the folder and name the user chose
        // and runs on («Start download»), or waits paused in the chosen
        // queue («Download later»).
        await api.placeStarted(request.folder, request.name, request.start, request.connections);
        await api.setItemDetails(started, request.category, request.description);
        if (request.queue && request.queue !== "main") await api.moveToQueue([started], request.queue);
        added = true;
        if (request.start) void api.openStartedProgress(started, request.name ?? probe?.name ?? "").catch(() => {});
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
  // with its files as the window closes (Rust).
  async function cancel() {
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
  {guess}
  {error}
  {busy}
  {queues}
  {categories}
  onadd={(r, remember) => void add(r, remember)}
  oncancel={() => void cancel()}
  onchoosefolder={chooseFolder}
  onnewcategory={() => void api.showMainView("categories")}
  started={started !== null}
  {connections}
  {maxConnections}
/>
