// The add windows are as tall as what they show (the owner's notes on 1.0:
// at a fixed height the content was cut at the top and a scroll bar came).
import { tick } from "svelte";
import { api } from "./api";

/** Fits the window to its content now, if this page has a content that
 * fits it ([`fitWindow`]). */
let fitting: (() => Promise<void>) | null = null;

/** The window takes its content's height now, before it shows: it shows
 * at its size and does not grow in sight (the owner's notes). */
export function fitNow(): Promise<void> {
  return fitting?.() ?? Promise.resolve();
}

/** An add window's page has its rows, with their values: the window takes
 * their height, then shows (`windows.rs`), so its first frame is the
 * whole window and nothing grows in sight. */
export async function showFitted(): Promise<void> {
  await tick();
  await fitNow();
  await api.windowReady().catch(() => {});
}

/** A Svelte action on the window's content, drawn at its own height: the
 * window takes that height when it opens and whenever it changes (the
 * server's answer, a line that wraps, an error), and no less; Rust never
 * makes it shorter than it opened. */
export function fitWindow(content: HTMLElement): { destroy: () => void } {
  let asked = 0;
  let last: Promise<void> = Promise.resolve();
  const page = () => document.scrollingElement ?? document.documentElement;
  // Nothing to scroll once the window fits: a focus may have scrolled the
  // page before the window grew (the owner's notes on the 1.0 draft).
  const unscroll = () => {
    page().scrollTop = 0;
  };
  const fit = () => {
    // From the top of the page, not of what is scrolled into view (the 1.0
    // draft: scrolled, it measured itself short); a scene draws a title bar
    // above it.
    const height = Math.ceil(content.getBoundingClientRect().bottom + page().scrollTop);
    if (height === asked) return last;
    asked = height;
    last = api.fitWindow(height, window.innerHeight).then(unscroll, () => {});
    return last;
  };
  const observer = new ResizeObserver(() => void fit());
  observer.observe(content);
  window.addEventListener("resize", unscroll);
  fitting = fit;
  return {
    destroy: () => {
      observer.disconnect();
      window.removeEventListener("resize", unscroll);
      if (fitting === fit) fitting = null;
    },
  };
}
