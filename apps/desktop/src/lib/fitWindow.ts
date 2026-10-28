// The add windows are as tall as what they show (the owner's notes on 1.0:
// at a fixed height the content was cut at the top and a scroll bar came).
import { api } from "./api";

/** A Svelte action on the window's content, drawn at its own height: the
 * window takes that height when it opens and whenever it changes (the
 * server's answer, a line that wraps, an error), and no less; Rust never
 * makes it shorter than it opened. */
export function fitWindow(content: HTMLElement): { destroy: () => void } {
  let asked = 0;
  const page = () => document.scrollingElement ?? document.documentElement;
  // Nothing to scroll once the window fits: a focus may have scrolled the
  // page before the window grew (the owner's notes on the 1.0 draft).
  const unscroll = () => (page().scrollTop = 0);
  const observer = new ResizeObserver(() => {
    // From the top of the page, not of what is scrolled into view (the 1.0
    // draft: scrolled, it measured itself short); a scene draws a title bar
    // above it.
    const height = Math.ceil(content.getBoundingClientRect().bottom + page().scrollTop);
    if (height === asked) return;
    asked = height;
    void api.fitWindow(height, window.innerHeight).then(unscroll, () => {});
  });
  observer.observe(content);
  window.addEventListener("resize", unscroll);
  return {
    destroy: () => {
      observer.disconnect();
      window.removeEventListener("resize", unscroll);
    },
  };
}
