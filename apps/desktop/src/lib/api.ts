// The commands of the Rust side (src-tauri/src/lib.rs). The types are
// generated from Rust (ts-rs), so both sides agree. The calls of the UI
// tests are in ../uitest/api.ts, never in a release build.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Snapshot } from "./bindings/Snapshot";

export const api = {
  snapshot: () => invoke<Snapshot>("snapshot"),
  pause: (ids: number[]) => invoke<void>("pause", { ids }),
  resume: (ids: number[]) => invoke<void>("resume", { ids }),
  remove: (ids: number[]) => invoke<void>("remove", { ids }),
  onChange: (handler: () => void): Promise<UnlistenFn> => listen("downloads-changed", handler),
};
