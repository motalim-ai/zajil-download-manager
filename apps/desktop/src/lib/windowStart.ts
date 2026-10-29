// What every window but the main one does first (M5.5): the user's
// language, digits and theme, from the settings. Unreadable settings: the
// defaults (Arabic, as the main window).
import { api } from "./api";
import { locale, type Digits, type Lang } from "./i18n.svelte";
import { setTheme } from "./theme";

/** The macOS menu's command that reaches this window when it is in
 * front: Select all takes the text box's text. Every other command goes to
 * the main window, which comes forward (#98). */
function fromMenu(command: string) {
  const box = document.activeElement;
  if (command === "select-all" && (box instanceof HTMLInputElement || box instanceof HTMLTextAreaElement)) box.select();
}

let started: Promise<void> | null = null;

/** Once per page: a window made beforehand did it while hidden, so the
 * window it becomes shows at once in the user's language and theme. */
export function startWindow(): Promise<void> {
  started ??= (async () => {
    void api.onMenuCommand(fromMenu).catch(() => {});
    locale.set("ar");
    await loadSettings();
  })();
  return started;
}

/** The settings again (a window made beforehand, as it shows: they may
 * have changed while it waited). */
export async function loadSettings(): Promise<void> {
  try {
    const settings = await api.settings();
    locale.set(settings.language as Lang);
    locale.digits = settings.digits as Digits;
    setTheme(settings.theme);
  } catch {
    // The defaults.
  }
}
