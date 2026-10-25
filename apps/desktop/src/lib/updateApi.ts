// The «new version» notice's commands (src-tauri/src/update.rs, #124).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { UpdateView } from "./bindings/UpdateView";

export const updateApi = {
  /** The newer release found at this start, if any. */
  notice: () => invoke<UpdateView | null>("update_notice"),
  /** Opens its release page in the browser, from Rust. */
  openReleasePage: () => invoke<void>("open_release_page"),
  onFound: (handler: (found: UpdateView) => void): Promise<UnlistenFn> =>
    listen<UpdateView>("update-available", (event) => handler(event.payload)),
};
