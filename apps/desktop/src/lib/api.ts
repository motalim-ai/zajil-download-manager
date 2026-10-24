// The commands of the Rust side (src-tauri/src/lib.rs). The types are
// generated from Rust (ts-rs), so both sides agree. The calls of the UI
// tests are in ../uitest/api.ts, never in a release build.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AddRequest } from "./bindings/AddRequest";
import type { DetailsView } from "./bindings/DetailsView";
import type { ProbeView } from "./bindings/ProbeView";
import type { Snapshot } from "./bindings/Snapshot";

export const api = {
  snapshot: () => invoke<Snapshot>("snapshot"),
  probe: (url: string) => invoke<ProbeView>("probe", { url }),
  cancelProbe: () => invoke<void>("cancel_probe"),
  add: (request: AddRequest) => invoke<number>("add", { request }),
  pause: (ids: number[]) => invoke<void>("pause", { ids }),
  resume: (ids: number[]) => invoke<void>("resume", { ids }),
  remove: (ids: number[]) => invoke<void>("remove", { ids }),
  details: (id: number) => invoke<DetailsView>("details", { id }),
  onChange: (handler: () => void): Promise<UnlistenFn> => listen("downloads-changed", handler),
};
