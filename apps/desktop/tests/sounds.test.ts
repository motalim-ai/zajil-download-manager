import { describe, expect, it } from "vitest";
import { finishedQueues } from "../src/lib/sounds";
import type { DownloadView } from "../src/lib/bindings/DownloadView";

// When the «queue finished» sound plays (M5.5).
const d = (id: number, queue: string, status: DownloadView["status"]) => ({ id, queue, status }) as DownloadView;
const seen = (...list: DownloadView[]) => new Map(list.map((x) => [x.id, { queue: x.queue, status: x.status }]));

describe("a finished queue", () => {
  it("is one whose running and waiting downloads are all complete now", () => {
    const before = seen(d(1, "main", "active"), d(2, "main", "waiting"), d(3, "night", "active"));
    expect(finishedQueues(before, [d(1, "main", "done"), d(2, "main", "done"), d(3, "night", "active")])).toEqual(["main"]);
  });

  it("is not one paused, or with a download that failed, or one that was idle", () => {
    const before = seen(d(1, "main", "active"), d(2, "main", "active"), d(3, "night", "done"));
    expect(finishedQueues(before, [d(1, "main", "paused"), d(2, "main", "done"), d(3, "night", "done")])).toEqual([]);
    expect(finishedQueues(before, [d(1, "main", "failed"), d(2, "main", "done"), d(3, "night", "done")])).toEqual([]);
  });
});
