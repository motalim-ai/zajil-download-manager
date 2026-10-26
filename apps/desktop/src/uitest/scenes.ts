// ui-test builds only (loaded by App.svelte behind `__UI_TEST__`): the demo
// data of the windows that show none of their own in a scene (M5.5:
// a screenshot of every window). The names mix Arabic, Latin and numbers,
// as the demo list's (src-tauri/src/ui_test.rs).
import type { CompleteView } from "../lib/bindings/CompleteView";
import type { GrabScan } from "../lib/bindings/GrabScan";
import type { QueuePlan } from "../lib/bindings/QueuePlan";

const MIB = 1024 * 1024;

/** «Download complete»: the demo list's finished report. */
export const complete: CompleteView = {
  id: BigInt(3),
  name: "تقرير المبيعات Q3-2026 النهائي.pdf",
  size: 18 * MIB,
  folder: "~/Downloads/Documents",
  url: "https://drive.example.com/reports/q3-2026.pdf",
};

/** The schedule window: the demo list's two queues, the night one timed. */
export const schedule: QueuePlan[] = [
  { name: "main", max_active: 3, start: null, stop: null, days: [], closed_now: false, after: "nothing", max_tries: 1, after_open: null },
  {
    name: "night",
    max_active: 2,
    start: "01:00",
    stop: "07:00",
    days: ["sun", "mon", "tue", "wed", "thu"],
    closed_now: true,
    after: "sleep",
    max_tries: 3,
    after_open: null,
  },
];

/** «Batch download»: a numbered pattern (its links are made by Rust). */
export const batch = "https://lectures.example.edu/rust/part-*.mp4";

/** «Site grabber»: a check already done (no site is read in a scene). */
export const grab: { start: string; scan: GrabScan } = {
  start: "https://files.example.org/books/",
  scan: {
    id: 1,
    files: [
      "https://files.example.org/books/rust-2026.pdf",
      "https://files.example.org/books/algorithms-part-2.pdf",
      "https://files.example.org/books/archive/2025.zip",
      "https://files.example.org/books/audio/lecture-12.mp3",
      "https://files.example.org/books/setup-4.5.exe",
      "https://cdn.example.org/books/maps.tar.gz",
    ],
    pages: 14,
    skipped: 37,
    limits: [],
    robots_closed: false,
    stopped: false,
  },
};
