// The add windows are as tall as what they show (the owner's notes on 1.0:
// at a fixed height the content was cut at the top and a scroll bar came).
import { api } from "./api";

/** A Svelte action on the window's content, drawn at its own height: the
 * window takes that height when it opens and whenever it changes (the
 * server's answer, a line that wraps, an error), and no less. */
export function fitWindow(content: HTMLElement): { destroy: () => void } {
  let asked = 0;
  const observer = new ResizeObserver(() => {
    // From the top of the page: a scene draws a title bar above it.
    const height = Math.ceil(content.getBoundingClientRect().bottom);
    if (height === asked) return;
    asked = height;
    void api.fitWindow(height).catch(() => {});
  });
  observer.observe(content);
  return { destroy: () => observer.disconnect() };
}
