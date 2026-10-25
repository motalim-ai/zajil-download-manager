// The commands of the Rust side (src-tauri/src/lib.rs). The types are
// generated from Rust (ts-rs), so both sides agree. The calls of the UI
// tests are in ../uitest/api.ts, never in a release build.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { IntegrationView } from "./bindings/IntegrationView";
import type { AddRequest } from "./bindings/AddRequest";
import type { DetailsView } from "./bindings/DetailsView";
import type { ProbeView } from "./bindings/ProbeView";
import type { Snapshot } from "./bindings/Snapshot";
import type { SettingsView } from "./bindings/SettingsView";
import type { SettingsChange } from "./bindings/SettingsChange";
import type { CategoryView } from "./bindings/CategoryView";
import type { ImportView } from "./bindings/ImportView";

export const api = {
  snapshot: () => invoke<Snapshot>("snapshot"),
  probe: (url: string) => invoke<ProbeView>("probe", { url }),
  cancelProbe: () => invoke<void>("cancel_probe"),
  setTrayTexts: (show: string, quit: string) => invoke<void>("set_tray_texts", { show, quit }),
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
  /** Clipboard monitoring (M5): a download link the user copied. */
  onClipboardLink: (handler: (url: string) => void): Promise<UnlistenFn> =>
    listen<string>("clipboard-link", (event) => handler(event.payload)),
};
