<script lang="ts">
  import { onMount, tick } from "svelte";
  import { getCurrentWindow, ProgressBarStatus } from "@tauri-apps/api/window";
  import { ask, message, open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
  import Tree, { type Filter, type View } from "./lib/components/Tree.svelte";
  import Toolbar, { type ToolAction } from "./lib/components/Toolbar.svelte";
  import { onMac } from "./lib/keys";
  import { accelerator, commandFor, keyOf, type Command } from "./lib/commands";
  import { MENU_BAR, commandKey } from "./lib/menus";
  import MenuBar from "./lib/components/MenuBar.svelte";
  import TitleBar from "./lib/components/TitleBar.svelte";
  import ContextMenu from "./lib/components/ContextMenu.svelte";
  import DeleteAsk from "./lib/components/DeleteAsk.svelte";
  import Settings from "./lib/components/Settings.svelte";
  import Categories from "./lib/components/Categories.svelte";
  import DownloadTable from "./lib/components/DownloadTable.svelte";
  import AddDialog from "./lib/components/AddDialog.svelte";
  import AddLink from "./lib/components/AddLink.svelte";
  import ProgressWindow from "./windows/ProgressWindow.svelte";
  import Details from "./lib/components/Details.svelte";
  import RefreshDialog from "./lib/components/RefreshDialog.svelte";
  import { asRefreshProblem, refreshText } from "./lib/refresh";
  import Icon from "./lib/components/Icon.svelte";
  import { api } from "./lib/api";
  import { scheduleApi } from "./lib/scheduleApi";
  import { showTrayTip, trayTip } from "./lib/trayTip";
  import { setTheme } from "./lib/theme";
  import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
  import { locale, type Lang, type Digits } from "./lib/i18n.svelte";
  import UpdateBar from "./lib/components/UpdateBar.svelte";
  import { num, size, speed } from "./lib/format";
  import type { Snapshot } from "./lib/bindings/Snapshot";
  import type { DetailsView } from "./lib/bindings/DetailsView";
  import type { ProbeView } from "./lib/bindings/ProbeView";
  import type { SettingsView } from "./lib/bindings/SettingsView";
  import type { SettingsChange } from "./lib/bindings/SettingsChange";
  import type { CategoryView } from "./lib/bindings/CategoryView";

  let snapshot = $state<Snapshot>({ downloads: [], queues: [], categories: [], speed_limit: null, problem: null });
  let filter = $state<Filter>({ kind: "all" });
  // The selected downloads, in table order (M5.5: several at once).
  let selected = $state<number[]>([]);
  let details = $state<DetailsView | null>(null);
  let search = $state("");
  // The table's columns chooser, and the right-click menu where it opened.
  let chooser = $state(false);
  let rowMenu = $state<{ x: number; y: number } | null>(null);
  // The small «add a link» dialog, and whether its link came from the
  // clipboard.
  // The «add» UI test scene: «Download file info» drawn in this window
  // (the add windows are windows of their own, M5.5).
  let addScene = $state<{ url: string; probe: ProbeView } | null>(null);
  // The «progress» scene: the first demo download's progress window.
  let progressScene = $state(false);
  // The «link» scene: «Add a link» with a link from the clipboard.
  let linkScene = $state(false);
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
    void api.setTrayTexts(locale.t("tray-open"), locale.t("tray-drop-box"), locale.t("tray-quit"), locale.t("app-name"), locale.t("tray-still-running")).catch(() => {});
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
      showTrayTip(trayTip(next.downloads, locale.lang, locale.digits, locale.t));
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

  // A dialog is open: the toolbar waits (review of #96: a second dialog
  // lost the first one's link).
  // Deleting downloads that have not finished: which, and how many.
  let deleteAsk = $state<{ ids: number[]; unfinished: number } | null>(null);
  async function deleteAnswered(withFiles: boolean | null) {
    const ask = deleteAsk;
    deleteAsk = null;
    void tick().then(() => document.getElementById("tb-delete")?.focus());
    if (!ask || withFiles === null) return;
    if (!withFiles) return act("remove", ask.ids);
    failed = null;
    try {
      await api.removeWithFiles(ask.ids);
    } catch (e) {
      failed = String(e);
    }
    await refresh();
  }
  const dialogOpen = $derived(Boolean(refreshing || deleteAsk));
  // The table is on screen: what the selection buttons act on is seen
  // (review of #96: Delete removed rows hidden behind the settings).
  const tableShown = $derived(view === "list" && !details);
  // A download held for the user's OK (#84) starts only when it is chosen,
  // never with a whole queue (review of #96).
  const runnable = (d: Snapshot["downloads"][number]) => d.queue === queueShown && !d.needs_ok && (d.status === "paused" || d.status === "failed");

  /** Whether a toolbar button has something to act on. */
  function can(action: ToolAction): boolean {
    if (dialogOpen) return false;
    if (!tableShown && (action === "resume" || action === "pause" || action === "delete" || action === "delete-done")) return false;
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
        return shown.some((d) => d.status === "done");
      case "run-queue":
        return snapshot.downloads.some(runnable);
      case "halt-queue":
        return snapshot.downloads.some((d) => d.queue === queueShown && (d.status === "active" || d.status === "waiting"));
      default:
        return true;
    }
  }

  function doAction(action: ToolAction) {
    switch (action) {
      case "add":
        void api.openAddLink(null, false, locale.t("addlink-title")).catch((e) => (failed = String(e)));
        break;
      case "resume":
        void act("resume");
        break;
      case "pause":
        void act("pause");
        break;
      // A download that has not finished asks first, and whether its
      // unfinished file goes too (an IDM touch).
      case "delete": {
        const unfinished = chosen.filter((d) => d.status !== "done").length;
        if (unfinished > 0) deleteAsk = { ids: [...selected], unfinished };
        else void confirmThen(locale.t("confirm-delete", { count: selected.length }), selected.length > 1, () => act("remove"));
        break;
      }
      case "pause-all":
        void act("pause", idsOf((d) => d.status === "active" || d.status === "waiting"));
        break;
      // Only the finished downloads the table shows, and once the user
      // agrees (review of #96: every category's went, with no word).
      case "delete-done": {
        const done = shown.filter((d) => d.status === "done").map((d) => d.id);
        void confirmThen(locale.t("confirm-delete-done", { count: done.length }), true, () => act("remove", done));
        break;
      }
      case "run-queue":
        void act("resume", idsOf(runnable));
        break;
      case "halt-queue":
        void act("pause", idsOf((d) => d.queue === queueShown && (d.status === "active" || d.status === "waiting")));
        break;
      case "settings":
        details = null;
        view = "settings";
        break;
      // The schedule window (M5.5), a window of its own.
      case "schedule":
        void scheduleApi.open(locale.t("schedule-title")).catch((e) => (failed = String(e)));
        break;
    }
  }

  /** The toolbar's word on what a button acts on: the queue's name. */
  function hint(action: ToolAction): string | undefined {
    if (action !== "run-queue" && action !== "halt-queue") return undefined;
    const name = queueShown === "main" ? locale.t("queue-main") : queueShown;
    return `${locale.t(action === "run-queue" ? "tb-run-queue" : "tb-halt-queue")}: ${name}`;
  }

  /** Runs `then` once the user agrees to `question` (when `needed`). */
  async function confirmThen(question: string, needed: boolean, then: () => Promise<void>) {
    if (needed) {
      // The buttons in the user's language, not the system's Yes/No.
      const yes = await ask(question, {
        title: locale.t("app-name"),
        kind: "warning",
        okLabel: locale.t("confirm-remove"),
        cancelLabel: locale.t("confirm-cancel"),
      }).catch(() => false);
      if (!yes) return;
    }
    await then();
  }

  // The commands of the menus, the right-click menu and the keyboard
  // (M5.5): the toolbar's own, and the rest.
  const TOOL: Command[] = ["add", "resume", "pause", "pause-all", "delete", "delete-done", "run-queue", "halt-queue", "schedule", "settings"];
  const one = $derived(chosen.length === 1 ? chosen[0] : undefined);

  /** Whether `command` has something to act on now. */
  function canRun(command: Command): boolean {
    if (TOOL.includes(command)) return can(command as ToolAction);
    if (dialogOpen && command !== "quit" && command !== "about") return false;
    switch (command) {
      case "open":
      case "open-folder":
      case "progress":
      case "properties":
        return tableShown && one !== undefined;
      case "refresh-link":
        return tableShown && one !== undefined && one.status !== "done";
      case "copy-link":
        return tableShown && chosen.length > 0;
      // Back to the main queue: only what is in another one.
      case "remove-from-queue":
        return tableShown && chosen.some((d) => d.queue !== "main");
      default:
        return true;
    }
  }

  // The macOS menu and the page's keydown can both see one shortcut: the
  // second is the same press.
  let last = { command: "", at: 0 };
  function run(command: Command) {
    const now = performance.now();
    if (last.command === command && now - last.at < 300) return;
    last = { command, at: now };
    // In a text box, Select all is its text's, dialog or not (review of
    // #101: ⌘A goes through the macOS menu).
    const box = document.activeElement;
    if (command === "select-all" && (box instanceof HTMLInputElement || box instanceof HTMLTextAreaElement)) return box.select();
    if (!canRun(command)) return;
    if (TOOL.includes(command)) return doAction(command as ToolAction);
    switch (command) {
      case "import":
        return void importQueues();
      case "export":
        return void exportQueues();
      case "quit":
        return void quit();
      case "find":
        view = "list";
        details = null;
        return document.getElementById("search")?.focus();
      // In a text box, its text; elsewhere, every download the table shows.
      case "select-all":
        if (tableShown) selected = shown.map((d) => d.id);
        return;
      case "copy-link":
        return void copyLinks();
      case "remove-from-queue":
        return void moveTo("main", chosen.filter((d) => d.queue !== "main").map((d) => d.id));
      case "categories":
        details = null;
        view = "categories";
        return;
      case "columns":
        details = null;
        view = "list";
        chooser = true;
        return;
      case "open":
        return void openOne(one!.id);
      case "open-folder":
        return void showIn(one!.id, true);
      case "progress":
        return void showProgress(one!.id);
      case "properties":
        return void open(one!.id);
      case "refresh-link":
        return void open(one!.id).then(() => details && startRefreshing());
      case "about":
        return void about();
    }
  }

  /** A finished download opens; one still on its way shows its progress. */
  async function openOne(id: number) {
    const d = snapshot.downloads.find((x) => x.id === id);
    if (d?.status === "done") await showIn(id, false);
    else await showProgress(id);
  }
  /** The download's progress window (M5.5). */
  async function showProgress(id: number) {
    const d = snapshot.downloads.find((x) => x.id === id);
    await api.openProgress(id, d?.name ?? locale.t("progress-title")).catch((e) => (failed = String(e)));
  }

  let shownProgress: number | null | undefined = undefined;
  // The running downloads' progress on the taskbar (Windows) or the Dock
  // (macOS): how much of what they fetch has come, or nothing.
  $effect(() => {
    const sized = active.filter((d) => d.size !== null && d.size > 0);
    const all = sized.reduce((sum, d) => sum + (d.size ?? 0), 0);
    const got = sized.reduce((sum, d) => sum + d.done, 0);
    const shown = all > 0 ? Math.min(100, Math.floor((got / all) * 100)) : null;
    // Only when it changes (review of #111).
    if (shown === shownProgress) return;
    shownProgress = shown;
    void getCurrentWindow()
      .setProgressBar(shown === null ? { status: ProgressBarStatus.None } : { status: ProgressBarStatus.Normal, progress: shown })
      .catch(() => {});
  });
  // A file that can run code opens once the user agrees (#84; review of
  // #101: a double-click ran a program).
  async function showIn(id: number, folder: boolean) {
    failed = null;
    try {
      if (await api.openDownload(id, folder)) return;
      const name = snapshot.downloads.find((x) => x.id === id)?.name ?? "";
      const yes = await ask(locale.t("open-code-question", { name }), {
        title: locale.t("app-name"),
        kind: "warning",
        okLabel: locale.t("open-code-ok"),
        cancelLabel: locale.t("confirm-cancel"),
      }).catch(() => false);
      if (yes) await api.openDownload(id, folder, true);
    } catch (e) {
      failed = String(e);
    }
  }
  async function copyLinks() {
    failed = null;
    try {
      await api.copyLinks(selected);
    } catch (e) {
      failed = String(e);
    }
  }
  async function moveTo(queue: string, ids: number[] = selected) {
    failed = null;
    try {
      await api.moveToQueue(ids, queue);
    } catch (e) {
      failed = String(e);
    }
    await refresh();
  }
  /** Quits; with downloads running, once the user agrees (owner's
   * decision on #83). */
  let quitting = false;
  async function quit() {
    // A close that quits waits for this page's answer (`closing.rs`).
    void api.quitHeard().catch(() => {});
    // One question at a time (review of #105: a second ⌘Q asked again).
    if (quitting) return;
    const running = active.length;
    if (running > 0) {
      quitting = true;
      const yes = await ask(locale.t("quit-question", { count: running }), {
        title: locale.t("app-name"),
        kind: "warning",
        okLabel: locale.t("quit-ok"),
        cancelLabel: locale.t("confirm-cancel"),
      }).catch(() => false);
      quitting = false;
      if (!yes) return;
    }
    await api.quitApp();
  }

  async function about() {
    const version = await api.appVersion().catch(() => "");
    await message(locale.t("about-text", { version }), { title: locale.t("app-name"), kind: "info" }).catch(() => {});
  }

  // Copy (Ctrl+C, or the system's Copy on macOS) on the table is «Copy
  // link», unless the copy is a text box's own or of selected text. The app
  // writes the clipboard, so its monitoring does not offer the links back.
  function onCopy(event: ClipboardEvent) {
    const box = document.activeElement;
    if (box instanceof HTMLInputElement || box instanceof HTMLTextAreaElement || !canRun("copy-link")) return;
    if (window.getSelection()?.toString()) return;
    event.preventDefault();
    void copyLinks();
  }

  /** A command from the macOS menu: in a text box, the keys a text box
   * has (⌘⌫, ⌘O, ⌘I) do not act on the downloads (review of #101). */
  function fromMenu(command: Command) {
    const box = document.activeElement;
    const typing = box instanceof HTMLInputElement || box instanceof HTMLTextAreaElement || (box instanceof HTMLElement && box.isContentEditable);
    if (typing && command !== "select-all" && TEXT_KEYS.includes(command)) return;
    run(command);
  }

  // The shortcuts. In a text box, its own keys stay its own.
  const TEXT_KEYS: Command[] = ["select-all", "copy-link", "delete", "open", "properties"];
  function onWindowKey(event: KeyboardEvent) {
    if (event.defaultPrevented) return;
    const command = commandFor(event);
    if (!command) return;
    const target = event.target as HTMLElement | null;
    const typing = target?.closest("input, textarea, select, [contenteditable]") !== null && target !== null;
    if (typing && TEXT_KEYS.includes(command)) return;
    // Enter on a row is the table's; elsewhere it is the focused button's.
    if (command === "open" && keyOf("open")?.key === "Enter") return;
    // Ctrl+C copies through the copy event.
    if (command === "copy-link") return;
    event.preventDefault();
    run(command);
  }

  // The system's menu bar on macOS, in the page's language.
  $effect(() => {
    if (!onMac() || NamesPage) return;
    const t = locale.t;
    const menus = MENU_BAR.map((m) => ({
      label: t(m.key),
      edit: Boolean(m.edit),
      items: m.items.flatMap((item): { id: Command | null; label?: string; accelerator?: string | null }[] => {
        if (item === "-") return [{ id: null }];
        if (!("command" in item)) return [];
        const key = keyOf(item.command, true);
        // ⌘C is the system's Copy, which «Copy link» answers.
        const shortcut = key && item.command !== "copy-link" ? accelerator(key) : null;
        return [{ id: item.command, label: t(commandKey(item.command)), accelerator: shortcut }];
      }),
    }));
    const system = Object.fromEntries(
      ["hide", "hide_others", "show_all", "undo", "redo", "cut", "copy", "paste", "window", "minimize", "zoom"].map((k) => [k, t(`sys-${k.replace("_", "-")}`, { app: t("app-name") })]),
    );
    void api.setMenu(t("app-name"), menus, system).catch(() => {});
  });



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
    // Dark: a Google Drive link as the owner tried it (a web page, no
    // size, the link's last part as its name), to see its warning, name
    // and size in words; light: an ordinary file.
    if (screen === "add" && theme === "dark")
      addScene = {
        url: "https://drive.google.com/uc?export=download&id=1AbC",
        probe: {
          name: "download",
          size: null,
          resumable: false,
          ranges: false,
          category: null,
          category_site: null,
          category_extension: null,
          folder: "~/Downloads",
          page: true,
          download_folder: "~/Downloads",
        },
      };
    else if (screen === "add")
      addScene = {
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
          page: false,
          download_folder: "~/Downloads",
        },
      };
    if (screen === "progress") progressScene = true;
    if (screen === "link") linkScene = true;
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
    let unlistenMenu: (() => void) | undefined;
    let unlistenView: (() => void) | undefined;
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
        if (refreshing) return;
        void api.openAddLink(url, true, locale.t("addlink-title")).catch(() => {});
      });
      // «New category» in «Download file info».
      unlistenView = await api.onMainView((next) => {
        if (next !== "categories") return;
        details = null;
        view = "categories";
      });
      unlistenMenu = await api.onMenuCommand((command) => fromMenu(command as Command));
      await refresh();
    })();
    return () => {
      unlisten?.();
      unlistenClipboard?.();
      unlistenMenu?.();
      unlistenView?.();
    };
  });
