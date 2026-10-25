// The schedule window's commands (src-tauri/src/schedule.rs). The types
// are generated from Rust (ts-rs), so both sides agree.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AfterChoice } from "./bindings/AfterChoice";
import type { Countdown } from "./bindings/Countdown";
import type { PlanChange } from "./bindings/PlanChange";
import type { QueuePlan } from "./bindings/QueuePlan";

export const scheduleApi = {
  /** Opens the schedule window, titled in the page's language. */
  open: (title: string) => invoke<void>("open_schedule", { title }),
  plans: () => invoke<QueuePlan[]>("queue_plans"),
  newQueue: (name: string) => invoke<void>("new_queue", { name }),
  /** Its downloads go to the end of the main queue. */
  deleteQueue: (name: string) => invoke<void>("delete_queue", { name }),
  saveQueue: (name: string, change: PlanChange) => invoke<void>("save_queue", { name, change }),
  moveInQueue: (id: number, up: boolean) => invoke<void>("move_in_queue", { id, up }),
  startQueue: (name: string) => invoke<void>("start_queue", { name }),
  /** This run only: never saved. */
  setAfter: (queue: string, choice: AfterChoice) => invoke<void>("set_after", { queue, choice }),
  countdown: () => invoke<Countdown | null>("countdown"),
  cancelAfter: () => invoke<void>("cancel_after"),
  onCountdown: (handler: (countdown: Countdown | null) => void): Promise<UnlistenFn> =>
    listen<Countdown | null>("after-countdown", (event) => handler(event.payload)),
  onAfterFailed: (handler: (error: string) => void): Promise<UnlistenFn> =>
    listen<string>("after-failed", (event) => handler(event.payload)),
};
