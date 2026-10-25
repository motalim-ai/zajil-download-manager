<script lang="ts">
  import { onMount, tick } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
  import Tree, { type Filter, type View } from "./lib/components/Tree.svelte";
  import Toolbar, { type ToolAction } from "./lib/components/Toolbar.svelte";
  import AddLink from "./lib/components/AddLink.svelte";
  import { isMod } from "./lib/keys";
  import Settings from "./lib/components/Settings.svelte";
  import Categories from "./lib/components/Categories.svelte";
  import DownloadTable from "./lib/components/DownloadTable.svelte";
  import AddDialog from "./lib/components/AddDialog.svelte";
  import Details from "./lib/components/Details.svelte";
  import RefreshDialog from "./lib/components/RefreshDialog.svelte";
  import { asRefreshProblem, refreshText } from "./lib/refresh";
  import Icon from "./lib/components/Icon.svelte";
  import { api } from "./lib/api";
  import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
  import { locale, type Lang, type Digits } from "./lib/i18n.svelte";
  import { num, size, speed } from "./lib/format";
  import type { Snapshot } from "./lib/bindings/Snapshot";
  import type { DetailsView } from "./lib/bindings/DetailsView";
  import type { ProbeView } from "./lib/bindings/ProbeView";
  import type { AddRequest } from "./lib/bindings/AddRequest";
  import type { SettingsView } from "./lib/bindings/SettingsView";
  import type { SettingsChange } from "./lib/bindings/SettingsChange";
  import type { CategoryView } from "./lib/bindings/CategoryView";

  let snapshot = $state<Snapshot>({ downloads: [], queues: [], categories: [], speed_limit: null, problem: null });
  let filter = $state<Filter>({ kind: "all" });
  // The selected downloads, in table order (M5.5: several at once).
  let selected = $state<number[]>([]);
  let details = $state<DetailsView | null>(null);
  let search = $state("");
  // The small «add a link» dialog, and whether its link came from the
  // clipboard.
  let linking = $state<{ url: string; fromClipboard: boolean } | null>(null);
  let adding = $state<{ url: string; probe: ProbeView | null; error: string | null; busy: boolean } | null>(null);
  // The "refresh the download link" dialog (M5): its download, and why the
  // last try was refused.
  let refreshing = $state<{ id: number; name: string; problem: string | null; detail: string | null; busy: boolean } | null>(null);
  let error = $state<string | null>(null);
  // What the last toolbar command answered, if it failed.
  let failed = $state<string | null>(null);
  let view = $state<View>("list");
  let settings = $state<SettingsView | null>(null);
  let settingsMessage = $state<{ text: string; problem: boolean } | null>(null);
  // Why the settings could not be read (a damaged settings.json or
  // queues.json): said on the settings screen, never overwritten by a
  // refresh (review of #57: it went into `error`, which the next snapshot
  // cleared).
  let settingsProblem = $state<string | null>(null);
  let keepFolders = $state(false);
  // Said to screen readers when a download ends: not each progress event.
  let announcement = $state("");
  let lastStatus = new Map<number, string>();
  // The names test page, loaded only in a ui-test build.
  let NamesPage = $state<typeof import("./uitest/Names.svelte").default | null>(null);

  // What the tree picks, then what the search finds in the name or the
  // site (Ctrl+F).
  const shown = $derived.by(() => {
    const words = search.trim().toLocaleLowerCase();
    return snapshot.downloads.filter((d) => {
      const picked = (() => {
        switch (filter.kind) {
          case "all":
            return true;
          case "category":
            return d.category === filter.id && (filter.part === undefined || (filter.part === "finished") === (d.status === "done"));
          case "uncategorized":
            return d.category === null;
          case "queue":
            return d.queue === filter.name;
        }
      })();
      return picked && (!words || d.name.toLocaleLowerCase().includes(words) || (d.host ?? "").toLocaleLowerCase().includes(words));
    });
  });
  // The toolbar acts only on downloads the user sees: one filtered out or
  // gone is no longer selected (review of #46: Delete removed a hidden one).
  $effect(() => {
    const seen = new Set(shown.map((d) => d.id));
    if (selected.some((id) => !seen.has(id))) selected = selected.filter((id) => seen.has(id));
  });
  const active = $derived(snapshot.downloads.filter((d) => d.status === "active"));
  const waiting = $derived(snapshot.downloads.filter((d) => d.status === "waiting").length);
  const totalSpeed = $derived(active.reduce((sum, d) => sum + d.speed, 0));

  $effect(() => {
    // The window title follows the language.
    void getCurrentWindow().setTitle(locale.t("app-name")).catch(() => {});
  });

  /// Language, digits and theme, at once, without a restart.
  function applySettings(next: SettingsView) {
    settings = next;
    locale.set(next.language as Lang);
    locale.digits = next.digits as Digits;
    setTheme(next.theme);
    // The tray menu speaks the same language.
    void api.setTrayTexts(locale.t("tray-open"), locale.t("tray-quit")).catch(() => {});
  }

  async function changeSettings(change: SettingsChange) {
    try {
      applySettings(await api.saveSettings(change));
      settingsMessage = null;
      await refresh();
    } catch (e) {
      settingsMessage = { text: String(e), problem: true };
    }
  }

  async function chooseFolder(): Promise<string | null> {
    const chosen = await openDialog({ directory: true, multiple: false });
    return typeof chosen === "string" ? chosen : null;
  }

  async function exportQueues() {
    const path = await saveDialog({ defaultPath: "zajil-queues.json", filters: [{ name: "JSON", extensions: ["json"] }] });
    if (!path) return;
    try {
      await api.exportQueues(path);
      settingsMessage = { text: locale.t("export-done"), problem: false };
    } catch (e) {
      settingsMessage = { text: String(e), problem: true };
    }
  }

  async function importQueues() {
    const path = await openDialog({ multiple: false, filters: [{ name: "JSON", extensions: ["json"] }] });
    if (typeof path !== "string") return;
    try {
      const done = await api.importQueues(path, keepFolders);
      const kept = done.kept_settings.length
        ? " " + locale.t("import-kept", { names: done.kept_settings.join("، ") })
        : "";
      settingsMessage = { text: locale.t("import-done", { count: done.items_added }) + kept, problem: false };
      await refresh();
    } catch (e) {
      settingsMessage = { text: String(e), problem: true };
    }
  }

  async function saveCategories(next: CategoryView[]): Promise<string | null> {
    try {
      await api.saveCategories(next);
      await refresh();
      return null;
    } catch (e) {
      return String(e);
    }
  }

  /// A system notification for each download that just ended (the
  /// setting), once the system allows them.
  async function notify(said: string[]) {
    try {
      let allowed = await isPermissionGranted();
      if (!allowed) allowed = (await requestPermission()) === "granted";
      if (!allowed) return;
      for (const text of said) {
        const [title, ...rest] = text.split(": ");
        sendNotification({ title: title ?? text, body: rest.join(": ") });
      }
    } catch {
      // A notification that cannot be shown is not a problem of the download.
    }
  }

  /// Tells screen readers about downloads that just finished or failed.
  function announce(downloads: Snapshot["downloads"]) {
    const said: string[] = [];
    for (const d of downloads) {
      const before = lastStatus.get(d.id);
      if (before && before !== d.status && (d.status === "done" || d.status === "failed")) {
        said.push(`${locale.t(d.status === "done" ? "notify-done-title" : "notify-failed-title")}: ${d.name}`);
      }
      lastStatus.set(d.id, d.status);
    }
    if (said.length) {
      announcement = said.join(". ");
      if (settings?.notifications) void notify(said);
    }
  }

  // Snapshots can come back out of order: only the latest asked counts
  // (review of #45: an older one could win and stay).
  let asked = 0;
  async function refresh() {
    const mine = ++asked;
    try {
      const next = await api.snapshot();
      if (mine !== asked) return;
      snapshot = next;
      announce(snapshot.downloads);
      error = snapshot.problem;
      // The details follow the list: closed when their download is gone
      // (review of #48: they stayed, stale, and every refresh failed), and
      // an answer that comes after Back, or after a newer refresh, is
      // dropped (it reopened them).
      const id = details?.download.id;
      if (id !== undefined) {
        if (!next.downloads.some((d) => d.id === id)) details = null;
        else {
          const fresh = await api.details(id);
          if (mine === asked && details?.download.id === id) details = fresh;
        }
      }
    } catch (e) {
      if (mine === asked) error = String(e);
    }
  }

  async function startAdding(url: string) {
    if (adding) return; // one dialog at a time
    adding = { url, probe: null, error: null, busy: false };
    try {
      const probe = await api.probe(url);
      if (adding?.url === url) adding = { ...adding, probe, error: null };
    } catch (e) {
      if (adding?.url === url) adding = { ...adding, probe: null, error: String(e) };
    }
  }

  // Closed: a probe still running is given up, and the focus goes back to
  // the «add a link» button (review of #48).
  function closeAdding() {
    adding = null;
    void api.cancelProbe().catch(() => {});
    void tick().then(() => document.getElementById("tb-add")?.focus());
  }

  function closeLinking() {
    linking = null;
    void tick().then(() => document.getElementById("tb-add")?.focus());
  }

  async function add(request: AddRequest) {
    if (!adding || adding.busy) return;
    adding = { ...adding, busy: true };
    try {
      await api.add(request);
      closeAdding();
      await refresh();
    } catch (e) {
      if (adding) adding = { ...adding, busy: false, error: String(e) };
    }
  }

  async function open(id: number) {
    selected = [id];
    failed = null;
    try {
      details = await api.details(id);
    } catch (e) {
      failed = String(e);
    }
  }

  async function withoutCookies(id: number | null) {
    if (id === null) return;
    failed = null;
    try {
      await api.goWithoutCookies(id);
    } catch (e) {
      failed = String(e);
    }
    await refresh();
  }

  function startRefreshing() {
    const d = details?.download;
    if (d && !refreshing) refreshing = { id: d.id, name: d.name, problem: null, detail: null, busy: false };
  }

  // Closed: a refresh still asking the server is given up, and the focus
  // goes back to the details' buttons (review of #78).
  function closeRefreshing() {
    if (refreshing?.busy) void api.cancelRefresh(refreshing.id).catch(() => {});
    refreshing = null;
    void tick().then(() => {
      const back = ["details-refresh", "details-resume", "details-pause", "details-back"].map((id) => document.getElementById(id)).find((b) => b);
      back?.focus();
    });
  }

  async function refreshLink(url: string) {
    if (!refreshing || refreshing.busy) return;
    const id = refreshing.id;
    refreshing = { ...refreshing, busy: true };
    try {
      await api.refreshLink(id, url);
      if (refreshing?.id === id) {
        refreshing = { ...refreshing, busy: false };
        await refresh();
        closeRefreshing();
        return;
      }
    } catch (e) {
      const said = refreshText(locale.t, locale.lang, locale.digits, asRefreshProblem(e));
      if (refreshing?.id === id) refreshing = { ...refreshing, busy: false, problem: said.text, detail: said.detail };
    }
    await refresh();
  }

  // A command that fails is said, and the list is asked for again either
  // way, so it shows what really happened (review of #46).
  // `id`: the details' own download, whatever the list selects (review of
  // #48: a sidebar click cleared the selection, and Resume did nothing).
  async function act(kind: "pause" | "resume" | "remove", ids: number[] | number | null = selected) {
    const list = ids === null ? [] : Array.isArray(ids) ? ids : [ids];
    if (list.length === 0) return;
    failed = null;
    try {
      await api[kind](list);
    } catch (e) {
      failed = String(e);
    }
    await refresh();
  }

  // The queue the queue buttons act on: the one the tree shows, else the
  // main one.
  const queueShown = $derived(filter.kind === "queue" ? filter.name : "main");
  const idsOf = (pick: (d: Snapshot["downloads"][number]) => boolean) => snapshot.downloads.filter(pick).map((d) => d.id);
  const chosen = $derived(snapshot.downloads.filter((d) => selected.includes(d.id)));

  /** Whether a toolbar button has something to act on. */
  function can(action: ToolAction): boolean {
    switch (action) {
      case "resume":
        return chosen.some((d) => d.status === "paused" || d.status === "failed");
      case "pause":
        return chosen.some((d) => d.status === "active" || d.status === "waiting");
      case "delete":
        return chosen.length > 0;
      case "pause-all":
        return snapshot.downloads.some((d) => d.status === "active" || d.status === "waiting");
      case "delete-done":
        return snapshot.downloads.some((d) => d.status === "done");
      case "run-queue":
        return snapshot.downloads.some((d) => d.queue === queueShown && (d.status === "paused" || d.status === "failed"));
      case "halt-queue":
        return snapshot.downloads.some((d) => d.queue === queueShown && (d.status === "active" || d.status === "waiting"));
      default:
        return true;
    }
  }

  function doAction(action: ToolAction) {
    switch (action) {
      case "add":
        linking = { url: "", fromClipboard: false };
        break;
      case "resume":
        void act("resume");
        break;
      case "pause":
        void act("pause");
        break;
      case "delete":
        void act("remove");
        break;
      case "pause-all":
        void act("pause", idsOf((d) => d.status === "active" || d.status === "waiting"));
        break;
      case "delete-done":
        void act("remove", idsOf((d) => d.status === "done"));
        break;
      case "run-queue":
        void act("resume", idsOf((d) => d.queue === queueShown && (d.status === "paused" || d.status === "failed")));
        break;
      case "halt-queue":
        void act("pause", idsOf((d) => d.queue === queueShown && (d.status === "active" || d.status === "waiting")));
        break;
      // The queues' schedules are in the settings until the schedule
      // window comes.
      case "settings":
      case "schedule":
        details = null;
        view = "settings";
        break;
    }
  }

  // Ctrl+F (⌘F) goes to the search box, unless a dialog is open.
  function onWindowKey(event: KeyboardEvent) {
    if (isMod(event, "f") && !adding && !linking && !refreshing) {
      event.preventDefault();
      view = "list";
      details = null;
      document.getElementById("search")?.focus();
    }
  }

  function setTheme(theme: string) {
    if (theme === "light" || theme === "dark") document.documentElement.dataset.theme = theme;
    else delete document.documentElement.dataset.theme;
  }

  /// `ui-test` builds only: show a fixed scenario (`<view>-<lang>-<theme>`)
  /// and tell the Rust side when it is on screen.
  async function runUiTest(): Promise<boolean> {
    if (!__UI_TEST__) return false;
    const test = await import("./uitest/api");
    const scenario = await test.uiTestScenario();
    if (!scenario) return false;
    const [screen = "list", lang = "ar", theme = "light"] = scenario.split("-");
    locale.set(lang as Lang);
    setTheme(theme);
    snapshot = await api.snapshot();
    // Two rows selected, as in the design.
    selected = [1, 2];
    if (screen === "settings" || screen === "switch") {
      const loaded = await api.settings();
      settings = { ...loaded, language: lang as SettingsView["language"], theme: theme as SettingsView["theme"] };
      if (screen === "settings") view = "settings";
    }
    if (screen === "categories") view = "categories";
    if (screen === "details") details = await api.details(1);
    // The failed download, its new link refused as another file's (M5).
    if (screen === "refresh") {
      details = await api.details(6);
      const problem = { kind: "not_the_same_file", why: "size", theirs: 15 * 1024 ** 2, ours: 14 * 1024 ** 2 } as const;
      const said = refreshText(locale.t, locale.lang, locale.digits, problem);
      refreshing = { id: 6, name: details.download.name, problem: said.text, detail: null, busy: false };
    }
    if (screen === "add")
      adding = {
        url: "https://releases.ubuntu.com/24.04.3/ubuntu-24.04.3-desktop-amd64.iso",
        probe: {
          name: "ubuntu-24.04.3-desktop-amd64.iso",
          size: 6.1 * 1024 ** 3,
          resumable: true,
          ranges: true,
          category: "programs",
          category_site: null,
          category_extension: "iso",
          folder: "~/Downloads",
        },
        error: null,
        busy: false,
      };
    if (screen === "names") NamesPage = (await import("./uitest/Names.svelte")).default;
    const settle = async () => {
      await tick();
      await document.fonts.ready;
      await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
    };
    await settle();
    // The language changes in the same session, through the settings'
    // own code path: the direction and the texts change, nothing restarts.
    let languageSwitch: unknown = undefined;
    if (screen === "switch" && settings) {
      const seen = () => ({ dir: document.documentElement.dir, lang: document.documentElement.lang, text: document.querySelector("nav")?.textContent ?? "" });
      const before = seen();
      await changeSettings({ ...settings, language: lang === "ar" ? "en" : "ar" });
      await settle();
      languageSwitch = { before, after: seen() };
    }
    if (screen !== "names") {
      // Automatic accessibility check of what is on screen (axe-core,
      // a test-only dependency loaded only in this build).
      const axe = (await import("axe-core")).default;
      const result = await axe.run(document, { resultTypes: ["violations"] });
      const accessibility = result.violations.map((v) => ({
        id: v.id,
        impact: v.impact,
        help: v.help,
        nodes: v.nodes.slice(0, 5).map((n) => n.target.join(" ")),
      }));
      await test.uiTestReport({ scenario, dpr: window.devicePixelRatio, window: await test.windowRect(), accessibility, languageSwitch });
    }
    return true;
  }

  onMount(() => {
    locale.set("ar");
    let unlisten: (() => void) | undefined;
    let unlistenClipboard: (() => void) | undefined;
    void (async () => {
      if (await runUiTest()) return;
      try {
        applySettings(await api.settings());
      } catch (e) {
        settingsProblem = String(e);
      }
      // Listening first, so a change during the first snapshot is not lost.
      unlisten = await api.onChange(() => void refresh());
      // A copied download link waits in the paste bar, and the window
      // shows; nothing asks the server before the user presses "Download"
      // (review of #67). Ignored while a dialog is open: with the refresh
      // dialog, the new link is copied for it (review of #78).
      unlistenClipboard = await api.onClipboardLink((url) => {
        if (adding || refreshing || linking) return;
        linking = { url, fromClipboard: true };
        void api.showWindow().catch(() => {});
      });
      await refresh();
    })();
    return () => {
      unlisten?.();
      unlistenClipboard?.();
    };
  });
