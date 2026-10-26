// Zajil's sounds (M5.5, an IDM touch): short tones it makes itself with
// Web Audio, no sound files (so no one else's license): two rising notes
// when a download finishes, one low note when one fails, three when a
// queue finishes. Which ones play is the user's choice (the settings).
import type { DownloadView } from "./bindings/DownloadView";

export type Sound = "done" | "failed" | "queue";

const NOTES: Record<Sound, [number, number][]> = {
  // [frequency in Hz, start in s]
  done: [
    [660, 0],
    [880, 0.12],
  ],
  failed: [[220, 0]],
  queue: [
    [523, 0],
    [659, 0.12],
    [784, 0.24],
  ],
};

let context: AudioContext | null = null;

export function play(sound: Sound): void {
  try {
    context ??= new AudioContext();
    const now = context.currentTime;
    for (const [frequency, start] of NOTES[sound]) {
      const tone = context.createOscillator();
      const level = context.createGain();
      tone.type = "sine";
      tone.frequency.value = frequency;
      level.gain.setValueAtTime(0.0001, now + start);
      level.gain.exponentialRampToValueAtTime(0.18, now + start + 0.02);
      level.gain.exponentialRampToValueAtTime(0.0001, now + start + 0.35);
      tone.connect(level).connect(context.destination);
      tone.start(now + start);
      tone.stop(now + start + 0.4);
    }
  } catch {
    // No sound where the page cannot make one.
  }
}

type Seen = Map<number, { queue: string; status: DownloadView["status"] }>;

/** The queues that finished between two looks: every download of the
 * queue that was running or waiting before is complete now (pausing them,
 * or one failing, is not finishing). */
export function finishedQueues(before: Seen, downloads: DownloadView[]): string[] {
  const now = new Map(downloads.map((d) => [d.id, d.status]));
  const busy = new Map<string, number[]>();
  for (const [id, { queue, status }] of before) {
    if (status === "active" || status === "waiting") busy.set(queue, [...(busy.get(queue) ?? []), id]);
  }
  return [...busy]
    .filter(([, ids]) => ids.every((id) => now.get(id) === "done"))
    .map(([queue]) => queue);
}
