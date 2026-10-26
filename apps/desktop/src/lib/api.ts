// The commands of the Rust side (src-tauri/src/lib.rs). The types are
// generated from Rust (ts-rs), so both sides agree. The calls of the UI
// tests are in ../uitest/api.ts, never in a release build.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { IntegrationView } from "./bindings/IntegrationView";
import type { AddRequest } from "./bindings/AddRequest";
import type { DetailsView } from "./bindings/DetailsView";
import type { PendingLink } from "./bindings/PendingLink";
import type { Sounds } from "./bindings/Sounds";
import type { AddPrefs } from "./bindings/AddPrefs";
import type { ProbeView } from "./bindings/ProbeView";
import type { ToolView } from "./bindings/ToolView";
import type { BatchRequest } from "./bindings/BatchRequest";
import type { BatchPreview } from "./bindings/BatchPreview";
import type { VideoInfoView } from "./bindings/VideoInfoView";
import type { VideoErrorView } from "./bindings/VideoErrorView";
import type { ChoiceView } from "./bindings/ChoiceView";
import type { Snapshot } from "./bindings/Snapshot";
import type { SettingsView } from "./bindings/SettingsView";
import type { SettingsChange } from "./bindings/SettingsChange";
import type { CategoryView } from "./bindings/CategoryView";
import type { ImportView } from "./bindings/ImportView";