</script>

<svelte:window onkeydown={onWindowKey} />

{#if NamesPage}
  <NamesPage />
{:else}
  <div class="window">
    <Toolbar bind:search {can} onaction={doAction} />
    <div class="body">
    <Tree {snapshot} bind:filter bind:view />
    <main class="main">
      {#if view === "settings" && !settings}
        <p class="problem" role="alert"><Icon name="alert" />{settingsProblem ?? "…"}</p>
      {:else if view === "settings" && settings}
        <Settings
          {settings}
          bind:keepFolders
          onchange={(c) => void changeSettings(c)}
          onchoosefolder={async () => {
            const folder = await chooseFolder();
            if (folder !== null && settings) void changeSettings({ ...settings, download_dir: folder });
          }}
          onexport={() => void exportQueues()}
          onimport={() => void importQueues()}
          message={settingsMessage}
        />
      {:else if view === "categories"}
        <Categories
          categories={snapshot.categories}
          onsave={saveCategories}
          onchoosefolder={chooseFolder}
          ontry={(c, name) => api.tryCategory(c, name)}
          problem={null}
        />
      {:else if details}
        {#if failed}<p class="problem" role="alert"><Icon name="alert" />{failed}</p>{/if}
        <Details
          {details}
          onback={() => (details = null)}
          onpause={() => void act("pause", details?.download.id ?? null)}
          onresume={() => void act("resume", details?.download.id ?? null)}
          onrefresh={startRefreshing}
          onwithoutcookies={() => void withoutCookies(details?.download.id ?? null)}
        />
      {:else}
        {#if error}<p class="problem" role="alert"><Icon name="alert" />{error}</p>{/if}
        {#if failed}<p class="problem" role="alert"><Icon name="alert" />{failed}</p>{/if}
        <DownloadTable downloads={shown} bind:selected onopen={(id) => void open(id)} />
      {/if}
      {#if adding}
        <AddDialog
          url={adding.url}
          probe={adding.probe}
          error={adding.error}
          busy={adding.busy}
          queues={snapshot.queues}
          categories={snapshot.categories}
          onadd={(r) => void add(r)}
          oncancel={closeAdding}
        />
      {/if}
      {#if linking}
        <AddLink
          url={linking.url}
          fromClipboard={linking.fromClipboard}
          onok={(url) => {
            linking = null;
            void startAdding(url);
          }}
          oncancel={closeLinking}
        />
      {/if}
      {#if refreshing}
        <RefreshDialog
          name={refreshing.name}
          problem={refreshing.problem}
          detail={refreshing.detail}
          busy={refreshing.busy}
          onrefresh={(url) => void refreshLink(url)}
          oncancel={closeRefreshing}
        />
      {/if}
    </main>
    </div>
    <footer class="status">
      <span>{locale.t("downloads-count", { count: snapshot.downloads.length })}</span>
      <span>{locale.t("footer-active", { count: active.length })}</span>
      <span>{locale.t("footer-total-speed", { speed: speed(locale.lang, locale.digits, totalSpeed) || size(locale.lang, locale.digits, 0) })}</span>
      <span>{snapshot.speed_limit ? locale.t("footer-limit", { speed: speed(locale.lang, locale.digits, snapshot.speed_limit) }) : locale.t("footer-limit-none")}</span>
      <span>{locale.t("footer-waiting", { count: waiting })}</span>
      {#if selected.length > 0}<span class="end">{locale.t("footer-selected", { number: num(locale.lang, locale.digits, selected.length, 0) })}</span>{/if}
    </footer>
    <div class="visually-hidden" aria-live="polite" role="status">{announcement}</div>
  </div>
{/if}

<style>
  .window {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .body {
    flex: 1;
    display: grid;
    grid-template-columns: 228px 1fr;
    min-height: 0;
  }
  .main {
    position: relative;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .problem {
    display: flex;
    gap: 8px;
    align-items: center;
    margin: 8px 16px 0;
    padding: 8px 12px;
    border-radius: 8px;
    background: var(--red-bg);
    color: var(--red);
  }
  .status {
    display: flex;
    gap: 18px;
    align-items: center;
    padding: 5px 14px;
    border-top: 1px solid var(--line);
    font-size: 12px;
    color: var(--muted);
    background: var(--surface);
  }
  .end {
    margin-inline-start: auto;
  }
</style>
