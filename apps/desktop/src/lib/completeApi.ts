// The «Download complete» windows' commands (src-tauri/src/complete.rs),
// and the user's choice of showing them, kept in `localStorage` as the
// table's columns are: without it, they show.
import { invoke } from "@tauri-apps/api/core";
import type { CompleteView } from "./bindings/CompleteView";

const KEY = "zajil.complete-window";

export const completeApi = {
  info: (id: number) => invoke<CompleteView>("complete_info", { id }),
  /** Tells Rust whether the window shows when a download finishes. */
  setShow: (on: boolean) => invoke<void>("set_show_complete", { on }),
};

/** Whether the window shows: yes, unless the user said otherwise. */
export function loadShow(storage: Pick<Storage, "getItem"> | undefined = globalThis.localStorage): boolean {
  try {
    return storage?.getItem(KEY) !== "off";
  } catch {
    return true;
  }
}

/** Keeps the choice, and tells Rust. */
export async function saveShow(on: boolean, storage: Pick<Storage, "setItem"> | undefined = globalThis.localStorage): Promise<void> {
  try {
    storage?.setItem(KEY, on ? "on" : "off");
  } catch {
    // Not kept: it still applies until the program closes.
  }
  await completeApi.setShow(on);
}
