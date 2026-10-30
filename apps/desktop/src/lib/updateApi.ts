// The «new version» notice's commands (src-tauri/src/update.rs, #124), and
// the signed update's (ADR-0011).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { UpdateProgressView } from "./bindings/UpdateProgressView";
import type { UpdateView } from "./bindings/UpdateView";

export const updateApi = {
  /** The newer release found at this start, if any. */
  notice: () => invoke<UpdateView | null>("update_notice"),
  /** Opens its release page in the browser, from Rust. */
  openReleasePage: () => invoke<void>("open_release_page"),
  /** «Skip this version»: not shown again; a newer one is. */
  skip: () => invoke<void>("update_skip"),
  /** «Update now»: downloads, checks and installs it, then Zajil opens again. */
  install: () => invoke<void>("update_install"),
  onFound: (handler: (found: UpdateView) => void): Promise<UnlistenFn> =>
    listen<UpdateView>("update-available", (event) => handler(event.payload)),
  onProgress: (handler: (progress: UpdateProgressView) => void): Promise<UnlistenFn> =>
    listen<UpdateProgressView>("update-progress", (event) => handler(event.payload)),
};
