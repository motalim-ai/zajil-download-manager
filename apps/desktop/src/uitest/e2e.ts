// ui-test builds only (main.ts loads it behind `__UI_TEST__`): M5.5's
// end-to-end test (`src-tauri/tests/end_to_end.rs`). Run with
// `--ui-test-e2e <link>`, each window does what the user would, through
// its own button found by the label the user reads: the main window's
// «Add link», then «OK» with the link, «Start download», and «Open folder»
// in «Download complete». Each says its step to Rust, which writes it down
// for the test (and writes down the folder instead of opening it).
import { invoke } from "@tauri-apps/api/core";
import { locale } from "../lib/i18n.svelte";

const step = (name: string, detail: Record<string, unknown> = {}) => invoke<void>("ui_test_step", { step: name, detail });
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** What `find` gives, waited for 60 s at most; otherwise the test is told
 * what was missing and what the window showed. */
async function until<T>(what: string, find: () => T | null | undefined): Promise<T> {
  for (let i = 0; i < 600; i++) {
    const found = find();
    if (found) return found;
    await sleep(100);
  }
  await step("failed", { what, shown: document.body.innerText.slice(0, 2000) });
  throw new Error(`end-to-end test: no ${what}`);
}

/** The enabled button whose label is `key`'s text, in the window's language. */
const button = (key: string) =>
  [...document.querySelectorAll<HTMLButtonElement>("button")].find((b) => !b.disabled && b.textContent?.trim() === locale.t(key));

/** The name the window shows (drawn by FileName), if any. */
const shownName = () => document.querySelector<HTMLElement>("bdi.fn")?.title ?? null;

export async function drive(win: string | null): Promise<void> {
  const link = await invoke<string | null>("ui_test_e2e").catch(() => null);
  if (!link) return;
  // The window's page runs (Windows: the main one's did not, #220).
  if (win === null) await step("start");
  if (win === null) {
    (await until("«Add link» in the toolbar", () => button("tb-add"))).click();
  } else if (win === "add-link") {
    const box = await until("the link box", () => document.getElementById("addlink-url") as HTMLInputElement | null);
    box.value = link;
    box.dispatchEvent(new Event("input", { bubbles: true }));
    const ok = await until("«OK»", () => button("addlink-ok"));
    await step("link", { url: box.value });
    ok.click();
  } else if (win === "add-info") {
    // Enabled once the link has been checked.
    const start = await until("«Start download»", () => button("add-start"));
    await step("info", { name: (document.getElementById("add-name") as HTMLInputElement | null)?.value ?? null });
    start.click();
  } else if (/^progress-\d+$/.test(win)) {
    // It draws its name only in its title bar; its heading for screen
    // readers has the name once the download's data came.
    const name = await until("the download's name", () => {
      const text = document.getElementById("progress-name")?.textContent?.trim();
      return text && text !== locale.t("progress-title") ? text : null;
    });
    await step("progress", { id: Number(win.slice("progress-".length)), name });
  } else if (win === "complete") {
    const open = await until("«Open folder»", () => button("complete-open-folder"));
    await step("complete", { name: shownName() });
    open.click();
  }
}