export const api = {
  snapshot: () => invoke<Snapshot>("snapshot"),
  probe: (url: string) => invoke<ProbeView>("probe", { url }),
  cancelProbe: () => invoke<void>("cancel_probe"),
  /** The tray menu's texts, and the notification the first time the
   * window hides into the tray. */
  setTrayTexts: (show: string, dropBox: string, quit: string, noticeTitle: string, notice: string) =>
    invoke<void>("set_tray_texts", { show, dropBox, quit, noticeTitle, notice }),
  /** The drop box (M5.5): shown or not, and showing or hiding it. */
  dropBoxShown: () => invoke<boolean>("drop_box_shown"),
  setDropBox: (shown: boolean) => invoke<void>("set_drop_box", { shown }),
  /** Closing the window quits Zajil; and whether there is a tray. */
  closeQuits: () => invoke<[boolean, boolean]>("close_quits"),
  /** Which sounds play (M5.5). */
  soundPrefs: () => invoke<Sounds>("sound_prefs"),
  /** How a new download shows (M5.5): its progress window, and starting
   * while «Download file info» is open. */
  addPrefs: () => invoke<AddPrefs>("add_prefs"),
  setAddPrefs: (prefs: AddPrefs) => invoke<void>("set_add_prefs", { prefs }),
  openStartedProgress: (id: number, title: string) => invoke<void>("open_started_progress", { id, title }),
  setItemDetails: (id: number, category: string | null, description: string | null) =>
    invoke<void>("set_item_details", { id, category, description }),
  setSoundPrefs: (sounds: Sounds) => invoke<void>("set_sound_prefs", { sounds }),
  setCloseQuits: (quits: boolean) => invoke<void>("set_close_quits", { quits }),
  /** The page heard the «quit» a close sent: Zajil waits for its answer. */
  quitHeard: () => invoke<void>("quit_heard"),
  add: (request: AddRequest) => invoke<number>("add", { request }),
  pause: (ids: number[]) => invoke<void>("pause", { ids }),
  resume: (ids: number[]) => invoke<void>("resume", { ids }),
  remove: (ids: number[]) => invoke<void>("remove", { ids }),
  details: (id: number) => invoke<DetailsView>("details", { id }),
  settings: () => invoke<SettingsView>("settings"),
  saveSettings: (change: SettingsChange) => invoke<SettingsView>("save_settings", { change }),
  categories: () => invoke<CategoryView[]>("categories"),
  saveCategories: (categories: CategoryView[]) => invoke<CategoryView[]>("save_categories", { categories }),
  tryCategory: (categories: CategoryView[], name: string) =>
    invoke<string | null>("try_category", { categories, name }),
  exportQueues: (path: string) => invoke<void>("export_queues", { path }),
  importQueues: (path: string, keepFolders: boolean) =>
    invoke<ImportView>("import_queues", { path, keepFolders }),
  onChange: (handler: () => void): Promise<UnlistenFn> => listen("downloads-changed", handler),
  showWindow: () => invoke<void>("show_window"),
  fixBrowserIntegration: () => invoke<IntegrationView>("fix_browser_integration"),
  browserIntegration: () => invoke<IntegrationView | null>("browser_integration"),
  /** "Refresh the download link" (M5); refused with a `RefreshProblem`. */
  refreshLink: (id: number, url: string) => invoke<void>("refresh_link", { id, url }),
  /** Gives up that refresh (its dialog was closed). */
  cancelRefresh: (id: number) => invoke<void>("cancel_refresh", { id }),
  /** A download that waits for its browser goes on without its cookies. */
  goWithoutCookies: (id: number) => invoke<void>("go_without_cookies", { id }),
  /** The macOS menu bar (M5.5): the page's menus in its language. */
  setMenu: (appName: string, menus: unknown[], system: Record<string, string>) =>
    invoke<void>("set_menu", { appName, menus, system }),
  /** A command chosen in the macOS menu bar. */
  onMenuCommand: (handler: (command: string) => void): Promise<UnlistenFn> =>
    listen<string>("menu-command", (event) => handler(event.payload)),
  quitApp: () => invoke<void>("quit_app"),
  appVersion: () => invoke<string>("app_version"),
  /** Opens a finished download (`folder` false) or shows its folder. */
  /** `false`: the file can run code, and opens only once the user is
   * `sure`. */
  openDownload: (id: number, folder: boolean, sure = false) => invoke<boolean>("open_download", { id, folder, sure }),
  /** Takes downloads off the list and deletes their unfinished files. */
  removeWithFiles: (ids: number[]) => invoke<void>("remove_with_files", { ids }),
  moveToQueue: (ids: number[], queue: string) => invoke<void>("move_to_queue", { ids, queue }),
  /** «Copy link»: the downloads' links on the clipboard; how many went. */
  copyLinks: (ids: number[]) => invoke<number>("copy_links", { ids }),
  /** The add windows (M5.5): «Add a link», with a link or none, titled in
   * the page's language. */
  openAddLink: (url: string | null, fromClipboard: boolean, title: string) =>
    invoke<void>("open_add_link", { url, fromClipboard, title }),
  pendingLink: () => invoke<PendingLink | null>("pending_link"),
  /** A link copied while «Add a link» is open. */
  onPendingLink: (handler: (link: PendingLink) => void): Promise<UnlistenFn> =>
    listen<PendingLink>("pending-link", (event) => handler(event.payload)),
  /** «Download file info» for `url`; «Add a link» closes. */
  openAddInfo: (url: string, title: string) => invoke<void>("open_add_info", { url, title }),
  pendingInfo: () => invoke<string | null>("pending_info"),
  /** A download's progress window (M5.5), titled `title` until it names
   * itself. */
  openProgress: (id: number, title: string) => invoke<void>("open_progress", { id, title }),
  /** A download's own speed limit, bytes per second; `null`: none. */
  setLimit: (id: number, limit: number | null) => invoke<void>("set_limit", { id, limit }),
  /** Closes the window that asks. */
  closeWindow: () => invoke<void>("close_window"),
  /** The main window, on `view` («New category»: the categories). */
  showMainView: (view: string) => invoke<void>("show_main_view", { view }),
  onMainView: (handler: (view: string) => void): Promise<UnlistenFn> =>
    listen<string>("main-view", (event) => handler(event.payload)),
  /** Video downloads (M6, #149). */
  openVideo: (url: string | null) => invoke<void>("open_video", { url }),
  /** «Batch download» (M7): opened with a link or none; its links shown
   * before anything is added; then added to a queue, started or not. */
  openBatch: (url: string | null) => invoke<void>("open_batch", { url }),
  batchPending: () => invoke<string | null>("batch_pending"),
  batchPreview: (request: BatchRequest) => invoke<BatchPreview>("batch_preview", { request }),
  batchAdd: (request: BatchRequest, queue: string | null, folder: string | null, start: boolean) =>
    invoke<number>("batch_add", { request, queue, folder, start }),
  /** The clipboard's link, if it holds one. */
  clipboardLink: () => invoke<string | null>("clipboard_link"),
  videoPending: () => invoke<string | null>("video_pending"),
  videoFolder: () => invoke<string>("video_folder"),
  videoToolsStatus: () => invoke<ToolView[]>("video_tools_status"),
  videoToolsInstall: () => invoke<void>("video_tools_install"),
  videoToolsCancel: () => invoke<void>("video_tools_cancel"),
  videoToolsRemove: () => invoke<void>("video_tools_remove"),
  videoProbe: (url: string) => invoke<VideoInfoView>("video_probe", { url }),
  videoProbeCancel: () => invoke<void>("video_probe_cancel"),
  videoDownload: (url: string, choice: ChoiceView, dest: string, name: string) =>
    invoke<number>("video_download", { url, choice, dest, name }),
  videoCancel: (id: number) => invoke<void>("video_cancel", { id }),
  onVideoToolsProgress: (handler: (p: { tool: string; done: number; total: number | null }) => void): Promise<UnlistenFn> =>
    listen<{ tool: string; done: number; total: number | null }>("video-tools-progress", (e) => handler(e.payload)),
  onVideoProgress: (handler: (p: { id: number; done: number; total: number | null; speed: number | null }) => void): Promise<UnlistenFn> =>
    listen<{ id: number; done: number; total: number | null; speed: number | null }>("video-progress", (e) => handler(e.payload)),
  onVideoFinished: (handler: (p: { id: number; file: string }) => void): Promise<UnlistenFn> =>
    listen<{ id: number; file: string }>("video-finished", (e) => handler(e.payload)),
  onVideoFailed: (handler: (p: { id: number; error: VideoErrorView }) => void): Promise<UnlistenFn> =>
    listen<{ id: number; error: VideoErrorView }>("video-failed", (e) => handler(e.payload)),
  /** Clipboard monitoring (M5): a download link the user copied. */
  onClipboardLink: (handler: (url: string) => void): Promise<UnlistenFn> =>
    listen<string>("clipboard-link", (event) => handler(event.payload)),
};