</script>

<svelte:window onkeydown={onWindowKey} />
<svelte:document oncopy={onCopy} />

{#if NamesPage}
  <NamesPage />
{:else if linkScene}
  <div class="scene" class:mac={onMac()}><AddLink url="https://cdn.example.org/courses/rust/part-2.mp4" fromClipboard={true} onok={() => {}} oncancel={() => {}} /></div>
{:else if progressScene}
  <div class="scene" class:mac={onMac()}><ProgressWindow id={1} /></div>
{:else if addScene}
  <div class="scene" class:mac={onMac()}><AddDialog
    url={addScene.url}
    probe={addScene.probe}
    error={null}
    busy={false}
    queues={snapshot.queues}
    categories={snapshot.categories}
    onadd={() => {}}
    oncancel={() => {}}
    onchoosefolder={async () => null}
    onnewcategory={() => {}}
  /></div>
{:else}
  <div class="window">
    <header>
      <!-- In the header landmark (axe «region», #118). -->
      <TitleBar />
      <h1 class="visually-hidden">{locale.t("app-name")}</h1>
      {#if !onMac()}<MenuBar can={canRun} onrun={run} />{/if}
      <Toolbar bind:search {can} {hint} onaction={doAction} />
    </header>
    <UpdateBar />
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
          oncategories={() => (view = "categories")}
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
        <DownloadTable
          downloads={shown}
          bind:selected
          bind:chooser
          label={locale.t("table-label")}
          onopen={(id) => void openOne(id)}
          oncontext={(x, y) => (rowMenu = { x, y })}
        />
      {/if}
      {#if rowMenu && tableShown && !dialogOpen}
        <ContextMenu
          x={rowMenu.x}
          y={rowMenu.y}
          queues={snapshot.queues.map((q) => ({ name: q.name, label: q.name === "main" ? locale.t("queue-main") : q.name }))}
          can={canRun}
          onrun={run}
          onmove={(q) => void moveTo(q)}
          onclose={(refocus) => {
            rowMenu = null;
            if (refocus) document.querySelector<HTMLElement>('tr[tabindex="0"]')?.focus();
          }}
        />
      {/if}
      {#if deleteAsk}
        <DeleteAsk unfinished={deleteAsk.unfinished} total={deleteAsk.ids.length} onanswer={(withFiles) => void deleteAnswered(withFiles)} />
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
  /* A window scene (the add, link and progress windows, drawn here for
     the screenshots): on macOS the main window's own title bar is hidden
     (#118), and the real windows have the system's above their content. */
  .scene {
    height: 100%;
  }
  .scene.mac {
    box-sizing: border-box;
    padding-top: 28px;
  }
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
